# Performance and render contracts

Performance results are local measurements, not durable project facts. Keep raw
outputs under the ignored `output/perf/` directory, record the revision and
working-tree state, and compare runs made with the same inputs, build profile,
hardware, driver, and runtime settings.

Checked-in measurement inputs live with the application crate:

- `crates/vearth-app/testdata/replays/` contains deterministic camera traces.
- `crates/vearth-app/testdata/contracts/fixture-manifest.json` defines the
  synthetic source-fixture set and its dataset identity.
- `crates/vearth-app/testdata/contracts/capture-manifest.json` defines render
  capture cases and comparison tolerances.

Generated reports and captures are intentionally not source-controlled.

## Surface pipeline check

`surface-baseline` exercises deterministic terrain, dense buildings with holes
and overlapping envelopes, river crossings, forests and indexed landmarks at
multiple terrain LODs. Its executable validates the fixture manifest, then
checks generated surfaces, GPU preparation and `VXS4` cache round trips.
It also fingerprints every reusable tree profile at all three mesh LODs,
including the bounds used by placement and rendering.

```sh
cargo run --release -p vearth-app --bin surface-baseline -- \
  --samples 9 \
  --output output/perf/surface-pipeline.json \
  --summary output/perf/surface-pipeline.md
```

The JSON records generation and packing samples, vertex/index counts, encoded
sizes, collision primitive counts, generated and prepared semantic fingerprints,
a visual fingerprint, generation version and codec version. Fingerprints are
computed outside timed regions and only when
explicitly requested by diagnostic tools; ordinary cell preparation performs
structural validation without hashing the complete world geometry.

Compare fixture IDs and fingerprints before interpreting timing changes.
The generated identity includes ordered mesh bytes, materials, collision
primitives, water metadata, road guides, source completeness and vegetation.
The visual identity excludes the solid collider and ground heightfield, allowing
intentional collision changes to be checked independently of rendered output.
Prepared identity includes collision and therefore changes with it.
Use the same source inputs, compiler, build profile and runtime settings.
Reports are local diagnostics, not committed performance guarantees.

For local architecture comparisons, pass `--architecture-probes PATH` with a JSON
array containing `style_key`, `latitude_deg` and `longitude_deg`. Use
`--architecture-levels 16,17,18,19` to sample both dense coarse cells and nearby
facades. Preserve the same probe file for both binaries, including locations
missing from the earlier catalog. The probes use synthetic mapped buildings;
they measure generation, packing and cache size, not live provider loading or FPS.
Add `--highrise-probes` to exercise the same locations with dense 48–304 m
commercial and residential envelopes. Keep this flag identical in both runs;
it changes only the external architecture probes, not the default fixtures.

## Render capture contract

Validate the checked-in capture schema, camera pitches, and required tag
coverage without launching the app:

```sh
cargo run -p vearth-app --bin render-contract -- validate-manifest
```

Build the application, then capture the offline staging cases:

```sh
cargo build --release -p vearth-app
cargo run --release -p vearth-app --bin render-contract -- capture \
  --app target/release/vearth-app \
  --output-dir output/perf/render-captures \
  --offline-only
```

The repository does not keep baseline PNGs. To compare against a separately
preserved baseline directory, add `--baseline-dir PATH`. Comparisons are exact
RGBA8 by default and are appropriate only on the same adapter and driver. Add
`--portable` to use the bounded floating-point tolerances read from the
manifest. A successful same-size comparison that exceeds tolerance writes an
amplified difference image; capture, file, decode, and dimension failures do
not.

Two individual images can also be compared directly:

```sh
cargo run --release -p vearth-app --bin render-contract -- compare \
  BASELINE.png CANDIDATE.png output/perf/difference.png
```

## Streaming replay reports

Passing both `--replay` and `--replay-report` runs a deterministic camera trace,
waits for full idle or the configured settle timeout, and writes a
schema-versioned report whose terminal reason distinguishes those outcomes.
Pin the surface size and pacing-related options when comparing runs.

For the globe-to-city search lifecycle, use the native search benchmark:

```sh
cargo run --release -p vearth-app --bin vearth-app -- \
  --benchmark-search Rotterdam --benchmark-search-frame 120 \
  --replay-report output/perf/rotterdam-search.json \
  --replay-max-settle-seconds 90 --replay-idle-frames 30 \
  --replay-size 1600x900
```

