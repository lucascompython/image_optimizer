use rimage::codecs::avif::{AvifEncoder, AvifOptions};
use rimage::operations::resize::{Resize, ResizeAlg};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_image::codecs::jpeg::JpegDecoder;
use zune_image::codecs::png::PngDecoder;
use zune_image::codecs::png::zune_core::options::DecoderOptions;
use zune_image::image::Image;
use zune_image::traits::{EncoderTrait, OperationsTrait};
use zune_imageprocs::composite::{Composite, CompositeMethod};

/// Watermark data for compositing onto images
pub struct Watermark {
    image: Image,
    width: usize,
    height: usize,
}

impl Watermark {
    /// Load a watermark from a PNG file path
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let data = fs::read(path)?;
        Self::from_bytes(&data)
    }

    /// Load a watermark from PNG bytes
    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        let decoder_options = DecoderOptions::new_fast();
        let decoder = PngDecoder::new_with_options(ZCursor::new(data.to_vec()), decoder_options);
        let image = Image::from_decoder(decoder)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let (width, height) = image.dimensions();

        Ok(Self {
            image,
            width,
            height,
        })
    }

    /// Get watermark dimensions
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
}

// Watermark needs to be Send + Sync for Arc sharing across threads
// The underlying Image type should already be Send + Sync
// unsafe impl Send for Watermark {}
// unsafe impl Sync for Watermark {}

/// Options for image processing
#[derive(Clone, Copy)]
pub struct ProcessingOptions {
    /// Target width for resized images (height scales proportionally)
    pub target_width: usize,
    /// AVIF encoding speed (1-10). Lower = smaller files but slower.
    pub avif_speed: u8,
}

impl Default for ProcessingOptions {
    fn default() -> Self {
        Self {
            target_width: 800,
            avif_speed: 1,
        }
    }
}

impl ProcessingOptions {
    pub fn new(target_width: usize, avif_speed: u8) -> Self {
        Self {
            target_width,
            avif_speed: avif_speed.clamp(1, 10),
        }
    }
}

/// Error type for image processing operations
#[derive(Debug)]
pub enum ProcessingError {
    Io(io::Error),
    Decode(String),
    ColorConversion,
    Resize,
    Watermark,
    Encode,
}

impl std::fmt::Display for ProcessingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessingError::Io(e) => write!(f, "I/O error: {}", e),
            ProcessingError::Decode(e) => write!(f, "Decode error: {}", e),
            ProcessingError::ColorConversion => write!(f, "Color conversion failed"),
            ProcessingError::Resize => write!(f, "Resize failed"),
            ProcessingError::Watermark => write!(f, "Watermark application failed"),
            ProcessingError::Encode => write!(f, "AVIF encoding failed"),
        }
    }
}

impl std::error::Error for ProcessingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProcessingError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ProcessingError {
    fn from(e: io::Error) -> Self {
        ProcessingError::Io(e)
    }
}

/// Process a single JPEG image from bytes and return AVIF bytes
pub fn process_image_bytes(
    jpeg_data: &[u8],
    watermark: &Watermark,
    options: ProcessingOptions,
) -> Result<Vec<u8>, ProcessingError> {
    let decoder_options = DecoderOptions::new_fast();
    let decoder = JpegDecoder::new_with_options(ZCursor::new(jpeg_data.to_vec()), decoder_options);

    let mut img =
        Image::from_decoder(decoder).map_err(|e| ProcessingError::Decode(e.to_string()))?;

    if img.convert_color(ColorSpace::RGBA).is_err() {
        return Err(ProcessingError::ColorConversion);
    }

    let (w, h) = img.dimensions();
    let scale = options.target_width as f32 / w as f32;
    let target_height = (h as f32 * scale) as usize;

    let resize = Resize::new(
        options.target_width,
        target_height,
        ResizeAlg::Convolution(rimage::operations::resize::FilterType::Bilinear),
    );

    if resize.execute(&mut img).is_err() {
        return Err(ProcessingError::Resize);
    }

    let x_offset = (options.target_width - watermark.width) / 2;
    let y_offset = (target_height - watermark.height) / 2;
    let composite = Composite::new(
        &watermark.image,
        CompositeMethod::Over,
        (x_offset, y_offset),
    );

    if composite.execute(&mut img).is_err() {
        return Err(ProcessingError::Watermark);
    }

    let avif_encoder_options = AvifOptions {
        speed: options.avif_speed,
        ..Default::default()
    };

    let mut avif_encoder = AvifEncoder::new_with_options(avif_encoder_options);
    let mut result = Vec::with_capacity(64 * 1024);

    if avif_encoder.encode(&img, &mut result).is_err() {
        return Err(ProcessingError::Encode);
    }

    Ok(result)
}

