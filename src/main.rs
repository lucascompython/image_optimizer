use fast_image_resize::{self as fr};
use image::{ImageReader, imageops};
use rayon::prelude::*;
use std::fs;

use clap::Parser;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
struct Args {
    /// Path to the input directory
    #[clap(short, long)]
    input: String,

    /// Path to the output directory
    #[clap(short, long)]
    output: String,

    /// Path to the watermark file
    #[clap(short, long)]
    watermark: Option<String>,
}

struct Watermark {
    image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    width: u32,
    height: u32,
}

fn main() {
    let args = Args::parse();
    const TARGET_WIDTH: f32 = 800.0;

    fs::create_dir_all(&args.output).expect("Failed to create output directory");

    let watermark_image = args.watermark.as_ref().map(|watermark| {
        let wm = ImageReader::open(watermark)
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();
        let wm_w = wm.width();
        let wm_h = wm.height();

        Watermark {
            image: wm,
            width: wm_w,
            height: wm_h,
        }
    });

    // iterrate over files in the input directory
    fs::read_dir(args.input)
        .unwrap()
        .par_bridge()
        .for_each(|entry| {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            // Check if the entry is a file
            if path.is_file()
                && let Some(ext) = path.extension()
                && matches!(ext.to_str().unwrap_or(""), "jpeg" | "jpg")
            {
                let img = ImageReader::open(&path)
                    .unwrap()
                    .decode()
                    .unwrap()
                    .into_rgba8();

                let (w, h) = (img.width(), img.height());
                let scale = TARGET_WIDTH / w as f32;
                let target_height = (h as f32 * scale) as u32;

                let mut dst_image = fr::images::Image::new(
                    TARGET_WIDTH as u32,
                    target_height,
                    fast_image_resize::PixelType::U8x4, // img.pixel_type().unwrap(),
                );

                let src_img = fr::images::Image::from_vec_u8(
                    w,
                    h,
                    img.into_raw(),
                    fast_image_resize::PixelType::U8x4,
                )
                .unwrap();

                let mut resizer = fr::Resizer::new();

                resizer
                    .resize(
                        &src_img,
                        &mut dst_image,
                        &fr::ResizeOptions {
                            algorithm: fr::ResizeAlg::Convolution(fr::FilterType::Box),
                            cropping: fr::SrcCropping::None,
                            mul_div_alpha: false,
                        },
                    )
                    .unwrap();

                let dst_w = dst_image.width();
                let dst_h = dst_image.height();

                let mut out_img = image::RgbaImage::from_raw(
                    TARGET_WIDTH as u32,
                    target_height,
                    dst_image.buffer().to_vec(),
                )
                .unwrap();

                if let Some(watermark) = &watermark_image {
                    imageops::overlay(
                        &mut out_img,
                        &watermark.image,
                        ((dst_w - watermark.width) / 2) as i64,
                        ((dst_h - watermark.height) / 2) as i64,
                    );
                }

                // OPTIMIZE: Use the `libwebp-sys` crate to convert to webp
                let out_img = image::DynamicImage::from(out_img);

                let encoder = webp::Encoder::from_image(&out_img).unwrap();

                let webp_data = encoder.encode(40.0);

                std::fs::write(
                    format!(
                        "{}/{}.webp",
                        args.output,
                        path.file_name().unwrap().to_string_lossy()
                    ),
                    &*webp_data,
                )
                .unwrap();
            }
        });

    println!("All images processed successfully!");
}
