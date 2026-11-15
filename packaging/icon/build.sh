#!/usr/bin/env bash
# Render vikui.svg into the platform icon files committed next to it:
#   vikui.icns (macOS), vikui.ico (Windows) and vikui.png (Linux, 512px).
#
# Needs macOS (iconutil), rsvg-convert (librsvg) and ImageMagick. Run it after
# editing the SVG and commit the results.
set -euo pipefail

dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
svg="$dir/vikui.svg"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

render() { rsvg-convert --width "$1" --height "$1" "$svg" --output "$2"; }

# macOS
iconset="$work/vikui.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
  render "$size" "$iconset/icon_${size}x${size}.png"
  render "$((size * 2))" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil --convert icns --output "$dir/vikui.icns" "$iconset"

# Windows: one PNG-compressed image per size Explorer and the taskbar ask for.
ico_images=()
for size in 16 24 32 48 64 128 256; do
  render "$size" "$work/ico-$size.png"
  ico_images+=("$work/ico-$size.png")
done
magick "${ico_images[@]}" "$dir/vikui.ico"

# Linux
render 512 "$dir/vikui.png"
