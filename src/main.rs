use rayon::prelude::*;
use rimage::codecs::avif::{AvifEncoder, AvifOptions};
use rimage::operations::resize::{Resize, ResizeAlg};
use std::fs;
use std::path::PathBuf;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_image::codecs::jpeg::JpegDecoder;
use zune_image::codecs::png::PngDecoder;
use zune_image::codecs::png::zune_core::options::DecoderOptions;
use zune_image::image::Image;
use zune_image::traits::{EncoderTrait, OperationsTrait};
use zune_imageprocs::composite::{Composite, CompositeMethod};

use clap::Parser;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
struct Args {
    /// Path to the input directory
    #[clap(short, long)]
    input: String,

    /// Path to the watermark file
    #[clap(short, long)]
    watermark: Option<String>,

    /// AVIF encoding speed (1-10). Lower = smaller files but slower.
    #[clap(short, long, default_value = "1")]
    speed: u8,
}

struct Watermark {
    image: Image,
    width: usize,
    height: usize,
}

fn main() {
    let args = Args::parse();
    const TARGET_WIDTH: f32 = 800.0;

    let decoder_options = DecoderOptions::new_fast();
    let avif_speed = args.speed.clamp(1, 10);

    // Load watermark once at startup
    let watermark_image = args.watermark.as_ref().map(|watermark| {
        let wm_raw = fs::read(watermark).expect("Failed to read watermark file");
        let decoder = PngDecoder::new_with_options(ZCursor::new(wm_raw), decoder_options);
        let wm = Image::from_decoder(decoder).expect("Failed to decode watermark");
        let (wm_w, wm_h) = wm.dimensions();

        Watermark {
            image: wm,
            width: wm_w,
            height: wm_h,
        }
    });

    // Collect all JPEG files
    let files: Vec<(PathBuf, PathBuf)> = fs::read_dir(&args.input)
        .expect("Failed to read input directory")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|dir_entry| {
            let dir_path = dir_entry.path();
            fs::read_dir(&dir_path)
                .unwrap()
                .filter_map(|sub_entry| sub_entry.ok())
                .filter_map(move |sub_entry| {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file()
                        && sub_path
                            .extension()
                            .is_some_and(|ext| matches!(ext.to_str().unwrap_or(""), "jpeg" | "jpg"))
                    {
                        Some((dir_path.clone(), sub_path))
                    } else {
                        None
                    }
                })
        })
        .collect();

    let total_files = files.len();
    println!("Found {} JPEG files to process", total_files);

    // Process images in parallel using rayon
    files.into_par_iter().for_each(|(parent_dir, file_path)| {
        let data = match fs::read(&file_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to read {:?}: {}", file_path, e);
                return;
            }
        };

        let decoder = JpegDecoder::new_with_options(ZCursor::new(data), decoder_options);
        let mut img = match Image::from_decoder(decoder) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("Failed to decode {:?}: {}", file_path, e);
                return;
            }
        };

        if img.convert_color(ColorSpace::RGBA).is_err() {
            eprintln!("Failed to convert color for {:?}", file_path);
            return;
        }

        let (w, h) = img.dimensions();
        let scale = TARGET_WIDTH / w as f32;
        let target_height = (h as f32 * scale) as usize;

        let resize = Resize::new(
            TARGET_WIDTH as usize,
            target_height,
            ResizeAlg::Convolution(rimage::operations::resize::FilterType::Bilinear),
        );

        if resize.execute(&mut img).is_err() {
            eprintln!("Failed to resize {:?}", file_path);
            return;
        }

        if let Some(watermark) = &watermark_image {
            let x_offset = (TARGET_WIDTH as usize - watermark.width) / 2;
            let y_offset = (target_height - watermark.height) / 2;
            let composite = Composite::new(
                &watermark.image,
                CompositeMethod::Over,
                (x_offset, y_offset),
            );
            if composite.execute(&mut img).is_err() {
                eprintln!("Failed to apply watermark to {:?}", file_path);
                return;
            }
        }

        let avif_encoder_options = AvifOptions {
            speed: avif_speed,
            ..Default::default()
        };

        let mut avif_encoder = AvifEncoder::new_with_options(avif_encoder_options);
        let mut result = Vec::with_capacity(64 * 1024);

        if avif_encoder.encode(&img, &mut result).is_err() {
            eprintln!("Failed to encode {:?}", file_path);
            return;
        }

        let processed_dir = parent_dir.join("processed");
        let filename_without_extension = file_path.file_stem().unwrap().to_string_lossy();
        let output_path = processed_dir.join(format!("{}.avif", filename_without_extension));
        if let Err(e) = fs::write(&output_path, &result) {
            eprintln!("Failed to write {:?}: {}", output_path, e);
        }
    });

    println!("All images processed successfully!");
}
