#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/vearth-app"
FRAMES="$ROOT/site/capture/frames"
OUT="$ROOT/site/assets/showcase"
FPS="${FPS:-30}"

mkdir -p "$FRAMES" "$OUT"

capture_fly_to() {
  local name="$1"
  local frames="$2"
  local size="$3"
  local start="$4"
  local destination="$5"
  local hold_frames="$6"
  shift 6
  if [[ -n "${ONLY:-}" && "$ONLY" != "$name" ]]; then
    return
  fi
  mkdir -p "$FRAMES/$name"
  local existing=("$FRAMES/$name"/frame-*.png)
  if [[ "${RESUME:-0}" == "1" && "${#existing[@]}" -eq "$frames" ]]; then
    echo "reuse $name: $frames fully settled source frames"
    return
  fi
  rm -f "$FRAMES/$name"/frame-*.png
  "$BIN" \
    --capture-fly-to "$destination" \
    --at "$start" \
    --capture-hold-frames "$hold_frames" \
    --capture-dir "$FRAMES/$name" \
    --capture-frames "$frames" \
    --size "$size" \
    --timeout 90 \
    "$@"
}

capture_path() {
  local name="$1"
  local frames="$2"
  local size="$3"
  local route="$4"
  local hold_frames="$5"
  shift 5
  if [[ -n "${ONLY:-}" && "$ONLY" != "$name" ]]; then
    return
  fi
  mkdir -p "$FRAMES/$name"
  local existing=("$FRAMES/$name"/frame-*.png)
  if [[ "${RESUME:-0}" == "1" && "${#existing[@]}" -eq "$frames" ]]; then
    echo "reuse $name: $frames fully settled source frames"
    return
  fi
  rm -f "$FRAMES/$name"/frame-*.png
  "$BIN" \
    --capture-path "$ROOT/site/capture/routes/$route" \
    --capture-hold-frames "$hold_frames" \
    --capture-dir "$FRAMES/$name" \
    --capture-frames "$frames" \
    --size "$size" \
    --timeout 90 \
    "$@"
}

encode_webm() {
  local name="$1"
  local poster_frame="$2"
  if [[ -n "${ONLY:-}" && "$ONLY" != "$name" ]]; then
    return
  fi
  if [[ "${RESUME:-0}" == "1" && -s "$OUT/$name.webm" && -s "$OUT/$name.jpg" ]]; then
    echo "reuse $name: WebM and poster exist"
    return
  fi
  ffmpeg -hide_banner -loglevel warning -y \
    -framerate "$FPS" -i "$FRAMES/$name/frame-%04d.png" \
    -an -c:v libvpx-vp9 -b:v 0 -crf 30 -row-mt 1 -tile-columns 2 \
    -pix_fmt yuv420p "$OUT/$name.webm"
  ffmpeg -hide_banner -loglevel warning -y \
    -i "$FRAMES/$name/frame-$poster_frame.png" -frames:v 1 \
    -update 1 -q:v 2 "$OUT/$name.jpg"
}

verify_webm() {
  local name="$1"
  local expected_size="$2"
  local expected_frames="$3"
  local minimum_duration="$4"
  if [[ -n "${ONLY:-}" && "$ONLY" != "$name" ]]; then
    return
  fi
  local path="$OUT/$name.webm"
  local size
  local frames
  local duration
  size="$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=s=x:p=0 "$path")"
  frames="$(ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of default=noprint_wrappers=1:nokey=1 "$path")"
  duration="$(ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 "$path")"
  [[ "$size" == "$expected_size" ]] || { echo "$name: expected $expected_size, got $size" >&2; exit 1; }
  [[ "$frames" == "$expected_frames" ]] || { echo "$name: expected $expected_frames frames, got $frames" >&2; exit 1; }
  awk -v duration="$duration" -v minimum="$minimum_duration" 'BEGIN { exit !(duration >= minimum) }' || { echo "$name: duration $duration is below $minimum_duration seconds" >&2; exit 1; }
  echo "$name: ${size}, ${frames} frames, ${duration}s · WebM $(du -h "$path" | awk '{print $1}')"
}

