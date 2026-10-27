#!/usr/bin/env bash
set -euo pipefail

SITE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$SITE/assets/web"
mkdir -p "$OUT"
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != --images-only ) ]]; then
  echo 'usage: site/capture/prepare-web.sh [--images-only]' >&2
  exit 2
fi

# Derive delivery assets from the published in-engine captures, without
# changing their colour or composition. Keep the original captures intact.
while IFS= read -r name; do
  for width in 640 960 1920; do
    cwebp -quiet -q 78 -resize "$width" 0 "$SITE/assets/gallery/$name.jpg" \
      -o "$OUT/$name-$width.webp"
  done
done < <(python3 - "$SITE/assets/gallery/gallery.json" <<'PY'
import json, sys
for screenshot in json.load(open(sys.argv[1]))['screenshots']:
    print(screenshot['id'])
PY
)

if [[ "${1:-}" == --images-only ]]; then
  exit 0
fi

python3 "$SITE/capture/capture-flight.py" --publish
