use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use rimage::codecs::avif::{AvifEncoder, AvifOptions};
use rimage::operations::resize::{Resize, ResizeAlg};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_image::codecs::jpeg::JpegDecoder;
use zune_image::codecs::png::PngDecoder;
use zune_image::image::Image;
use zune_image::traits::{EncoderTrait, OperationsTrait};
use zune_imageprocs::composite::{Composite, CompositeMethod};

const PROCESSED_DIR: &str = "Resized";

pub struct Watermark {
    image: Image,
    width: usize,
    height: usize,
}

impl Watermark {
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let data = fs::read(path)?;
        Self::from_bytes(&data)
    }

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
    watermark: Option<&Watermark>,
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

    if let Some(watermark) = watermark {
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
    watermark: Option<&Watermark>,
    options: ProcessingOptions,
) -> Result<(), ProcessingError> {
    let data = fs::read(input_path)?;
    let result = process_image_bytes(&data, watermark, options)?;
    fs::write(output_path, &result)?;
    Ok(())
}

#[derive(Debug, Default)]
pub struct BatchResult {
    pub successful: usize,
    pub failed: usize,
}

struct CallbackWorkItem {
    output_relative_path: PathBuf,
    input_file_path: PathBuf,
}

/// Collect all JPEG files from a directory (with subdirectory structure)
/// Returns Vec of (processed_dir, file_path)
/// Be careful this function deletes all the files that don't have a .jpg or .jpeg extension
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
                    if sub_path.is_file() {
                        if sub_path.extension().is_some_and(|ext| {
                            matches!(
                                ext.to_ascii_lowercase().to_str().unwrap_or(""),
                                "jpeg" | "jpg"
                            )
                        }) {
                            // TODO: in the future, when I don't need to have a "processed" subdirectory, we can just remove this
                            let processed_dir = dir_path.join(PROCESSED_DIR);
                            Some((processed_dir, sub_path))
                        } else {
                            fs::remove_file(&sub_path).unwrap();
                            None
                        }
                    } else {
                        None
                    }
                })
        })
        .collect();

    Ok(files)
}

/// Process multiple images in parallel using scoped threads
///
/// # Arguments
/// * `files` - Vec of (output_directory, input_file_path)
/// * `watermark` - Arc-wrapped watermark to apply to all images
/// * `options` - Processing options
/// * `num_threads` - Number of worker threads (None for auto-detect based on CPU cores)
pub fn process_batch(
    files: Vec<(PathBuf, PathBuf)>,
    watermark: Option<Watermark>,
    options: ProcessingOptions,
    num_threads: Option<usize>,
) -> BatchResult {
    if files.is_empty() {
        return BatchResult::default();
    }

    let unique_dirs: HashSet<&PathBuf> = files.iter().map(|(dir, _)| dir).collect();
    for dir in unique_dirs {
        let _ = fs::create_dir_all(dir);
    }

    let run = || {
        let watermark = watermark.as_ref();
        files
            .par_iter()
            .map(|(processed_dir, file_path)| {
                let filename = file_path.file_stem().unwrap_or_default().to_string_lossy();
                let output_path = processed_dir.join(format!("{}.avif", filename));

                if process_image_file(file_path, output_path, watermark, options).is_ok() {
                    (1usize, 0usize)
                } else {
                    (0usize, 1usize)
                }
            })
            .reduce(|| (0usize, 0usize), |a, b| (a.0 + b.0, a.1 + b.1))
    };

    let pool_threads = num_threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });
    let pool = ThreadPoolBuilder::new()
        .num_threads(pool_threads)
        .build()
        .unwrap_or_else(|_| ThreadPoolBuilder::new().build().unwrap());

    let (successful, failed) = pool.install(run);

    BatchResult { successful, failed }
}