# One exact in-game FlyTo from the Pacific-facing globe into a lesser-known
# Peruvian mountain range. The final second holds after the smooth arrival.
capture_fly_to orbit-to-huayhuash 241 1440x810 \
  "-5.000,-95.000,9000000,25,0" "-10.268,-76.905,9000" 30 \
  --capture-end-heading 20 \
  --capture-end-pitch 55 \
  --time 2026-06-18T15:00:00Z \
  --weather clear \
  --weather-seed 101 \
  --weather-override "temperature=7,humidity=.42,visibility=115000,precip=0,kind=none,cloud_base=5600,cloud_top=7800,cloud=.12,wetness=.12,snow=.22,sea_state=.18,aerosol=.08,wind_east=4,wind_north=1,gust=7,turbulence=.08"
encode_webm orbit-to-huayhuash 0210

capture_fly_to simien-highlands 181 1280x720 \
  "13.020,37.720,42000,42,12" "13.246,38.006,6500" 18 \
  --capture-end-heading 42 \
  --capture-end-pitch 55 \
  --time 2026-01-18T06:30:00Z \
  --weather clear \
  --weather-seed 212 \
  --weather-override "temperature=16,humidity=.28,visibility=100000,precip=0,kind=none,cloud_base=5000,cloud_top=7200,cloud=.08,wetness=.04,snow=0,vegetation=.38,drought=.32,aerosol=.12,wind_east=3,wind_north=2,gust=6,turbulence=.08"
encode_webm simien-highlands 0162

capture_fly_to svaneti 181 1280x720 \
  "43.350,42.420,40000,55,14" "43.040,42.730,7000" 18 \
  --capture-end-heading 55 \
  --capture-end-pitch 55 \
  --time 2026-09-18T06:20:00Z \
  --weather clear \
  --weather-seed 307 \
  --weather-override "temperature=9,humidity=.48,visibility=105000,precip=0,kind=none,cloud_base=4700,cloud_top=6900,cloud=.14,wetness=.16,snow=.38,vegetation=.62,drought=.08,aerosol=.07,wind_east=-2,wind_north=3,gust=7,turbulence=.1"
encode_webm svaneti 0162

capture_fly_to karijini 181 1280x720 \
  "-22.800,118.860,35000,310,13" "-22.468,118.558,1800" 18 \
  --capture-end-heading 310 \
  --capture-end-pitch 55 \
  --time 2026-07-18T02:00:00Z \
  --weather clear \
  --weather-seed 18 \
  --weather-override "temperature=23,humidity=.18,visibility=110000,precip=0,kind=none,cloud_base=5200,cloud_top=7200,cloud=.06,wetness=0,snow=0,vegetation=.16,drought=.78,aerosol=.22,wind_east=4,wind_north=-2,gust=8,turbulence=.12"
encode_webm karijini 0162

capture_fly_to mount-fuji 181 1280x720 \
  "35.720,139.020,42000,225,12" "35.390,138.720,4700" 18 \
  --capture-end-heading 185 \
  --capture-end-pitch 61 \
  --time 2026-02-12T00:00:00Z \
  --weather clear \
  --weather-seed 818 \
  --weather-override "temperature=-4,humidity=.48,visibility=100000,precip=0,kind=none,cloud_base=4800,cloud_top=6800,cloud=.12,wetness=.08,snow=.82,vegetation=.42,drought=.08,aerosol=.1,wind_east=3,wind_north=1,gust=8,turbulence=.08"
encode_webm mount-fuji 0162

