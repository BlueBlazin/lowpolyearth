#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repo_root=${script_dir:h}
source_image="$repo_root/packaging/macos/AppIcon.png"
icon_output="$repo_root/packaging/macos/AppIcon.icns"
icon_work_dir=$(mktemp -d "${TMPDIR:-/tmp}/vearth-icon.XXXXXX")
trap 'rm -rf "$icon_work_dir"' EXIT
iconset="$icon_work_dir/AppIcon.iconset"
mkdir -p "$iconset"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$source_image" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina_size=$((size * 2))
    sips -z "$retina_size" "$retina_size" "$source_image" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$iconset" -o "$icon_work_dir/AppIcon.icns"
cp "$icon_work_dir/AppIcon.icns" "$icon_output"
