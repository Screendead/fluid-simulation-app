#!/bin/bash
# Film the sim on this machine: the look oracle. The phone's stats line
# stays the cost oracle; film runs a fixed 1/120 s frame, so it can
# never show a pacing problem.
set -euo pipefail
cd "$(dirname "$0")/.."
geom=$(cargo run --release --features film --example film)
ffmpeg -y -v error -f rawvideo -pix_fmt rgba -s "$geom" -r 30 \
    -i target/film/film.raw -pix_fmt yuv420p target/film/film.mp4
rm target/film/film.raw
echo "target/film/film.mp4 ($geom, 30 fps)"