/// Process multiple images in parallel while reading input bytes eagerly into memory.
/// This keeps processing CPU-bound and reduces disk reads during worker execution.
pub fn process_batch_in_memory(
    files: Vec<(PathBuf, PathBuf)>,
    watermark: Option<Watermark>,
    options: ProcessingOptions,
    num_threads: Option<usize>,
) -> BatchResult {
    if files.is_empty() {
        return BatchResult::default();
    }

    let mut in_memory_work: Vec<(PathBuf, PathBuf, Vec<u8>)> = Vec::with_capacity(files.len());
    let mut preload_failed = 0usize;

    for (processed_dir, file_path) in files {
        match fs::read(&file_path) {
            Ok(bytes) => in_memory_work.push((processed_dir, file_path, bytes)),
            Err(_) => preload_failed += 1,
        }
    }

    if in_memory_work.is_empty() {
        return BatchResult {
            successful: 0,
            failed: preload_failed,
        };
    }

    let unique_dirs: std::collections::HashSet<&PathBuf> =
        in_memory_work.iter().map(|(dir, _, _)| dir).collect();
    for dir in unique_dirs {
        let _ = fs::create_dir_all(dir);
    }

    let num_threads = num_threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .unwrap_or_else(|_| ThreadPoolBuilder::new().build().unwrap());

    let watermark_ref = watermark.as_ref();

    let (successful, failed_from_processing) = pool.install(|| {
        in_memory_work
            .into_par_iter()
            .map(|(processed_dir, file_path, jpeg_bytes)| {
                let filename = file_path.file_name().unwrap_or_default();
                // let output_path = processed_dir.join(filename);
                let output_path: PathBuf =
                    processed_dir.join(format!("{}.avif", filename.display()));

                match process_image_bytes(&jpeg_bytes, watermark_ref, options) {
                    Ok(result) => {
                        if fs::write(output_path, result).is_ok() {
                            (1usize, 0usize)
                        } else {
                            (0usize, 1usize)
                        }
                    }
                    Err(_) => (0usize, 1usize),
                }
            })
            .reduce(|| (0usize, 0usize), |a, b| (a.0 + b.0, a.1 + b.1))
    });

    BatchResult {
        successful,
        failed: preload_failed + failed_from_processing,
    }
}

// watermark can probably be leaked and shared via &'static reference
/// Process a directory of images with the standard directory structure
///
/// Expects input_dir to contain subdirectories with JPEG files.
/// Creates "processed" subdirectories with AVIF output files.
pub fn process_directory(
    input_dir: impl AsRef<Path>,
    watermark: Option<Watermark>,
    options: ProcessingOptions,
    num_threads: Option<usize>,
) -> io::Result<BatchResult> {
    let files = collect_jpeg_files(input_dir)?;
    Ok(process_batch_in_memory(
        files,
        watermark,
        options,
        num_threads,
    ))
}

pub fn process_directory_with_callback<F>(
    input_dir: impl AsRef<Path>,
    watermark: Option<Watermark>,
    options: ProcessingOptions,
    num_threads: Option<usize>,
    on_processed: F,
) -> io::Result<BatchResult>
where
    F: Fn(&Path, Vec<u8>) -> io::Result<()> + Send + Sync,
{
    let input_dir = input_dir.as_ref().to_path_buf();
    let files = collect_jpeg_files(&input_dir)?;

    if files.is_empty() {
        return Ok(BatchResult::default());
    }

    let mut work_items = Vec::with_capacity(files.len());
    for (processed_dir, file_path) in files {
        let relative_dir = match processed_dir.strip_prefix(&input_dir) {
            Ok(relative) => relative,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "processed directory is outside input directory",
                ));
            }
        };

        let callback_output_dir = if relative_dir
            .file_name()
            .and_then(|segment| segment.to_str())
            .is_some_and(|segment| segment == PROCESSED_DIR)
        {
            relative_dir.parent().unwrap_or(Path::new(""))
        } else {
            relative_dir
        };

        let filename = file_path.file_stem().unwrap();
        let output_relative_path = callback_output_dir.join(format!("{}.avif", filename.display()));

        // let filename = file_path.file_name().unwrap();
        // let output_relative_path = callback_output_dir.join(filename);

        work_items.push(CallbackWorkItem {
            output_relative_path,
            input_file_path: file_path,
        });
    }

    let run = || {
        let watermark = watermark.as_ref();
        work_items
            .par_iter()
            .map(|item| {
                let jpeg_data = match fs::read(&item.input_file_path) {
                    Ok(bytes) => bytes,
                    Err(_) => return (0usize, 1usize),
                };

                let avif_data = match process_image_bytes(&jpeg_data, watermark, options) {
                    Ok(bytes) => bytes,
                    Err(_) => return (0usize, 1usize),
                };

                if on_processed(item.output_relative_path.as_ref(), avif_data).is_ok() {
                    (1usize, 0usize)
                } else {
                    (0usize, 1usize)
                }
            })
            .reduce(|| (0usize, 0usize), |a, b| (a.0 + b.0, a.1 + b.1))
    };

    let pool_threads = num_threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });
    let pool = ThreadPoolBuilder::new()
        .num_threads(pool_threads)
        .build()
        .unwrap_or_else(|_| ThreadPoolBuilder::new().build().unwrap());

    let (successful, failed) = pool.install(run);

    Ok(BatchResult { successful, failed })
}
