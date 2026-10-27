# Architecture

Low Poly Earth is a native, streamed surface globe. The application selects cube-sphere
cells from the camera, resolves their source data, and generates terrain and
feature triangles directly on CPU workers. Complete `SurfaceCell` products may
persist across runs; GPU packing and allocations remain transient. Production
generation, caching, collision and landmark loading contain no voxel volume or
occupancy-meshing fallback.

## Workspace structure

The dependency direction is deliberately one-way:

| Crate | Responsibility |
| --- | --- |
| `vearth-core` | Cube-sphere addressing, ellipsoid and frame math, triangle vertex formats, and semantic fingerprints. It has no I/O, GPU, or platform dependencies. |
| `vearth-data` | Geospatial provider traits and implementations, archive and raster decoders, provider selection, source caches, geocoding, and bundled data readers. |
| `vearth-gen` | Deterministic direct generation of `SurfaceCell` terrain, water, structures, landmarks, vegetation and collision. |
| `vearth-app` | Native windowing, camera and interaction, source assembly, streaming orchestration, generated-surface caching, residency, rendering, UI, capture, and replay. |
| `vearth-prep` | Offline commands that build checked-in or local data assets consumed by the other crates. It is not part of the interactive frame loop. |

`vearth-core` is the foundation; `vearth-data` builds on it, `vearth-gen`
combines both, and `vearth-app` composes all three. `vearth-prep` uses the data
and generation layers to produce assets without depending on the application.

## Runtime pipeline

### Selection and admission

The render thread derives a screen-error-driven wanted set of `CellId` values
from the current camera. The selection is a balanced quadtree cut over the six
cube faces. Resident ancestors and descendants provide coverage while a finer
cell is still loading. Fine arrivals draw independently of their siblings;
backdrops leave only when a complete substitute is available.

Visible and turn-guard leaves retain screen-error detail. In Explore mode,
complete background-only sibling groups coalesce after selection where the parent remains
outside that view, has no landmark lock, and preserves adjacent resolution.
The pass preserves the selected geographic footprint and cube-face boundary
leaves; freed capacity does not trigger additional foreground refinement.
Drone mode retains the surrounding background cut because its angular guard
alone does not guarantee enough time to acquire terrain during a turn.

Generation requests contain the exact wanted leaves and a minimum-level
bootstrap only where resident geometry does not already provide coverage.
Resident intermediate cells remain usable backdrops without causing ancestor
requests. The same explicit targets govern generation, upload admission and
prefetch leases.

Inherited LOD height uncertainty is bounded independently of horizontal sample
spacing, while recorded elevations retain their safety margin. Coarse curved
meshes contribute actual vertex elevations to physical height metadata; local
bounding-box corners are not terrain measurements. Loading a globe ancestor
must not make a stationary aerial camera request street-level descendants.
Mountain refinement uses relief from owned terrain samples, separately from
the content bounds that include buildings, trees and seam-closing skirts. Its
slope evidence is the step reached by the steepest tenth of a region's owned
quads, so isolated quay walls, cuttings and embankments in otherwise gentle
terrain do not promote a whole cell.
Inherited mountain metrics use only the requested descendant's portion of the
source height lattice. A shared CPU summary answers these queries without
extra elevation requests or intermediate meshes; below the source sample
spacing it retains the containing quad's measured range. Inherited vertical
uncertainty uses that region's measured height variation when available, while
retaining all recorded content extrema and the same safety margins.

Complete terrain measurements outlive mesh residency. The current and previous
selector queries protect the source records used for their own-cell, ancestor
and neighbouring-cell decisions; inactive history has a separate 16 MiB and
2,048-entry limit. Summaries share allocations. Retained measurements do not
provide render coverage, collision or camera-floor data and do not suppress
prefetch. If geometry is incomplete but elevation sampling succeeded, an active
query may retain that provisional observation across mesh eviction. It expires
when neither protected query uses it, regardless of the history budget; the
mesh remains incomplete and cannot enter the persistent cache. Missing-elevation
replacements are discarded on eviction. Provider or world resets clear history.

Missing cells enter a replaceable priority queue. Camera changes revise the
desired set, allowing queued work to be reprioritized or cancelled before it
consumes provider, CPU, or upload capacity. Completion and GPU upload queues
are bounded so acquisition and generation cannot grow memory without limit
when the render thread is busy.

### Provider resolution

