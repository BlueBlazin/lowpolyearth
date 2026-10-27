# Development

Low Poly Earth is a Rust 2024 workspace. The workspace manifest requires Rust
1.95 and defines five crates: `vearth-core`, `vearth-data`, `vearth-gen`,
`vearth-app`, and `vearth-prep`.

## Build and run

### Required local asset

`vearth-data` compiles
`crates/vearth-data/assets/place-search-v1.bin` into the application. The
generated 114 MiB file is intentionally ignored, so a fresh checkout needs a
copy matching the hash in the
[place-search source manifest](../crates/vearth-data/assets/place-search-SOURCES.md)
before the workspace can build normally.

To rebuild instead, download the manifest's GeoNames inputs. On a fresh
checkout only, create an empty file at the expected output path so the
`vearth-prep` dependency graph can compile, then run the manifest's
`pack-place-search` command. The command replaces the placeholder; do not run
the application or tests until the generated file has the recorded hash.

From the repository root:

```sh
cargo build --workspace
cargo run --release -p vearth-app
```

`vearth-app` enables asynchronous provider preparation by default. Build the
blocking-provider variant with:

```sh
cargo run --release -p vearth-app --no-default-features
```

The native renderer uses `wgpu`. A supported graphics adapter is required for
the interactive app and GPU-backed capture tests. World geometry uses 2× MSAA,
followed by the spatial tone-map edge filter; adapters without adapter-specific
sample counts use 4×. Set `VEARTH_WORLD_SAMPLES=4` for smoother edges or `1` for
a single-sample performance control. Dense distant geometry and water triangles
make shading cost grow with the sample count. Keep camera, dimensions, weather
and source state identical when comparing captures.

`VEARTH_LOD_RUGGEDNESS=max-step` restores the earlier mountain-refinement
evidence, the largest single terrain step in a region, for comparisons. With
the debug panel visible, `L` switches between that and the default `broad`
policy in a running session; the panel and replay reports name the active one.

Routine frame, flora, shadow, acquisition and cache reports are disabled by
default. Enable them with `RUST_LOG=info,vearth_app::diagnostics=debug` when
investigating performance. The HUD and replay reports remain available without
verbose logging; percentile reports are computed only when requested. Normal
unpackaged launches log to stderr; packaged launches use the rotating log file.

## Validation

Use the same sequence locally that should be expected for repository changes:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
git diff --check
```

Some data-provider tests bind a loopback HTTP listener, and some ignored render
tests require a native GPU. Run those in an environment that permits the
required resource rather than weakening the tests.

## Runtime files

Set `VEARTH_RUNTIME_ROOT` to isolate all mutable runtime state beneath one
directory. Without the override, an unpackaged launch uses these repository-
relative locations:

| Data | Location |
| --- | --- |
| Source and generated-surface caches | `cache/` |
| Preferences and application support | `output/application-support/` |
| Photos | `captures/photos/` |
| Logs | `output/logs/` |

A packaged macOS application resolves its home and Pictures directory through
Foundation. App Store builds keep caches, preferences, and logs inside the app
container; photo PNGs and recipe JSON files use the system Pictures directory
under `Low Poly Earth`, authorized by the Pictures read/write entitlement.

The bundle identifier, cache and preference locations, Cargo crate names, and
`VEARTH_*` environment variables are stable compatibility identifiers. Photos
and logs use the product name; existing exports remain in their original folders.

## Headless captures and replays

The argument parsers in `crates/vearth-app/src/main.rs` and
`crates/vearth-app/src/staging.rs` are the CLI source of truth. Common
development forms are:

```sh
cargo run --release -p vearth-app -- \
  --screenshot output/view.png \
  --at 46.577,7.990,3800,215,28 \
  --time 2026-01-15T12:00:00Z \
  --weather clear

cargo run --release -p vearth-app -- \
  --replay crates/vearth-app/testdata/replays/orbit_to_street.csv