/// Process a single JPEG file and write the result to an output path
pub fn process_image_file(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    watermark: &Watermark,
    options: ProcessingOptions,
) -> Result<(), ProcessingError> {
    let data = fs::read(input_path)?;
    let result = process_image_bytes(&data, watermark, options)?;
    fs::write(output_path, &result)?;
    Ok(())
}

/// Result of batch processing
#[derive(Debug, Default)]
pub struct BatchResult {
    pub successful: usize,
    pub failed: usize,
    pub errors: Vec<(PathBuf, ProcessingError)>,
}

/// Collect all JPEG files from a directory (with subdirectory structure)
/// Returns Vec of (processed_dir, file_path)
pub fn collect_jpeg_files(input_dir: impl AsRef<Path>) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    let files: Vec<(PathBuf, PathBuf)> = fs::read_dir(input_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|dir_entry| {
            let dir_path = dir_entry.path();
            fs::read_dir(&dir_path)
                .into_iter()
                .flatten()
                .filter_map(|sub_entry| sub_entry.ok())
                .filter_map(move |sub_entry| {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file()
                        && sub_path
                            .extension()
                            .is_some_and(|ext| matches!(ext.to_str().unwrap_or(""), "jpeg" | "jpg"))
                    {
                        let processed_dir = dir_path.join("processed");
                        Some((processed_dir, sub_path))
                    } else {
                        None
                    }
                })
        })
        .collect();

    Ok(files)
}

/// Process multiple images in parallel using scoped threads (no memory leaks)
///
/// # Arguments
/// * `files` - Vec of (output_directory, input_file_path)
/// * `watermark` - Arc-wrapped watermark to apply to all images
/// * `options` - Processing options
/// * `num_threads` - Number of worker threads (None for auto-detect based on CPU cores)
pub fn process_batch(
    files: Vec<(PathBuf, PathBuf)>,
    watermark: Watermark,
    options: ProcessingOptions,
    num_threads: Option<usize>,
) -> BatchResult {
    if files.is_empty() {
        return BatchResult::default();
    }

    let num_workers = num_threads.unwrap_or_else(|| {
        thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
    });

    let unique_dirs: HashSet<&PathBuf> = files.iter().map(|(dir, _)| dir).collect();
    for dir in unique_dirs {
        let _ = fs::create_dir_all(dir);
    }

    let work_index = AtomicUsize::new(0);
    let successful = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    // Use scoped threads to avoid 'static lifetime requirements
    // This ensures all threads complete before the function returns
    // and all borrowed data is valid for the duration
    thread::scope(|scope| {
        for _ in 0..num_workers {
            let files = &files;
            let work_index = &work_index;
            let successful = &successful;
            let failed = &failed;
            let watermark = &watermark;

            scope.spawn(move || {
                loop {
                    let idx = work_index.fetch_add(1, Ordering::Relaxed);
                    if idx >= files.len() {
                        break;
                    }

                    let (processed_dir, file_path) = &files[idx];
                    let filename = file_path.file_stem().unwrap_or_default().to_string_lossy();
                    let output_path = processed_dir.join(format!("{}.avif", filename));

                    match process_image_file(file_path, output_path, watermark, options) {
                        Ok(()) => {
                            successful.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });

    BatchResult {
        successful: successful.load(Ordering::Relaxed),
        failed: failed.load(Ordering::Relaxed),
        errors: Vec::new(), // Errors not collected in parallel version for simplicity
    }
}

// watermark can probably be leaked and shared via &'static reference
/// Process a directory of images with the standard directory structure
///
/// Expects input_dir to contain subdirectories with JPEG files.
/// Creates "processed" subdirectories with AVIF output files.
pub fn process_directory(
    input_dir: impl AsRef<Path>,
    watermark: Watermark,
    options: ProcessingOptions,
    num_threads: Option<usize>,
) -> io::Result<BatchResult> {
    let files = collect_jpeg_files(input_dir)?;
    Ok(process_batch(files, watermark, options, num_threads))
}