`vearth-data::registry::Registry` presents elevation, surface height, imagery,
and vector chains through the synchronous source traits consumed by the
generator. Providers are ordered by native resolution and filtered by
coverage. A provider may return no data so sampling can fall through to a
coarser source.

The application always registers bundled global surface imagery and a bundled
terrain fallback. It attempts to resolve the configured global network sources
at startup. The same registry produces the in-app source controls and
attribution rows.

Global vectors come from a daily Protomaps build, pinned in the source cache
while it remains published. Opening a build over the network records its
immutable header, root directory and metadata beside the pin. A pin at most
five days old reopens from those bytes without startup network I/O; a
background check removes it only on a definitive 404 or 410, so the next
launch resolves a current build.

Global terrain is Mapterhorn's Terrarium-encoded 512 px pyramid: a planet
archive through z12 (Copernicus GLO-30 plus regional models) that omits all-sea
tiles, and regional archives to z18 where national elevation models exist. A
missing planet tile is open sea; a missing finer tile is synthesized from its
parent and samples identically. Mapterhorn has no bathymetry, so exactly-zero
sea decodes to a nominal floor and sea classification stays sign-based. The
generated-world cache identity does not include the terrain source, so
changing it requires a `GEN_VERSION` bump.

Raw source responses live in provider- and revision-scoped namespaces in the
source cache. The normal store is SQLite-backed; supported legacy file layouts
are imported through those namespaces. Source cache identity is distinct from
generated-world cache identity.

### Asynchronous and blocking acquisition

The application enables the `async-providers` feature by default. In that
configuration, the staged pipeline checks the generated-surface cache first,
then asynchronously prepares each enabled, covering provider that exposes an
asynchronous preparer for the cell's conservative source footprint. Preparation
warms the exact caches read by the synchronous generator. An unexpected
sampling miss in an async-backed provider does not initiate network I/O on a
CPU generation worker.

The staged pipeline can also use blocking provider implementations. Building
`vearth-app` without default features selects those implementations, and
`VEARTH_ASYNC_PROVIDERS=legacy`, `off`, or `0` disables async providers in a
default build. `VEARTH_PIPELINE=legacy` selects the combined blocking worker
path used for rollback and diagnostics.

These paths differ in scheduling and cancellation, not in the interfaces seen
by generation. Provider preparation records genuine no-data separately from
transient failures and circuit-open misses so degraded fallback output is not
published as a valid persistent cell.

### Direct generation

`generate_surface_cell` receives a `CellId` and a `Sources` view. Its output
contains triangle batches, a shared ground height field where applicable,
metric solid triangles, typed water triangles, bounds, road
guides and vegetation descriptors. A warm cache hit restores these products
without rerunning generation. Workers validate and pack the same batches into
`PreparedRenderCell`; arbitrary coordinates retain full precision when they
cannot round-trip through the compact vertex format.

Cube-sphere cells and their local frames provide addressing, sample coordinates
and spatial ownership. The shared 33-by-33 terrain corner lattice and 32-by-32
semantic masks describe surfaces, not occupied vertical columns. `ChunkFrame`, `grid_scale` and `SurfaceVertex` describe local coordinates and
triangle buffers.

Ground uses shared geodetic elevations and a consistent triangle diagonal.
The halo supplies normals across cell boundaries; source-derived tint and
material responses blend natural surfaces while mapped boundaries retain their
authority. Coarse cells project their vertices onto the ellipsoid. Skirts close
mixed-resolution seams for rendering and do not add collision walls.
Ground contact queries reuse the terrain heightfield; only carved shore
geometry and coarse ellipsoid cells need separate terrain triangles.

Mapped roads use physical-width ribbons with continuous joins on the graded
terrain triangles. Class priority resolves overlapping ribbons; railways keep
a physically spaced pair. Canonical bridge catalogs enrich mapped crossings
with outlines and structural information. Unmatched mapped bridge ways also
produce continuous metric decks and approaches from shared clearance profiles;
ordinary bridges do not require a local catalog to receive surface geometry.
Ordinary roads, bridge decks and their approaches share the adaptive road query.
Distant crossings follow the source's generalized road geometry; closer views
receive finer source geometry through the ordinary terrain-detail selection.
Bridge flags do not trigger a separate maximum-detail tile acquisition phase.

