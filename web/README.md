# Browser build

The browser build runs the same crates as the native app in Chrome with
WebGPU. `crates/vearth-web` is the entry point, `crates/vearth-platform`
provides threads, the async runtime, storage and asset loading, and this
directory holds the page, its worker bootstraps and its one function.

## Build and run

```sh
rustup toolchain install nightly-2026-09-28 --profile minimal \
  --component rust-src --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <locked wasm-bindgen version> --locked
brew install binaryen          # wasm-opt, for optimized builds
tools/build-web.sh            # optimized; --dev for a faster debug build
python3 tools/serve-web.py    # http://localhost:8787/
```

`tools/build-web.sh` writes a static site to `target/web`: the page, the
wasm-bindgen package in `pkg/`, and under `assets/` every file bundled
through `vearth_platform::asset_bytes!` (fetched whole on first use) or
`asset_blocks!` (published as 64 KiB blocks and fetched as reads reach
them), listed with its hash in `asset-manifest.json` (see
`tools/publish-web-assets.py`). Browser-only derived data is generated into
`target/web-generated` and published under `assets/generated/`: the place
index split by `vearth-prep shard-place-search`, so a search downloads only
the shards its query needs, and each landmark's levels of detail split by
`vearth-prep split-landmarks`, so a view downloads only the levels it draws.
Location-title faces download when a title needs them and are prefetched
once the first view has loaded. `selftest.html` exercises the platform layer
in the browser.

URL parameters mirror the native start options: `at=lat,lon[,alt[,heading[,pitch]]]`,
`time=live|<UTC date-time>`, `weather=clear|procedural|preset:<id>` and `play`.

## Runtime model

All Rust code runs in Web Workers over one shared WebAssembly memory, so the
page must be cross-origin isolated (`_headers`). The main thread
(`main.js`) creates every worker (`threads.js`), forwards input to the root
worker (`app-worker.js`), which owns the WebGPU device and application state,
and performs page requests such as downloads and clipboard writes. Panics
unwind (WebAssembly exception handling), as natively.

Build-time requirements with no native counterpart:

- `vendor/wgpu` patches four WebGPU-backend closures for unwinding builds;
  see `vendor/wgpu/VEARTH_PATCH.md`.
- WGSL is compiled by Chrome, which enforces rules naga does not: mixed `*`
  and `^` need parentheses, and modules that sample in divergent branches
  declare `diagnostic(off, derivative_uniformity)`.

Storage uses the origin-private file system: preferences and cache state
through `vearth_platform::fs`, generated surfaces through the browser
`indexed_chunk_store`. Downloaded source tiles rely on the HTTP cache.
Settings > Storage offers one action, "Clear all data and exit": the page
stops every worker, then POSTs `/clear-site-data`, whose `Clear-Site-Data`
response removes the site's HTTP cache and storage, and also removes local
storage and origin-private files itself. Elevation tiles, fetched from
another origin, stay in the browser's HTTP cache: `Clear-Site-Data` only
clears the responding origin's entries.

## Deployment

The site needs Cloudflare Pages (or any host that serves `_headers` and the
`functions/clear-site-data.js` response). From this directory, so
`functions/` is included:

```sh
npx wrangler pages deploy ../target/web --project-name <project>
```

The page is an installable web app: `manifest.webmanifest` names it and
`icons/` holds the native app icon at the sizes browsers use, generated with
`sips -s format png -z N N packaging/macos/AppIcon.png --out web/icons/icon-N.png`
for N in 32, 180, 192 and 512 (the artwork keeps the globe inside the
maskable safe zone). There is no service worker: installation does not need
one, and the app works only online.

While the app loads, the page shows `loading-collage.webp`, one image of
gallery captures composed by `tools/build-loading-collage.py` from the
site's renditions.

Hosts may cache scripts for hours (a Cloudflare zone's default Browser Cache
TTL is four hours), while the wasm-bindgen glue must match its module
exactly. The page and worker sources therefore reference each other with a
`?v=__BUILD__` query that the build replaces with the build's hash; only
`index.html` and `selftest.html`, which are revalidated, name fixed URLs.

## Map data

Vector tiles come from the site's copy of one Protomaps daily build (whose
builds are not meant to be read by apps directly and stay published for
only a week). The build is stored in the R2 bucket `lowpolyearth-maps` as
consecutive 1 MiB objects, `protomaps/<YYYYMMDD>/<index>.bin`, served at
https://maps.lowpolyearth.online/protomaps/. Objects this small are cached
by Cloudflare's CDN, which a 140 GB archive is not, so repeat reads come
from the edge and no Worker or request limit is involved.
`PROTOMAPS_BUILD` in `crates/vearth-data/src/vectors.rs` names the build both
the browser and the Mac app read; they read nothing else.

The bucket has the custom domain above and a CORS policy allowing `GET` and
`HEAD` from the site and local development origins:

```sh
npx wrangler r2 bucket cors set lowpolyearth-maps --file tools/maps-bucket-cors.json
```

To move to a newer build, run the "Copy Protomaps build" GitHub workflow
(it runs `tools/copy-protomaps-build.py` with the `R2_ACCESS_KEY_ID` and
`R2_SECRET_ACCESS_KEY` repository secrets, an R2 API token with object read
and write access to the bucket):

```sh
gh workflow run copy-protomaps-build.yml -f build=<YYYYMMDD>
```

It streams the build from build.protomaps.com into the bucket (reruns skip
copied blocks), then compares sampled blocks with the source. Set
`PROTOMAPS_BUILD`, then deploy the site and release the Mac app. The Mac app
has no updater and keeps reading the build it shipped with, so delete a
previous build, with the same credentials, only once no release in use
reads it:

```sh
python3 tools/copy-protomaps-build.py --delete <old YYYYMMDD>
```

Assets larger than 20 MiB are published as numbered parts to stay under the
Pages per-file limit. The site has a few thousand files, within the Pages
file-count limit.
