# Image optimizer library and cli

Converts JPEG images to 99% smaller AVIF images.  
Can optionally apply watermark to the images.

Meant to batch process thousands of images.

Used to display thousands of fast to download, images on e-commerce and photography websites.

## CLI Usage:

```sh
Batch optimize JPEG images to AVIF with watermark

Usage: image_optimizer [OPTIONS] --input <INPUT>

Options:
  -i, --input <INPUT>          Path to the input directory
  -w, --watermark <WATERMARK>  Path to the watermark file
  -s, --speed <SPEED>          AVIF encoding speed (1-10). Lower = smaller files but slower [default: 1]
  -t, --threads <THREADS>      Number of worker threads (defaults to number of CPU cores)
      --width <WIDTH>          Target width for resized images [default: 800]
  -o, --output <OUTPUT>        Output directory for processed AVIF files
      --flat                   Flat output mode: read JPEGs directly from input folder, output flat to --output with sequential names
  -h, --help                   Print help
```

## TODO:

- [ ] add support for io_uring so we can increase throughput by reading and writing simultaneously and concurrently