This resolves the bundled geocoder result and dispatches the same `UiAction::FlyTo`
as search, with normal wall-clock navigation, destination prefetch, camera floor
and asynchronous selection. It holds the camera after arrival, bypasses saved
preferences and reports the actual selected place and camera poses. It does not
measure typing, search debounce or keyboard latency. Settlement requires an
adopted post-arrival selection and a current final selector response. Do not
combine it with pose replay; absolute pose replay replaces camera updates and
therefore cannot validate the same navigation lifecycle.

```sh
cargo run --release -p vearth-app -- \
  --replay crates/vearth-app/testdata/replays/orbit_to_street.csv \
  --replay-report output/perf/orbit-to-street.json \
  --replay-max-settle-seconds 60 \
  --replay-idle-frames 120 \
  --replay-size 1920x1080
```

`--fresh-chunks` applies to screenshot mode only. For interactive replays,
use separate `VEARTH_RUNTIME_ROOT` directories cloned from the same source-cache
seed, with empty generated-surface caches. Keep preferences, source pin, camera
trace and viewport identical. Distinguish source-cold network runs from
source-warm/generated-cold runs; network availability can dominate the former.
Do not run builds or other CPU-heavy checks concurrently with measured replays.

The default replay computes LOD synchronously for deterministic frame-indexed
selection. Set `VEARTH_ASYNC_REPLAY_LOD=1` to measure the interactive selector
path, and record that setting with the results. Always read the reported surface
dimensions: the window manager may clamp a requested viewport to the desktop.
GPU pass timestamps on Metal span vertex start through fragment completion and
can overlap other passes. They are latency intervals, not additive exclusive
costs; use the total interval and controlled comparisons when locating GPU work.
`gpu_world_ms` includes sky preparation and the combined world/procedural-flora
pass. Independent `gpu_flora_ms` samples are unavailable and remain null;
their distributions have zero samples. Replay reports identify this scope in
`gpu_timing_scope`. Compare total GPU time across reports with different pass
grouping rather than treating the old separate world interval as equivalent.

To isolate movement after a stationary warmup, add
`--replay-measure-frames START:END`. This inclusive, validated range must be
inside the trace, contain at most 4096 frames, and accompany `--replay-report`.
The existing `motion` and `final` summaries retain their original scope;
`measurement` contains only the requested frames. Preserve the same trace,
cache seed and range across paired runs, and inspect readiness at both window
boundaries rather than assuming the warmup finished loading.

Window rows contain the camera pose, CPU stages, submitted geometry, uploads,
before/after wanted coverage and work queues, and cumulative job/cache/network
counters. `resident_no_cell_work` requires all desired cells opaque at both
frame boundaries, no transitions or queued/running/ready cells, no mesh uploads,
and no observed job/cache/network counter changes within the frame. Its
complement is `streaming_or_transition`; these are observed cell-work states,
not a guarantee that every background task or GPU transfer has finished.
`opaque_world_indices` and `feedback_world_indices` partition submitted world
indices by fragment pipeline. Trees and active fades retain the coverage path;
these counts describe submission, not visible pixels. `submitted_world_vertices`
counts whole admitted vertex allocations before index-range rejection and must
not be interpreted as vertex-shader invocations.

Only recorded frames contribute to the corresponding distributions. Window
totals cover frames, missed-vsync events and mesh uploads; other pipeline totals
remain in the cumulative reports.

Each timed render records its issued GPU ID. A later completed ID fills that
render's row exactly once, including results received during the post-window
settle. All valid slots collected together are delivered, including IDs older
than the HUD's latest result. Batches are bounded by the readback ring and never
wait for unfinished slots. GPU samples are never assigned to the CPU frame that
receives them.
Check `gpu_samples` and each distribution's count: unavailable, skipped or
unreceived timings remain null. `window_complete` requires a successful render
for every requested trace frame, independently of GPU timing availability.
Generation and roof snapshots are taken only before/after the window; their
counter differences describe publications during that interval. Their durations
are accumulated worker elapsed time, including descheduling, and jobs can span
either boundary. They do not measure exclusive CPU time or per-frame work.

