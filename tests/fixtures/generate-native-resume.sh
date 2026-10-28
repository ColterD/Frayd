#!/bin/sh
# Own generated black-frame media: no external or copyrighted input.
# Run with FFmpeg containing libx264; CI consumes the small checked-in fixture.
set -eu
ffmpeg -hide_banner -loglevel error -f lavfi -i color=c=black:s=160x90:r=10:d=120 \
  -c:v libx264 -profile:v baseline -level:v 1.0 -pix_fmt yuv420p -preset slow \
  -crf 40 -g 10 -an -map_metadata -1 -movflags +faststart -y \
  "$(dirname "$0")/native-resume.mp4"
