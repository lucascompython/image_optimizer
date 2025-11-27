use rayon::prelude::*;
use rimage::codecs::avif::{AvifEncoder, AvifOptions};
use rimage::operations::resize::{Resize, ResizeAlg};
use std::fs::{self};
use zune_core::colorspace::ColorSpace;
use zune_image::traits::{EncoderTrait, OperationsTrait};
use zune_image::{
    codecs::{
        jpeg::JpegDecoder,
        png::{PngDecoder, zune_core::options::DecoderOptions},
    },
    image::Image,
};
use zune_imageprocs::composite::CompositeMethod;

use clap::Parser;
use mimalloc::MiMalloc;
use zune_core::bytestream::ZCursor;

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

    let watermark_image = args.watermark.as_ref().map(|watermark| {
        let wm_raw = fs::read(watermark).unwrap();

        let decoder = PngDecoder::new_with_options(ZCursor::new(wm_raw), decoder_options);

        let wm = Image::from_decoder(decoder).unwrap();
        let (wm_w, wm_h) = wm.dimensions();

        Watermark {
            image: wm,
            width: wm_w,
            height: wm_h,
        }
    });

    fs::read_dir(args.input)
        .unwrap()
        .par_bridge()
        .for_each(|entry| {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();
            if path.is_dir() {
                fs::read_dir(&path)
                    .unwrap()
                    .par_bridge()
                    .for_each(|sub_entry| {
                        let sub_path = sub_entry
                            .expect("Failed to read sub-directory entry")
                            .path();

                        if sub_path.is_file()
                            && let Some(ext) = sub_path.extension()
                            && matches!(ext.to_str().unwrap_or(""), "jpeg" | "jpg")
                        {
                            let img_raw = fs::read(&sub_path).unwrap();

                            let decoder = JpegDecoder::new_with_options(
                                ZCursor::new(img_raw),
                                decoder_options,
                            );
                            let mut img = Image::from_decoder(decoder).unwrap();
                            img.convert_color(ColorSpace::RGBA).unwrap();

                            let (w, h) = img.dimensions();
                            let scale = TARGET_WIDTH / w as f32;
                            let target_height = (h as f32 * scale) as usize;

                            let resize = Resize::new(
                                TARGET_WIDTH as usize,
                                target_height,
                                ResizeAlg::Convolution(
                                    rimage::operations::resize::FilterType::Bilinear,
                                ),
                            );

                            resize.execute(&mut img).unwrap();

                            if let Some(watermark) = &watermark_image {
                                let x_offset = (TARGET_WIDTH as usize - watermark.width) / 2;
                                let y_offset = (target_height - watermark.height) / 2;
                                let composite = zune_imageprocs::composite::Composite::new(
                                    &watermark.image,
                                    CompositeMethod::Over,
                                    (x_offset, y_offset),
                                );
                                composite.execute(&mut img).unwrap();
                            }

                            let avif_encoder_options = AvifOptions {
                                speed: 1,
                                ..Default::default()
                            };

                            let mut avif_encoder =
                                AvifEncoder::new_with_options(avif_encoder_options);

                            let mut result = vec![]; // TODO: pre-allocate with capacity

                            avif_encoder.encode(&img, &mut result).unwrap();

                            let processed_dir = path.join("processed");
                            fs::create_dir_all(&processed_dir).unwrap();
                            let filename_without_extension =
                                sub_path.file_stem().unwrap().to_string_lossy().to_string();
                            fs::write(
                                processed_dir.join(format!("{}.avif", filename_without_extension)),
                                &result,
                            )
                            .unwrap();
                        }
                    });
            }
        });

    println!("All images processed successfully!");
}