Replay reports enable bounded completeness diagnostics without per-cell logs.
`generation.stages.incomplete_by_stage` and
`source_completeness.acquisition_by_stage` contain 32 cell-level buckets;
causes overlap, so their totals must not be added. Acquisition counters include
finished plans, excluding canceled ones. `building_failures_by_reason` counts
failure events, including multiple rejected buildings or domains in one cell;
it is not a count of incomplete cells. `required_query_caps_by_zoom` counts
planner evaluations by source zoom, including repeated preparation/consumption.
Compare these with cache writes to distinguish unavailable inputs from persistent
geometry or query limits. `desired_missing` counts unresolved wanted cells;
coverage bootstrap work is separate, and `wanted_presented_delta` measures the
remaining difference between requested and presented detail.

`convergence_diagnostics` records the longest existing idle streaks and bounded
reset histories. Near-idle snapshots retain representative target and extra
draw IDs, signed fade bits and transition phases at most twice per second.
These observations do not alter readiness criteria. A selected-topology change
is an observation, not an additional cause for resetting an idle streak.

Add `--benchmark-drone-flight` to a native search benchmark to continue through
the normal Play entry and player controls after foreground readiness. The
protocol waits for the chase view, cruises, boosts, turns and returns, then
waits for foreground readiness again. It never overrides player or camera
positions. `drone_flight` reports phase-specific loading gaps, frame intervals,
velocity and prefetch activity. Use `--wind-influence off` for a controlled
baseline. The 240-second wall limit and per-wait settle limit can terminate an
unfinished protocol; check its completion and terminal reason before comparing
phases. These observations describe streaming coverage, not pixel visibility.

Use `--benchmark-city-tour` instead to add six seconds of normal Explore
keyboard zoom after the search view fully loads, then wait at the closer view.
Every stationary stop requires a fully idle streak, opaque foreground coverage,
a current selector result and successful presentation. The drone continuation
turns through 180 degrees under boost without braking first. After the final
flight stop fully loads, it exits Play, zooms out and levels the camera for
20 seconds, pans east around the globe for five seconds, and waits again.
The complete tour is bounded at 480 seconds. Protocol v2 reports five-second
checkpoints and phase-end summaries even if a run is interrupted. Camera and player poses remain
controlled by production navigation. Read phase timings and sampled camera
poses; movement durations use the production delta clamped to 0.1 seconds.
`surface_presented_frames` must match `frames` for a fully visible measurement;
`all_observed_frames_presented` flags any failed presentations. Occluded
windows retry around every 100 ms, which is not a foreground frame-rate result.
Full-idle counts and first observations are separate from foreground readiness;
neither first observation certifies a sustained idle interval.
Phase `ended_unix_ms` records the wall clock beside `ended_after_ms` for
alignment with a native profiler's trace start time.
Samples include resident GPU bytes, uploaded bytes, submitted world triangles,
opaque/feedback index counts, shadow caster indices and wanted/drawn levels.
GPU timings arrive asynchronously; use
`gpu_sample_id` to avoid counting a repeated result as a new GPU frame.
Current draw counts and the latest GPU result may refer to different frames;
use a measured replay window for exact issuance-frame attribution. Native
benchmarks pace against absolute 60 Hz deadlines without accumulating timer
overshoot. Actual elapsed time remains the movement and measurement clock.
Set `VEARTH_BENCHMARK_DEBUG_PANEL=1` to include the normal debug panel in a
native benchmark. Benchmark launches otherwise use default panel visibility
and do not load or save interactive preferences.

Set `VEARTH_RENDER_MATERIAL_STATS=1` for an opt-in histogram of submitted world
indices by material. `final.world_material_indices` reports the latest sample
and the coherent frame with the most submitted indices. This counts actual
draw ranges after culling, excluding separate instance passes; it does not
count every cached mesh. The diagnostic scans meshes at upload, so collect it
separately from performance comparisons.

`generation.stages.phases` records per-level execution counts and nanoseconds
for each generation phase. Durations accumulate across workers; they are not
wall-clock load times. `generation.roofs.by_shape_and_level` contains observed
roof tessellation work, including attempted subdivision and fallback paths.
Roof timings exclude field/domain preparation and mesh emission; the buildings
phase records their combined cost. Retry counters include discarded building
visual-mesh work, excluding collision storage. Roof elapsed buckets use the listed
exclusive upper bounds followed by an overflow bucket; `slowest` retains at
most 32 bounded source descriptors, without per-cell logs. These diagnostics
share the replay timing switch and remain disabled during ordinary play.

