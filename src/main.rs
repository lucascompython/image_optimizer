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

fn main() {
    let args = Args::parse();

    fs::create_dir_all(&args.output).expect("Failed to create output directory");

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
                println!("File: {}", path.file_name().unwrap().display());
            }
        });

    println!("Hello, world!");
}