# Manhattan is the single after-dark exception: its lighting is the subject.
capture_fly_to manhattan-night 181 1280x720 \
  "40.520,-74.360,36000,35,12" "40.758,-73.9855,1600" 18 \
  --capture-end-heading 35 \
  --capture-end-pitch 62 \
  --time 2026-10-19T01:30:00Z \
  --weather clear \
  --weather-seed 144 \
  --weather-override "temperature=18,humidity=.56,visibility=65000,precip=0,kind=none,cloud_base=3000,cloud_top=4800,cloud=.16,wetness=.06,snow=0,vegetation=.58,drought=.12,aerosol=.13,wind_east=3,wind_north=1,gust=7,turbulence=.06"
encode_webm manhattan-night 0162

# Lateral chase footage breaks the repeated descent rhythm. Both flights use
# the visible Kestrel at a moderate altitude and keep a low, stable horizon.
capture_path kestrel-picos 211 1280x720 kestrel-picos.csv 12 \
  --pitch 78 \
  --play \
  --play-vehicle kestrel \
  --time 2026-06-18T10:30:00Z \
  --weather clear \
  --weather-seed 731 \
  --weather-override "temperature=11,humidity=.48,visibility=100000,precip=0,kind=none,cloud_base=3100,cloud_top=4400,cloud=.14,wetness=.12,snow=.16,vegetation=.56,drought=.08,aerosol=.07,wind_east=4,wind_north=2,gust=8,turbulence=.08"
encode_webm kestrel-picos 0192

capture_path kestrel-tian-shan 211 1280x720 kestrel-tian-shan.csv 12 \
  --pitch 78 \
  --play \
  --play-vehicle kestrel \
  --time 2026-08-18T06:30:00Z \
  --weather clear \
  --weather-seed 489 \
  --weather-override "temperature=5,humidity=.38,visibility=105000,precip=0,kind=none,cloud_base=5200,cloud_top=6800,cloud=.12,wetness=.08,snow=.34,vegetation=.42,drought=.12,aerosol=.07,wind_east=3,wind_north=-2,gust=8,turbulence=.09"
encode_webm kestrel-tian-shan 0192

# Ground-relative capture resolves the loaded terrain height before each
# frame. The Sparrow therefore stays at a low chase-camera height while these
# paths follow real OSM road geometry through the bends.
capture_path hoverbike-sani-pass 241 1280x720 hoverbike-sani-pass.csv 15 \
  --pitch 84 \
  --capture-ground-clearance 4.5 \
  --play \
  --play-vehicle sparrow \
  --time 2026-05-18T08:00:00Z \
  --weather clear \
  --weather-seed 265 \
  --weather-override "temperature=15,humidity=.38,visibility=90000,precip=0,kind=none,cloud_base=3300,cloud_top=4700,cloud=.12,wetness=.08,snow=0,vegetation=.46,drought=.24,aerosol=.10,wind_east=2,wind_north=2,gust=6,turbulence=.06"
encode_webm hoverbike-sani-pass 0222

capture_path hoverbike-dades 241 1280x720 hoverbike-dades.csv 15 \
  --pitch 84 \
  --capture-ground-clearance 4.5 \
  --play \
  --play-vehicle sparrow \
  --time 2026-04-18T09:00:00Z \
  --weather clear \
  --weather-seed 906 \
  --weather-override "temperature=22,humidity=.20,visibility=95000,precip=0,kind=none,cloud_base=4200,cloud_top=5800,cloud=.08,wetness=.02,snow=0,vegetation=.14,drought=.68,aerosol=.17,wind_east=3,wind_north=1,gust=7,turbulence=.07"
encode_webm hoverbike-dades 0222

verify_webm orbit-to-huayhuash 1440x810 241 8.0
verify_webm simien-highlands 1280x720 181 6.0
verify_webm svaneti 1280x720 181 6.0
verify_webm karijini 1280x720 181 6.0
verify_webm mount-fuji 1280x720 181 6.0
verify_webm manhattan-night 1280x720 181 6.0
verify_webm kestrel-picos 1280x720 211 7.0
verify_webm kestrel-tian-shan 1280x720 211 7.0
verify_webm hoverbike-sani-pass 1280x720 241 8.0
verify_webm hoverbike-dades 1280x720 241 8.0
