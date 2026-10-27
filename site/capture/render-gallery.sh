#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/vearth-app"
QUALITY="${QUALITY:-4k}"
MASTER="$ROOT/site/capture/photos/$QUALITY"
OUT="$ROOT/site/assets/gallery"
PUBLISH="${PUBLISH:-1}"

mkdir -p "$MASTER"
if [[ "$PUBLISH" == "1" ]]; then
  mkdir -p "$OUT"
fi

capture() {
  local name="$1"
  local aspect="$2"
  shift 2
  if [[ -n "${ONLY:-}" && "$ONLY" != "$name" ]]; then
    return
  fi
  local png="$MASTER/$name.png"

  "$BIN" \
    --screenshot "$png" \
    --photo-quality "$QUALITY" \
    --photo-aspect "$aspect" \
    --timeout 60 \
    "$@"

  if [[ "$PUBLISH" == "1" ]]; then
    ffmpeg -hide_banner -loglevel warning -y \
      -i "$png" \
      -vf "scale='min(2400,iw)':-2" \
      -frames:v 1 -update 1 -q:v 2 \
      "$OUT/$name.jpg"
  fi
}

capture fuji-winter-morning 16:9 \
  --at 35.4300,138.7200,5200,183,72 \
  --time 2026-02-13T03:00:00Z \
  --weather clear \
  --weather-seed 818 \
  --weather-override "temperature=-6,humidity=.44,visibility=110000,precip=0,kind=none,cloud_base=5000,cloud_top=7200,cloud=0,wetness=.08,snow=.9,vegetation=.42,drought=.05,aerosol=.08,wind_east=2,wind_north=1,gust=7,turbulence=.1"

capture lauterbrunnen-winter-light 3:2 \
  --at 46.6400,7.9500,3900,215,70 \
  --time 2026-12-22T08:05:00Z \
  --weather clear \
  --weather-seed 307 \
  --weather-override "temperature=-8,humidity=.72,visibility=85000,precip=0,kind=none,cloud_base=3800,cloud_top=5200,cloud=0,wetness=.28,snow=.96,vegetation=.2,drought=0,aerosol=.06,wind_east=-2,wind_north=2,gust=8,turbulence=.12"

capture grand-canyon-last-light 3:2 \
  --at 36.0100,-112.0500,3900,315,68 \
  --time 2026-06-22T02:20:00Z \
  --weather clear \
  --weather-seed 18 \
  --weather-override "temperature=29,humidity=.16,visibility=100000,precip=0,kind=none,cloud_base=5400,cloud_top=7200,cloud=0,wetness=0,snow=0,vegetation=.24,drought=.88,aerosol=.24,wind_east=3,wind_north=-2,gust=8,turbulence=.15"

capture sydney-harbour-morning 16:9 \
  --at -33.9100,151.2700,3300,315,66 \
  --time 2026-08-10T23:30:00Z \
  --weather clear \
  --weather-seed 83 \
  --weather-override "temperature=13,humidity=.7,visibility=70000,precip=0,kind=none,cloud_base=2400,cloud_top=4200,cloud=0,wetness=.22,snow=0,sea_state=.28,vegetation=.62,drought=.1,aerosol=.12,wind_east=4,wind_north=-2,gust=9,turbulence=.14"

capture milford-storm-break 4:3 \
  --at -44.7300,168.0450,3500,305,68 \
  --time 2026-01-23T18:20:00Z \
  --weather procedural \
  --weather-seed 909 \
  --weather-override "temperature=13,humidity=.9,visibility=36000,precip=1.5,kind=rain,cloud_base=850,cloud_top=3300,cloud=.72,convection=.5,wetness=.92,snow=.08,sea_state=.42,vegetation=.84,drought=0,aerosol=.08,lightning=.15,wind_east=7,wind_north=-3,gust=16,turbulence=.38"

capture manhattan-blue-hour 16:9 \
  --at 40.7250,-74.0100,3100,28,64 \
  --time 2026-10-18T22:10:00Z \
  --weather clear \
  --weather-seed 144 \
  --weather-override "temperature=17,humidity=.58,visibility=65000,precip=0,kind=none,cloud_base=3200,cloud_top=4800,cloud=.02,wetness=.08,snow=0,vegetation=.58,drought=.12,aerosol=.14,wind_east=3,wind_north=1,gust=7,turbulence=.12"