```

The app also supports staging, cinematic capture, landmark calibration, replay
reports, deterministic weather inputs, and explicit render dimensions. Check
the parser and the focused modules before scripting less common modes.

For crop geometry comparisons, set `VEARTH_CROP_GEOMETRY=canopy` (the default)
or `VEARTH_CROP_GEOMETRY=ribbons` and capture the same pose, date, dimensions,
and weather with `--screenshot-report output/view.json`. Both use the same
flora budgets; ribbons affect near and middle distance annual crops. Reports
record the selected mode and GPU timings. Use `--fresh-chunks` when reviewing
generator changes. Agricultural treatments are regional visual priors from
`agriculture_catalog.rs`, including vineyard and tea choices inside existing
farmland in their growing regions, not a survey of individual farms; woodland communities
live in `vegetation.rs`, and decorative mapped field boundaries in `hedgerow.rs`.

## Generated assets

Source manifests beside the relevant assets document retained external-data
and third-party provenance. Naming varies between `README.md`, `*-SOURCE.md`,
`*-SOURCES.md`, and `SOURCES.md`. The manifests record inputs, licenses,
rebuild instructions, and hashes to the extent known, and explicitly disclose
provenance or redistribution gaps. When a manifest exists, keep it consistent
with the generator and consumer in the same change.

The preparation binary and focused tools cover the bundled terrain, place
search, climate, timezone, administrative-region, sky, map, and landmark
assets. Run a tool's argument parser or read its module usage block
instead of copying command options into additional documentation.

Vehicle sources, Blender builders, licenses and generated mesh manifests live in
`assets/vehicles/refined/`. Its exporter preserves evaluated geometry, corner
normals, material IDs and UVs; manifests hash the inputs, textures and output.
`crates/vearth-app/src/vehicles.rs` loads the same validated surface assets for
gameplay and staging. Rebuild commands are recorded beside the asset sources.

## Website media

Follow [the website capture and publishing workflow](../site/README.md) for
reviewed screenshots and flight footage from the current renderer. Source
frames and full-resolution photos are ignored; published media and provenance
manifests are tracked. Run `python3 tools/build-site.py` to validate local
references and published hashes before hosting.

## macOS package

On Apple silicon with macOS 13 or later, build an unsigned application and DMG with:

```sh
tools/package-macos.sh
tools/verify-macos-package.sh
```

Use `tools/package-macos.sh --app-only` for a faster application-bundle build.
The package includes the project's MIT License, the Statue of Liberty
source's MIT license, font licenses, and generated dependency and asset notices.
Hipparcos is retained for noncommercial use; its terms are recorded in
[Data and attribution](DATA_AND_ATTRIBUTION.md).

### Direct-download distribution

A distributable DMG is signed with a Developer ID Application identity and
notarized. Install that certificate in the login keychain, and store notary
credentials once under a keychain profile name (an app-specific password or
App Store Connect API key; keep both outside the source tree):

```sh
xcrun notarytool store-credentials lowpolyearth-notary --apple-id APPLE_ID --team-id TEAM_ID
```

Then package with both variables set:

```sh
VEARTH_DEVELOPER_ID_IDENTITY='Developer ID Application: Your Name (TEAMID)' \
VEARTH_NOTARY_PROFILE=lowpolyearth-notary tools/package-macos.sh
tools/verify-macos-package.sh
```

The script checks both credentials before building, signs the app with the
hardened runtime and no entitlements, signs the DMG, submits it to notarization,
fails unless the status is `Accepted`, and staples the ticket. Verification
requires the hardened runtime on Developer ID builds and a Gatekeeper-accepted
stapled DMG; it reports builds without them as not for distribution.

### Dependency notices

Packaging checks `packaging/licenses/cargo-notices.json` and includes the rendered
Cargo notices in `THIRD_PARTY_NOTICES.md`. The snapshot covers `vearth-app` with
default features on `aarch64-apple-darwin`, including build dependencies and
patched crates under `vendor/`, and excluding dev dependencies. Changes to the lockfile, workspace manifests, or
notice inputs require regeneration and review:

```sh
cargo install cargo-about --version 0.9.2 --locked --features cli
cargo-about generate --locked --fail \
  --manifest-path crates/vearth-app/Cargo.toml \
  --config packaging/licenses/about.toml --format json \
  --output-file /tmp/vearth-cargo-about.json
