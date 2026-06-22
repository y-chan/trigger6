#!/bin/sh
set -eu

if [ "$#" -lt 1 ]; then
  echo "usage: $0 INPUT_IMAGE [OUT_DIR]" >&2
  exit 2
fi

input=$1
out_dir=${2:-/tmp/t6-jpeg-compare}
mkdir -p "$out_dir"

base="$out_dir/input.png"
sips95="$out_dir/imageio-q95.jpg"
sips100="$out_dir/imageio-q100.jpg"
turbo95="$out_dir/turbo-q95-420.jpg"
turbo100="$out_dir/turbo-q100-420.jpg"

sips -s format png "$input" --out "$base" >/dev/null
sips -s format jpeg -s formatOptions 95 "$input" --out "$sips95" >/dev/null
sips -s format jpeg -s formatOptions 100 "$input" --out "$sips100" >/dev/null
cargo run --quiet --manifest-path mac/t6proto-rs/Cargo.toml --bin t6-encode-jpeg -- \
  --input "$input" \
  --output "$turbo95" \
  --quality 95 \
  --subsamp 420
cargo run --quiet --manifest-path mac/t6proto-rs/Cargo.toml --bin t6-encode-jpeg -- \
  --input "$input" \
  --output "$turbo100" \
  --quality 100 \
  --subsamp 420

echo "wrote:"
echo "  $base"
echo "  $sips95"
echo "  $sips100"
echo "  $turbo95"
echo "  $turbo100"
echo
echo "inspect:"
python3 tools/t6_jpeg_inspect.py "$sips95" "$sips100" "$turbo95" "$turbo100"
