# AVIF
./rimage avif test_images/horizontal.jpg --resize 800w -x --speed 1 --downscale --filter bilinear # also check --premultiply for when adding the watermark

# mozjpeg
./rimage mozjpeg -s="-opt" test_images/horizontal.jpg -q 50 -x --resize 800w --downscale --filter bilinear

# webp
./rimage webp test_images/horizontal.jpg -q 50 -x --resize 800w --downscale --filter bilinear
