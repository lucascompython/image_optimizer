use image_optimizer::{ProcessingOptions, Watermark, process_directory};
use std::process;

use clap::Parser;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
#[command(name = "image_optimizer")]
#[command(about = "Batch optimize JPEG images to AVIF with watermark")]
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

    /// Number of worker threads (defaults to number of CPU cores)
    #[clap(short = 't', long)]
    threads: Option<usize>,

    /// Target width for resized images
    #[clap(long, default_value = "800")]
    width: usize,
}

fn main() {
    let args = Args::parse();

    let watermark = match &args.watermark {
        Some(path) => match Watermark::from_file(path) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("Failed to load watermark '{}': {}", path, e);
                process::exit(1);
            }
        },
        None => None,
    };

    let options = ProcessingOptions::new(args.width, args.speed);

    println!("Processing images from '{}'", args.input);
    println!(
        "Settings: width={}, speed={}, threads={}",
        options.target_width,
        options.avif_speed,
        args.threads
            .map(|t| t.to_string())
            .unwrap_or_else(|| "auto".to_string())
    );

    match process_directory(&args.input, watermark, options, args.threads) {
        Ok(result) => {
            println!(
                "Processing complete: {} successful, {} failed",
                result.successful, result.failed
            );
        }
        Err(e) => {
            eprintln!("Failed to process directory: {}", e);
            process::exit(1);
        }
    }
}
