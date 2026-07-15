use image_optimizer::{ProcessingOptions, Watermark, process_directory, process_directory_flat};
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

    /// Output directory for processed AVIF files
    #[clap(short, long)]
    output: Option<String>,

    /// Flat output mode: read JPEGs directly from input folder, output flat to --output with sequential names
    #[clap(long)]
    flat: bool,
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

    if args.flat {
        let output_dir = match &args.output {
            Some(dir) => dir.clone(),
            None => {
                eprintln!("--output is required when using --flat");
                process::exit(1);
            }
        };
        println!("Output mode: flat -> '{}'", output_dir);
        match process_directory_flat(&args.input, &output_dir, watermark, options, args.threads) {
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
    } else {
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
}