Buildings use source-polygon walls, roofs and eaves with holes, concavity and
elevated parts. Disconnected MVT building components receive separate envelopes;
interacting components retain their combined even-odd domain. Component ordinals
control admission and replacement; shared source IDs remain art seeds.
Fixed-resolution full-feature datums align foundations, roofs
and storeys across cells. Ground, water and bridge datums read terrain at the
9.6 m-at-the-equator level (z13) or coarser at every latitude, and each cell
prepares that detail only near the features that read it. Accepted measured surface data controls absolute roof
heights. Levels 11–15 use simple wall bands and roofs without facade ornament.
Unresolved roof rises become caps at the source top over the mapped footprint;
resolved roof silhouettes and measured interior peaks retain sampled geometry.
Below the finest level, interior roof sampling may deviate by up to a quarter
of the pixel at the selector's three-pixel target; roof outlines keep their
fixed tolerance.
At closer levels, optional relief requires one estimated pixel and individual
panes require two in both dimensions. Resolvable glazing and night windows remain
independent of the trim budget. Unresolved terrace recesses become closed caps
at the same outer top. Footprint holes and elevated clearances remain present.
Budget simplification retains source
owners. Missing required geometry marks the cell incomplete instead of silently
persisting an empty replacement. Curated ownership respects each landmark's
source-class rules.

Architectural priors use exact administrative context and the bundled GHS-SMOD
settlement grid. A full-precision source anchor keeps those choices independent
of the visiting cell. Reported use, roof shape and measured dimensions override
regional defaults; the optional stage catalog does not select world buildings.
Visual styles compose opening proportions, joinery, projections and rooflines
over those construction choices. A compiled place atlas supplies researched
local character through smooth authored influence extents, with ordinary infill
and scale limits. Its extents are artistic priors, not surveyed heritage borders.
See [regional art direction](REGIONAL_ART_DIRECTION.md) for the source coverage,
construction recipes, style sourcebook and uncertainty limits.

Building contact uses footprint prisms with flat roof caps, retaining holes,
raised clearances and part ownership. Bridge contact uses the continuous deck;
optional arches, cables and towers do not add collision detail.
Collision acceleration is initialized on the first contact query, independently
of source validation, caching and render admission. Median partitioning retains
the same spatial leaves and deterministic primitive traversal as a full sort.

Water retains source polygons, holes and admitted river/canal widths. Disconnected
source components are tessellated independently; interacting rings retain one
even-odd domain. Ocean surfaces use ellipsoid sea level. Inland water uses a continuous, globally
keyed DEM-derived datum; its depth is inferred optical/bed information, not
measured bathymetry or navigable depth. Water and ground meet through direct
surface geometry. Complete empty vector coverage distinguishes dry below-sea
land from the terrain-only ocean fallback. Typed water triangles retain liquid metadata; their visible tops also enter
the shared contact index for vehicle movement and camera collision.

A small [distinctive-water catalog](../crates/vearth-gen/assets/water/README.md) bakes optical
palettes into existing inland-water vertex colors. Pinned lake outlines provide
geographic masks independent of merged vector-tile IDs. Cell filtering bounds
lookup work; the catalog adds no GPU resources, geometry or collision changes.
Variable-color lakes use documented representative states.

Trees share silhouette-preserving triangle LODs and geographic placement.
Render roots have one owner; source exclusions and landmark tree rules apply
before geometry is emitted. Trees do not create collision geometry. Exact mapped
trees outside a cell still suppress overlapping inferred roots inside it.

### Landmarks and inspection scenes

The runtime reads bounded indexed `LMS1` assets with independent source-mesh
LODs, material colors and corner normals. Each asset is decoded once, then a
spatial index selects only triangles intersecting the current cell. Registry
placement, source replacement footprints and shared LOD locks keep adjacent
fragments coherent. The stage viewer uses the same canonical indexed asset.

`tools/export_landmark_surfaces.py` builds assets from recorded originals or
project-authored analytic shapes. Source and output hashes are retained in
per-model manifests. Singapore Merlion uses a permanent offline exposed-boundary
bake from its hash-verified shipped archive; no archive reader is linked into
runtime generation. Taj Mahal and Sagrada Familia are absent from the active
registry because their required originals are unavailable. Asset terms and
credits remain in the landmark source manifest.

### Generated-surface cache

The persistent identity includes cell address, generation version, surface codec
version and dataset tag. The dataset tag incorporates the vector terrain
identity, source profile and selected imagery month. Provider-control changes
invalidate generated content and disable canonical cache use for the session.
Only complete acquisition and validated geometry are persisted. Worker validation
failures retire the exact job revision and receive one retry per continuous
wanted intent; they cannot terminate the worker or create a per-frame retry loop.