`timing.visible_converged_after_trace_ms` retains its whole-wanted-cut meaning,
including guard and background lanes. The separate
`foreground_opaque_converged_after_trace_ms` waits for every visible-lane target
(or legacy target in position-only mode) to be resident and fully opaque for
the configured idle-frame interval. It does not end the replay or certify full
idle. `view_lanes` reports terminal desired, missing, presented and opaque-target
counts by lane, plus bounded rolling distributions of missing and presented
targets. Presented counts describe the cell draw list, not a GPU pixel-visibility
test; ancestor backdrops do not count as their requested descendants.
`selector.wanted_by_lane_level` records the terminal cut. Its metric provenance
and `global_to_known_lane_at_terminal` compare the same target IDs using the
global 0–9 km visibility envelope and available padded cell/ancestor heights;
this is a report-time diagnostic only. `selector.visibility_policy` names the
global envelope used by production selection. Sampled-height reclassification
does not affect the wanted cut, generation priorities or convergence timers.
Its inherited heights are estimates, not a certified bound on unseen finer
terrain or structures. Screen-error distance still uses sampled heights with
the existing uncertainty allowance.

Large upload batches may contain spatial index ranges sharing one vertex buffer
and one parent admission slot. Shadow subsets share those vertices and retain
the full world indices; they are selected only when their recorded backing
distance fits within a quarter of the actual shadow-map texel. Caster counts
refer to parent batches, while index counts include the selected ranges. Include
both index streams and their alignment in resident GPU byte accounting.

Sun-shadow caster restriction is enabled by default. Each cached cascade
retains a conservative receiver footprint, including filtering margins and
upstream casters. Leaving that footprint triggers a full-map refresh; narrowing
resumes only after a fixed receiver probe covers eight frames and the normal
update cadence permits it. Use `VEARTH_SHADOW_RECEIVER_CULL=0` for a full-cascade
comparison and record the override with the run.

On macOS, record sampled function stacks for the actual search lifecycle:

```sh
xcrun xctrace record --template 'Time Profiler' --time-limit 120s \
  --output output/perf/city-load.trace \
  --launch -- "$PWD/target/release/vearth-app" \
  --benchmark-search Rotterdam --benchmark-search-frame 120 \
  --replay-report output/perf/city-load.json \
  --replay-max-settle-seconds 90 --replay-idle-frames 30 \
  --replay-size 1600x900
xcrun xctrace export --input output/perf/city-load.trace --toc
```

Use a separate runtime seeded like the unprofiled control. Preserve the exact
binary with the trace. Release function symbols support call-path attribution;
build with `CARGO_PROFILE_RELEASE_DEBUG=1` when source lines and inlined frames
are needed. Export the `time-profile` table to aggregate sampled leaf costs and
inclusive stacks by thread. Inclusive costs overlap, and accumulated weights
across workers are not elapsed load times. Separate startup, navigation and
stationary work; retain any uncertainty in phase alignment. Profiled runs are
diagnostic runs, not uninstrumented performance controls.

For blocking-thread context, also sample the launched engine's PID:

```sh
sample PID 15 5 -file output/perf/city-load.sample.txt
```

Inspect worker call stacks and lock owners alongside the replay's acquisition,
CPU, ready-upload and residency fields. An empty upload queue does not imply
GPU trouble when generation is still building cells.

## Focused microbenchmarks

`architecture_selection_bench` measures warm source-to-design resolution,
including administrative, settlement and named-place lookup. It varies source
seeds and nearby anchors across low buildings, towers and industrial envelopes:

```sh
cargo run --release -p vearth-gen --example architecture_selection_bench -- 20000
```

CSV rows contain location, observed use, height and median nanoseconds per
resolution across five batches. Atlas initialization is outside the timed
region. Compare identical profiles without concurrent builds; use separate
target directories for different source checkouts and retain executable hashes.
This measures generation CPU cost, not frame rate or geometric complexity.

`chunk-cache-bench` measures one generated-surface store mode per process using
an existing current-version legacy-file cache directory. It copies sampled
entries to a temporary directory before measuring migration or scanning,
reopen, read, and write paths. Run both modes to compare them:

```sh
VEARTH_CHUNK_STORE=files cargo run --release -p vearth-app \
  --bin chunk-cache-bench -- CACHE_VERSION_DIRECTORY 10000
VEARTH_CHUNK_STORE=sqlite cargo run --release -p vearth-app \
  --bin chunk-cache-bench -- CACHE_VERSION_DIRECTORY 10000
```

