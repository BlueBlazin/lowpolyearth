#!/usr/bin/env bash
# Builds the browser app into target/web (a static site root).
#
#   tools/build-web.sh            optimized build
#   tools/build-web.sh --dev      faster build with debug assertions
#
# Requires the pinned nightly toolchain with rust-src (threads and exception
# handling need a rebuilt standard library), wasm-bindgen-cli matching the
# locked wasm-bindgen version and, for optimized builds, binaryen's wasm-opt.
set -euo pipefail

TOOLCHAIN=nightly-2026-09-28
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/target/web"
profile=web
if [[ "${1:-}" == "--dev" ]]; then
    profile=dev
fi
artifact_dir=$([[ $profile == dev ]] && echo debug || echo "$profile")

bindgen_version=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/, ""); print}' "$root/Cargo.lock")
if [[ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $bindgen_version" ]]; then
    echo "error: need wasm-bindgen-cli $bindgen_version:" >&2
    echo "  cargo install wasm-bindgen-cli --version $bindgen_version --locked" >&2
    exit 1
fi
if ! rustup run "$TOOLCHAIN" rustc --version >/dev/null 2>&1; then
    echo "error: need $TOOLCHAIN:" >&2
    echo "  rustup toolchain install $TOOLCHAIN --profile minimal --component rust-src --target wasm32-unknown-unknown" >&2
    exit 1
fi
if [[ $profile != dev ]] && ! command -v wasm-opt >/dev/null; then
    echo "error: need wasm-opt: brew install binaryen" >&2
    exit 1
fi

cd "$root"
cargo "+$TOOLCHAIN" build \
    -p vearth-web \
    --target wasm32-unknown-unknown \
    --profile "$profile" \
    -Zbuild-std=std,panic_unwind

rm -rf "$out"
mkdir -p "$out/pkg"
wasm-bindgen \
    --target web \
    --out-dir "$out/pkg" \
    --out-name vearth_web \
    "target/wasm32-unknown-unknown/$artifact_dir/vearth_web.wasm"
if [[ $profile != dev ]]; then
    # -g keeps the function names that make panic stack traces readable.
    wasm-opt -O3 -g "$out/pkg/vearth_web_bg.wasm" -o "$out/pkg/vearth_web_bg.wasm"
fi
cp web/*.html web/*.js web/_headers web/manifest.webmanifest web/loading-collage.webp "$out/"
cp -R web/icons "$out/"
# Script URLs carry the build's hash (the sources' `__BUILD__`), so a browser
# never pairs a cached script with another build's module, whatever cache
# lifetime the host gives scripts.
build=$(cat "$out/pkg/vearth_web_bg.wasm" "$out/pkg/vearth_web.js" web/*.html web/*.js |
    shasum -a 256 | cut -c1-16)
perl -pi -e "s/__BUILD__/$build/g" "$out"/*.html "$out"/*.js
generated="$root/target/web-generated"
cargo run --release --quiet -p vearth-prep -- \
    shard-place-search --output "$generated/place-search"
cargo run --release --quiet -p vearth-prep -- \
    split-landmarks --output "$generated/landmarks"
python3 tools/publish-web-assets.py "$out" --generated "$generated"
echo "built $out ($(du -sh "$out/pkg/vearth_web_bg.wasm" | cut -f1) wasm)"