`VXS4` entries store final triangle batches, detail bounds, bounded
shadow-omission spans, solid triangles and capsules, terrain/water query data,
terrain relief and its physical source lattice, road guides and vegetation
descriptors inside a checksummed Zstandard frame.
Readers also accept existing gzip entries in the same namespace. Both wrappers
require one complete frame; bounded readers validate output/window sizes,
checksums, indices and finite values. Old voxel payloads are rejected; there is no compatibility decoder that
reconstructs a voxel world. A compressed memory LRU fronts SQLite BLOB storage;
the file-store fallback uses the same surface codec. Source-provider bytes use
separate namespaces and remain independent of generated output.

### Residency, rendering, and interaction

The render thread drains completed prepared cells into an upload staging area,
drops work that no longer belongs to the desired view, and uploads within
per-frame resource budgets. A generational resident store keeps CPU metadata
and surface collision aligned with GPU allocations. The drawable cut is
rebuilt as cells arrive so a coarser resident cell continues to cover any
incomplete refinement.

The renderer draws terrain and structure triangles together with instanced flora,
vehicles, landmarks, sky, atmosphere, weather, shadows, night lighting, and
post-processing. Nearby albedo uses material value/chroma bounds and broad
geographic hue variation; orbital imagery retains its measured colors. Mapped
forest, scrub, grass, and meadow use continuous climate-conditioned palettes
and supported rock exposure while their mapped boundaries remain authoritative.
Annual climate and broad land cover inform appearance, not exact ecological or
geological identity. Altitude alone does not establish snow or sand. A
single shadowed key and hemispheric fill share the same illustrative response
across world surfaces and flora. Camera navigation, place search, the map,
solar controls, weather settings, and vehicle input all feed application state
on the render thread. Vehicle collision and camera grounding query resident triangle, capsule, ground
and water data without remote-provider work on the frame loop. Finest resident
coverage owns point, swept-box and ray queries; superseded parent geometry is
clipped to uncovered regions.

Kestrel, Sparrow and Phase Manta use Blender-exported metre-space triangle meshes
with authored corner normals and UVs. Kestrel and Manta adapt Quaternius CC0
spacecraft; their sRGB base-color textures have linear-light mip chains. Sparrow
uses shaped bodywork and separate paint, trim, saddle and glass finishes. Transient material IDs 96–102 distinguish paint,
metal, graphite, cockpit glass and emissive surfaces without changing persisted
terrain semantics. Their specular response shares the sky reflection cache,
Sun shadows and aerial perspective. Exhaust uses a bounded additive instance
pass after the opaque craft, with scene-depth intersection fading and continuous
throttle/boost response. It writes no depth and does not enter cloud history.
Legacy watercraft preferences resolve to Kestrel; the watercraft is not selectable.
Run `cargo test -p vearth-app --test vehicle_material` for production material
validation and numerical energy checks on a native graphics adapter.

Nearby world materials use one shared mipmapped RGBA atlas for tone, surface
gradients, and roughness. Metric triplanar coordinates and translated tile
blends retain their phase across camera-origin wraps; coarse globe cells skip
this detail. Inferred brick, stone, plaster, timber, tile and slate use a
metre-scale world projection with geodetic-height courses and a complete
mip chain. The height phase occupies an unused terrain-uniform lane; vertex
storage and draw submission stay unchanged. The bundled stone source and its
generation provenance live in `crates/vearth-app/assets/materials`. Daylight
fill integrates the same sky model used by the visible sky; weather and local night lighting remain dynamic.

Natural terrain tops reserve face id 6 with an explicit normal. Their former
AO lane selects one of four corner material responses; the material id retains
source material identity. Interpolated weights blend atlas tone, tangent relief
and roughness without changing the packed 12-byte vertex layout. Roads, fields,
water and structures retain their own materials. Broad geographic color and
filtered physical-scale weathering remain independent of terrain LOD.