`native-codec-bench` measures legacy gzip cache payloads, Terrarium PNG, and Blue
Marble JPEG decoding against local cache databases and an image supplied by the
caller. Its generated-cache input must contain gzip entries; `surface-baseline`
uses the current surface codec for round-trip validation and encoded sizes:

```sh
cargo run --release -p vearth-data --bin native-codec-bench -- \
  LABEL CHUNK_DB SOURCE_DB BLUE_MARBLE_JPEG 15
```

`source_sampling` measures warm Terrarium sampling with deterministic
PNG fixtures at one, four and eight worker threads. The arguments select samples
per worker and repetitions. Enable all features to use the application's async
preparation and cache-only sampling path; omit them to measure legacy providers:

```sh
cargo run --release -p vearth-data --all-features --example source_sampling -- \
  200000 7 > output/perf/source-sampling.json
```

Compare checksums before timings, and run controls without concurrent builds or
benchmarks. Fixture preparation is outside the timed region; this measures
sampling contention rather than network latency, cold loading or frame rate.
Terrain sampling may retain up to 2 MiB (two decoded 512 px elevation tiles)
of raster payload per sampling thread. These local leases never retain keyed
cache slots, and an evicted key must stop answering even when another key
shares its raster. Source resolution, interpolation and dependency
completeness must remain unchanged.

These tools report observations only. Preserve their JSON alongside environment
metadata when a result is used to justify a change.

The source-tile and generated-surface SQLite stores separate their composite
key indexes from large payload records using ordinary rowid tables. Preserve
this separation:
`WITHOUT ROWID` stores payloads in the key B-tree and can read overflow records
even for cache misses. Keep metadata columns before the payload so accounting
and eviction do not traverse the BLOB's overflow pages. Schema upgrades preserve
payloads, pin flags and access times transactionally; they do not change
generation validity or source caches. Source-tile upgrades copy bounded batches
in the background; both tables remain readable until the atomic final cutover.
Closing the app must leave a restartable migration without waiting for the full
cache to convert.

## Validation after performance-sensitive changes

Run correctness checks before interpreting timing or image results:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
git diff --check
```

Then rerun only the contracts and workloads affected by the change. Keep the
dataset, fixture manifest, sample count, budgets, build profile, adapter and
driver identical. If generation, codec or render schema versions differ,
establish output compatibility before comparing timings; an intentional
collision change requires matching visual identity and separate contact tests.

## Technical references

- [Cesium Native's tile-selection algorithm](https://cesium.com/learn/cesium-native/ref-doc/selection-algorithm-details.html)
  separates geometric error from content bounds and preserves resident coverage
  without requiring every intermediate level to load.
- [Epic's automatic mesh LOD guidance](https://dev.epicgames.com/documentation/en-us/unreal-engine/static-mesh-automatic-lod-generation-in-unreal-engine)
  relates mesh simplification to projected error and silhouette importance.
- [Epic's Asset Loading Insights](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-insights-in-unreal-engine)
  separates asset loading, request priority and frame work when locating stalls.
- [SQLite threading rules](https://www.sqlite.org/threadsafe.html) permit
  independent reader connections while requiring exclusive ownership of each
  connection and its prepared statements.
- [Apple's Metal frame analysis](https://developer.apple.com/documentation/xcode/analyzing-the-performance-of-your-metal-app)
  covers CPU starvation, frame pacing, and CPU/GPU overlap. Distinguish render
  preparation, presentation waiting, and background loading in CPU samples.
- [SQLite's table-layout guidance](https://www.sqlite.org/withoutrowid.html)
  recommends ordinary rowid tables for large rows and BLOBs; index layout matters
  for missing-key lookups as well as cache hits.
- [NVIDIA's BVH construction analysis](https://developer.nvidia.com/blog/thinking-parallel-part-iii-tree-construction-gpu/)
  distinguishes construction cost, traversal cost and parallel occupancy.
  Use measured workload size and data-transfer cost when evaluating GPU work.

`surface-baseline --landscapes --month 7` adds offline geographic probes using
bundled monthly imagery and synthetic relief. Repeat with months 1, 4 and 10
for seasonal cold/warm generation comparisons. These probes include simple,
overlapping, dateline and fallback selections; they do not measure provider
loading or real terrain geometry. The default manifest fixtures remain present.
Use matching probe options and preserve the first sample separately from later
samples when assessing process-cold initialization.
