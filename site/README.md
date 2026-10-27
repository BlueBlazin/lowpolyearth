# Low Poly Earth website

The site is static HTML, CSS, and JavaScript with local fonts and media. Serve it
from the repository root:

```sh
python3 -m http.server 4175 --directory site
```

`index.html` is the landing page, `screenshots.html` the gallery (generated from
`capture/screenshots-template.html`), and `contact.html`, `privacy.html` and
`404.html` the text pages. All share `style.css` and `site.js`. The script only
enhances: every photograph is a plain link to its JPG, the film stays on its
poster without JavaScript or under reduced motion, and pages read top to bottom.

Headlines use a Latin subset of Archivo Black and body text Instrument Sans, both
under the SIL OFL (`fonts/`); the `@font-face` rules live in `style.css`. The design uses one accent colour and no decorative imagery;
every picture is an app capture.

The public brand is **Low Poly Earth**. The app is a free direct download for
Apple silicon: `index.html#mac` links to the latest notarized DMG on the public
[releases repository](https://github.com/BlueBlazin/lowpolyearth), so the link
needs no edit per release, but the version shown beside it does.
Controls at `index.html#guide` must match the app's input handling and settings.

## Build and hosting

```sh
python3 tools/build-site.py
python3 -m http.server 4175 --directory target/site
```

For Cloudflare Pages, use `python3 tools/build-site.py` and output `target/site`.
The builder validates local links, anchors, generated policy freshness, screenshot
hashes, and the 25 MiB per-file limit. It includes only referenced public assets
and the gallery manifest. Capture sources and unused legacy media are excluded.

`_headers` configures content and browser-permission policies. The builder
content-addresses every stylesheet, script, font, image and video
(`name.<sha256-prefix>.ext` under `assets/` and `fonts/`), rewrites the HTML and CSS
references, and appends one-year `immutable` caching for those prefixes. HTML
keeps its fixed names and default revalidation, so a deploy takes effect on the
next page load. `og:image` is written as an absolute URL on the Pages origin. `404.html`
provides recovery for missing routes.

The public address is https://lowpolyearth.online/ (Pages project `lowpolyearth`),
set as `ORIGIN` in `tools/build-site.py` for absolute social-preview URLs. The
browser version is a separate Pages project, `lowpolyearth-web`, served at
https://play.lowpolyearth.online/ and linked from `index.html#mac`. The Privacy
Policy URL is `/privacy` and the Support URL `/contact`.
Support/privacy email is contact.urbanphilosophy@gmail.com.

## Privacy

Policy content comes from `docs/PRIVACY.md` and is shared with the app.
Edit `tools/templates/privacy.html` for layout, then regenerate:

```sh
python3 tools/write-privacy-policy.py --site site/privacy.html
python3 tools/write-privacy-policy.py --site --check site/privacy.html
```

The policy and contact pages work without JavaScript. Preserve the shared policy
and support address. Data attribution remains at `index.html#source-credits`.

## Capture and publish

Requires the Mac GPU, scene-data network access, Python with Pillow, and `cwebp`.

```sh
cargo build --release -p vearth-app
python3 site/capture/capture-gallery.py
# Capture or retry selected scenes:
python3 site/capture/capture-gallery.py --only reine-winter
```

`capture/gallery-scenes.json` defines camera poses, dates, weather, and size.
The capture helper uses fresh geometry and rejects incomplete scenes, pending
assets, provider errors, and dropped geometry. Source PNGs, reports, logs, and
provenance are retained under the ignored `capture/photos/current/` directory.

Inspect each exact PNG for missing terrain, seams, obstructions, and framing.
Only then set `visual_review` to `approved` in its `.provenance.json`; reject
unusable views. A retry invalidates its prior review.

```sh
python3 site/capture/publish-gallery.py
```

The publisher requires at least 15 approved captures from the current release
binary. It verifies source hashes, scene specifications, and completion reports
before writing WebP renditions at 640–2048 px (no full-resolution originals),
the manifest, and gallery HTML. Publishing requires captures from the current
release binary; rebuilding the app means re-capturing before the next publish.
Older renderer captures are excluded. Keep source attribution and never retouch
geometry or color to conceal artifacts.

Before delivery, verify desktop and mobile layouts, film playback, photo dialogs,
gallery filters, reduced motion, JavaScript-disabled navigation,
and the release FAQ.

## Fly-by film

The landing-page film follows the per-frame camera path in
`capture/routes/alps-flyby.csv` (orbit to the Matterhorn, 30 fps):

```sh
./target/release/vearth-app --capture-trace site/capture/routes/alps-flyby.csv \
  --capture-dir site/capture/frames/alps-flyby --capture-frames 410 --capture-samples 8 \
  --size 1920x1080 --time 2026-07-20T17:04:00Z --weather procedural \
  --weather-seed 11 --weather-override temperature=18,humidity=.45,snow=.3,vegetation=.9,cloud=.3,cloud_base=3200,cloud_top=4200,convection=.2,precip=0,kind=none,aerosol=.05,visibility=120000
ffmpeg -framerate 30 -i site/capture/frames/alps-flyby/frame-%04d.png -an \
  -c:v libvpx-vp9 -b:v 0 -crf 36 -row-mt 1 -pix_fmt yuv420p site/assets/web/alps-flyby.webm
```

`--capture-samples 8` settles the temporal cloud history at every pose; with one
sample, clouds dither visibly in motion. Inspect the frames before encoding. The poster is the final frame, encoded with
`cwebp -q 85 -sharp_yuv`. `capture-flight.py` remains for the older H.264 film,
which the builder validates only when its manifest exists.

## Hero trailer

The hero plays the trailer at 1.25x, silent, as two-pass VP9 at 720p. Encode the
source trailer twice, first with
`-pass 1 -f null /dev/null`, then with `-pass 2` and the output file:

```sh
ffmpeg -i LowPolyEarth.mp4 -an -vf "setpts=PTS/1.25,scale=1280:720:flags=lanczos" \
  -r 60000/1001 -c:v libvpx-vp9 -b:v 1100k -maxrate 1800k -bufsize 1800k -row-mt 1 \
  -tile-columns 2 -auto-alt-ref 1 -lag-in-frames 25 -cpu-used 1 -pix_fmt yuv420p \
  -pass 2 site/assets/web/trailer-720.webm
```

The Yosemite still is the poster while the trailer loads.