python3 tools/cargo-notices.py --report /tmp/vearth-cargo-about.json
python3 tools/cargo-notices.py --check
python3 tools/test-cargo-notices.py
```

The generator preserves source license/NOTICE files and scanner excerpts.
`packaging/licenses/upstream.json` supplies checksum-verified license texts from
exact upstream revisions where published crates omit them. New missing notices
fail generation; the two explicitly reviewed MIT metadata-only releases are
identified in the output. Review new licenses and vendored assets when updating
dependencies, even when the scanner accepts their SPDX declarations. The
unmodified MPL-2.0 `smartstring` component has a source-download notice in the
generated document. These notices do not relicense project-authored code.

### Privacy and submission metadata

Use **Low Poly Earth** as the app name in App Store Connect and public
listings. The bundle identifier remains `com.blueblazin.voxelearth`.

The app menu and Settings open the offline policy embedded from `docs/PRIVACY.md`.
Packaging renders the same policy to `Contents/Resources/PrivacyPolicy.html` and
includes `PrivacyInfo.xcprivacy`. Verification checks both resources and the
`NSHumanReadableCopyright` value. See [the privacy disclosure worksheet](APP_STORE_PRIVACY.md)
for the API audit, App Store answers, and public-policy publishing requirements.

### Mac App Store packaging

Prepare an ad hoc signed, sandboxed application locally, without starting the
app, using an Apple account, or uploading anything:

```sh
tools/package-macos-app-store.sh --prepare
```

The output is `target/package/mac-app-store/Low Poly Earth.app`. This preparation
mode creates no installer and is not an App Store distribution build. Set
`VEARTH_APP_SIGN_IDENTITY` to a local development signing identity to use it
instead of ad hoc signing.

For an App Store distribution package, first install the application and
installer signing certificates, with their private keys, in the login keychain.
Create an explicit App ID matching `com.blueblazin.voxelearth` and download a
Mac App Store Connect distribution provisioning profile for that ID and
application signing certificate. Keep the profile outside the source tree.
Apple documents [profile creation](https://developer.apple.com/help/account/provisioning-profiles/create-an-app-store-provisioning-profile/)
and [installer packaging](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution).

```sh
export VEARTH_APP_STORE_PROFILE=/absolute/path/LowPolyEarth.provisionprofile
export VEARTH_APP_SIGN_IDENTITY='Apple Distribution: Your Name (TEAMID)'
export VEARTH_INSTALLER_SIGN_IDENTITY='3rd Party Mac Developer Installer: Your Name (TEAMID)'
export VEARTH_BUILD_NUMBER=1
tools/package-macos-app-store.sh
```

Use the exact identity names or certificate SHA-1 fingerprints returned by
`security find-identity -v`. A `3rd Party Mac Developer Application` identity
is also accepted for application signing. Increase `VEARTH_BUILD_NUMBER` for
each uploaded build; without the override, packaging uses the commit count.

The script validates the profile's platform, expiration, distribution type,
explicit App ID, team, and signing certificate. It embeds the profile, signs
the app with `AppStore.entitlements`, verifies its signature and entitlements,
and creates the signed `target/package/mac-app-store/LowPolyEarth.pkg` using
Apple's `productbuild`. It neither installs nor runs the app and never uploads
or submits the package. A later upload through Apple's Transporter is a
separate action.

The sandbox grants outbound networking, on-demand location, and Pictures
read/write access. It grants no inbound networking, general filesystem
access, or executable-memory exceptions. Existing unsandboxed caches and
preferences are not automatically migrated into the container.