World surfaces and flora share aerial perspective whose horizon radiance comes
from that sky model. Two analytic height segments retain Earth curvature and
near-ground aerosol density; measured visibility and weather set extinction.
The Sun integrates spherical atmospheric transmission only near its disc, so
its color and attenuation change with elevation, weather and observer altitude.
Near the surface, Sun and Moon presentation diameters are 1.75 times angular
size in the high sky and grow smoothly to four times near the local geometric
horizon. The boost eases between about 3 and 30 degrees of elevation; observer
altitudes between 8 and 80 km return both discs to unmodified angular size.
This perceptual cue does not change celestial directions or world lighting.
Low, Sun-facing sky can take on richer orange tones according to solar angle,
aerosol and humidity. Visible sky, sky reflections and distant haze share that
bounded response; foreground exposure and diffuse ambient remain unchanged.
The Moon retains Sun-directed phase shading with filtered procedural maria
in a stable celestial-north frame; the markings are not a surveyed lunar map.
This is a bounded atmospheric approximation, without
multiple scattering or volumetric terrain shadows. Run
`cargo test -p vearth-app --test atmospheric_lighting` for shader validation and,
when a graphics adapter is available, comparison with dense optical integrals.

Water combines filtered travelling waves with irregular gradients from the
shared material atlas and air-water Fresnel. Unresolved slope variance is
retained in the fourth power of perceptual roughness. Water and glass reflect
the current fitted sky, including weather and night response, through one
shared 64×64 RGBA16F environment texture. Its generation is included in world
GPU timing. Water colour bakes wavelength-dependent absorption, water-column
scattering and geographic substrate variation into the existing vertex tint.
A coastal-shelf depth prior and inferred inland optical depth control how much seabed
colour survives; climate blends clear, glacial and sediment-rich families.
The substrate fields share geographic coordinates across cells and filter out
at coarse LODs. Visible ocean shallows additionally shade stationary, warped
seabed patches at 4–256 m scales, filtered by pixel footprint. Dark water and
inland water skip this detail. This adds no render pass, texture or vertex
attribute. Water remains opaque; scene-object reflections, refraction and source-derived shore
foam are not implemented. Substrate patches are procedural appearance, not
surveyed seabed or collision depth.
`cargo test -p vearth-app --test water_material` checks the production shader
against numerical sky references when a graphics adapter is available.

The HDR resolve smooths silhouette coverage after exposure, filmic mapping, and
creative grading, preserving flat fields and sharp material detail. Short-range
contact shading reconstructs metre-space positions from the same reversed-Z
depth attachment; its eight occlusion taps have a 2 m radius and at most 14%
darkening. Sky, distant surfaces, and bright HDR pixels are excluded. This is a
post-lighting contact approximation, not indirect lighting; it adds no history
or render targets. `cargo test -p vearth-app --test spatial_aa` validates shader
syntax and numerical image behavior when a graphics adapter is available.

## Determinism boundary

`vearth-core` geometry results are deterministic for identical
inputs; optional profiling also reports elapsed time. `vearth-gen` defines
world content as a function of the cell address, resolved source data, and
`GEN_VERSION`; it does not use wall-clock time or unseeded randomness for
semantic output. Surface-cache round trips and prepared-render packing have
semantic-fingerprint tests protecting geometry, query products and metadata.

Determinism does not freeze external datasets. A different Protomaps terrain
identity, imagery month, source-profile tag, generation version, or surface
codec version belongs to a different generated-cache identity. Other source
revisions may scope their own source caches without changing that generated
identity. A non-default provider selection instead disables canonical
generated-cache use for the session. Network timing, worker scheduling,
telemetry durations, live clock mode, interactive camera input, and GPU frame
timing are operational state rather than generated-world semantics. Replay and
capture modes add fixed inputs where repeatable output is required.

## Process and storage boundaries

The native process separates work by ownership:

- The event/render thread owns the window, application state, UI, GPU device,
  resident set, transition state, and upload admission.
- Provider preparation runs on asynchronous acquisition tasks or dedicated
  blocking lanes, depending on configuration.
- CPU workers run deterministic surface generation, validation and render
  packing.
- The source cache stores downloaded provider bytes, and the generated cache
  stores complete `SurfaceCell` payloads. GPU state consists of the resident
  view plus renderer-global pipelines, lookup textures, environment resources,
  and static scene buffers.
- Preferences, captures, logs, source data, and generated cells are mutable
  runtime data written to ignored runtime directories, not tracked source
  files.

Unpackaged launches place runtime data below the current working directory;
the packaged macOS application uses per-user system directories.
`VEARTH_RUNTIME_ROOT` overrides those locations for isolated runs. See
[Development](DEVELOPMENT.md) for the concrete paths and validation commands,
and [Data and attribution](DATA_AND_ATTRIBUTION.md) for source-specific terms.
