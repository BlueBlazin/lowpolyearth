# Landmark asset provenance

Runtime landmarks are indexed triangle meshes in `surfaces/*.lms`,
`city-surfaces/*.lms`, `heritage-surfaces/*.lms`, `skyscraper-surfaces/*.lms`,
and `monument-surfaces/*.lms`. Each adjacent `*.source.json` records the exact source hash, transform kind, output
hash and triangle LOD counts. The application decodes these meshes once and
spatially clips source triangles; it never reads the archived occupancy assets.

Regenerate direct source meshes with Python, NumPy, trimesh and
fast-simplification:

```
python3 tools/export_landmark_surfaces.py --models /path/to/models3d --recovered /path/to/recovered.json
```

The recovered-source list contains `kind`, `path`, `url` and `sha256` for the
recorded third-party files below. Hashes and URLs are verified by the exporter.
The source models remain external authoring inputs and are not needed at runtime.

Merlion is a preservation bake of its hash-verified shipped archive because its
original authoring scene is unavailable. Reproduce it with
`python3 tools/bake_archived_landmark_surfaces.py`. This merges only coplanar,
same-material exposed faces without changing boundaries. The differently
licensed scan currently at `models3d/merlion` is not imported or substituted.
The original provenance record below is retained without a new ownership claim.

Project-owner-authored assets are covered by the repository's MIT `LICENSE`. Third-party assets retain their recorded terms.

License references used below: [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
and [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/).

## Authoring invariants

- Use project-owner-supplied meshes or original project-authored geometry. Do
  not substitute an external model.
- Every new landmark must explicitly audit its generated terrain. Assume the
  elevation source contains the landmark's built mass unless bare-earth data
  is verified, and define a DEM-cleanup footprint when it does. The source mesh
  and corrected terrain must share the clean perimeter-derived base so
  structures do not stand on duplicated hills, dunes, or building-shaped
  terrain. City-catalog ground patches also define building suppression; the
  heritage catalog uses audited convex ownership polygons independently of
  lower-only terrain cuts. Landmarks without either use their model envelope.
  Keep suppression limited to the architectural fabric being replaced.

## Monuments and gateways

`tools/monument_catalog/inventory.py` owns the thirty additional sites in
`monument-surfaces/catalog.json`. Each original model records its architectural
references, published dimensions, estimated details, materials, anchor, and
orientation. Reference photographs are not embedded textures. Source notes
distinguish simplified relief and inferred proportions from measured dimensions.
Mapped placements retain the recorded © OpenStreetMap contributors, ODbL 1.0
attribution; original geometry uses the repository `LICENSE`.

```sh
python3 tools/build_monument_catalog.py
python3 tools/build_monument_catalog.py --check
PYTHONPATH=tools:tools/city_catalog python3 -m unittest monument_catalog.test_assets
cargo test -p vearth-gen landmarks
python3 tools/capture_heritage_catalog.py --catalog crates/vearth-gen/assets/landmarks/monument-surfaces/catalog.json --only stonehenge --app target/debug/vearth-app --survey target/debug/city-survey --output output/monuments/world
```

The generator checks three non-increasing LODs, material IDs, dependency/output
hashes, and surveyed foundation contacts. Frozen raw elevation grids under
`tools/monument_catalog/references/ground` set component heights and embedded
architectural toes; all thirty leave the terrain unchanged. No terrain flatten,
cut, grade, or water correction is registered. The fixed survey datum anchors
mesh placement. Source roads remain available through open gateways. `--catalog` also selects this
pack in `tools/render_heritage_catalog.py` for inspection of encoded geometry.

## Heritage catalog

`tools/heritage_catalog/inventory.py` defines the 52-site collection. Its
geometry modules use metric plans and architectural references retained in
`tools/heritage_catalog/references`. Authored geometry uses the repository
`LICENSE`; OpenStreetMap-derived plans and placement retain their recorded
© OpenStreetMap contributors, ODbL 1.0 attribution.

Eltz's measured terrain additionally uses the state DGM1 dataset:
© GeoBasis-DE / LVermGeoRP 2026, [dl-de/by-2-0](https://www.govdata.de/dl-de/by-2-0),
[www.lvermgeo.rlp.de](https://www.lvermgeo.rlp.de) [Daten bearbeitet]. The retained
terrain reference records the source tile, state metalink checksum, coordinate
system, and derived samples; these data attribution terms remain applicable.
Predjama's local relief derives from the [ARSO national terrain service](https://gis.arso.gov.si/arcgis/rest/services/Slovenija_DMR_D96TM/ImageServer),
© ARSO / Slovenian Environment Agency. Its retained samples and authored cliff
geometry acknowledge that source under the agency's [attribution-based reuse terms](https://kazalci.arso.gov.si/index.php/sl/node/24460).

```sh
python3 tools/build_heritage_catalog.py
python3 tools/build_heritage_catalog.py --check
PYTHONPATH=tools:tools/city_catalog python3 -m unittest heritage_catalog.test_geometry
cargo build --release -p vearth-app --bin vearth-app --bin city-survey
python3 tools/capture_heritage_catalog.py --only qaitbay-citadel
```

The bake writes three bounded LODs, dependency/output hashes, and the compiled
registry. `--only` accepts site or group names during authoring. The final
unfiltered check requires the complete inventory. Convex ownership polygons
preserve open courtyards; terrain cuts lower contaminated elevation within
feathered, explicit bounds relative to each model's datum. Activation bounds
must contain the geometry, ownership polygons, and terrain corrections.
Dense relief corrections use a cached spatial index; feather distances are
measured to finite polygon edges so acute corners stay inside those bounds.
Explicit `terrain_grades` can also fill verified source depressions. They act
before lower-only cuts, use bounded feathered planes, and never change the
architectural mesh datum. Retained source buildings use the same graded ground.
Grades require a fixed model datum and cannot be combined with `terrain_flatten`.
Separate opt-in water cuts lower the inferred inland water datum inside a
bounded reach before surface and riverbed generation. They require a fixed
building datum, preserve oceans, and record estimated levels as estimates.

Material assignments share the engine atlas. Weathered limestone, veined marble,
and earthen plaster add distinct tone, relief, and roughness without embedded
reference images. Blender previews inspect geometry; production captures verify
materials, terrain, and lighting. Rebuild the executable after baking before
capturing. Captures default to local solar noon, isolate generated-cell caches
by executable hash, and record camera, executable, asset, and image hashes.

For footing checks, `city-survey LAT LON OUTPUT --ground-only --ground-points
POINTS.json --landmark-ground NAME` accepts east/north offsets in metres and
exports raw elevation, the compiled landmark's continuous terrain correction,
canonical model coordinates, and its local mesh datum. These samples do not
include terrain-grid interpolation, water-bed carving, or authored rock support;
verify those contacts in low-angle engine views as well.
Retained source buildings sample the same corrected ground in absolute geography,
independently of the receiving cell. Corrected heritage sites therefore require
explicit clean datums. Support toes may extend slightly below terrain to close
small gaps from mesh interpolation; visible floors and roof levels retain their
architectural datums.

## Handcrafted city catalog

`tools/city_catalog/{iconic,sculptural,waterfront,outdoors}.py` and
`tools/city_catalog/expansion_*.py` contain original metric geometry authored
for this project. Reference URLs establish architectural
scale, identity and geographic placement; no external meshes, images or ad
artwork are imported. Authored mesh surfaces use the repository `LICENSE`.
OpenStreetMap placement, footprint measurements and approach alignments are
© OpenStreetMap contributors under ODbL 1.0; source identities and measurements
are recorded in the model manifests.

With Python, NumPy and the workspace Rust toolchain installed:

```
python3 tools/build_city_catalog.py
python3 tools/build_city_catalog.py --check
```

The generator writes three indexed LODs per model, the runtime registry, and
`city-surfaces/catalog.json`. Manifests hash the geometry source, shared
primitives, generator and compressed output. The existing landmark consumer
provides geographic placement, source suppression, spatial clipping and collision.
Authored `terrain_flatten` records compile into the same runtime ground patches;
their feathered bounds participate in activation and source-building suppression.
Measured fixed datums and source-footprint audits stay with each model record.
An audited `max_source_building_span_m` can preserve a larger neighboring source
envelope that overlaps the replacement footprint.
Static colored illumination uses the renderer's city-light material and existing
night-lighting switch.

Facade attachments are separate finite recipes in `../cities/signs.json`, bound
to surveyed source-building identities and geographic wall points. Their
OpenStreetMap-derived placement retains the attribution recorded in that file.
`python3 tools/build_city_signs.py --check` verifies the compiled fixture list;
omit `--check` to regenerate it. Unmatched or hidden source walls emit no signs.

World camera recipes live in `tools/city_catalog/captures.json`.
`python3 tools/capture_city_catalog.py --help` describes reproducible in-engine
captures and source-elevation checks. Capture the additional collection with:

```
python3 tools/capture_city_catalog.py --recipes tools/city_catalog/captures-expansion.json --output-dir output/city-catalog-expansion
python3 tools/capture_city_catalog.py --recipes tools/city_catalog/captures-outdoors.json --output-dir output/city-catalog-outdoors
```

The capture tool resolves authored pad datums before aiming cameras, so a roof
height in the source elevation cannot shift the target above its corrected model.

## Source authoring records

The following authoring records identify the original converter inputs and
output identifiers. Current indexed output names and verified source hashes are in
`surfaces/*.source.json`; these records preserve the original asset attribution; only the Merlion
occupancy archive is retained as an offline source input.
The source package is intentionally not stored in this repository.

| Landmark | Source package | Converter kind | Output |
| --- | --- | --- | --- |
| Angkor Wat | `angkor-wat/angkorwat.glb` | `angkor-wat` | `angkor_wat.vxl` |
| Buckingham Palace | `buckingham-palace/buckingham.glb` | `buckingham-palace` | `buckingham_palace.vxl` |
| Burj Al Arab | `burj-al-arab/burjalarab.glb` | `burj-al-arab` | `burj_al_arab.vxl` |
| Burj Khalifa | `burj-khalifa/burjkhalifa.glb` | `burj-khalifa-authored` | `burj_khalifa.vxl` |
| Brooklyn Bridge | `brooklyn-bridge/brooklyn.glb` | `brooklyn-bridge-authored` | `brooklyn_bridge.vxl` |
| Chrysler Building | `chrysler-building/chrysler.glb` | `chrysler-building` | `chrysler_building.vxl` |
| CN Tower | `cn-tower/cntower.glb` | `cn-tower` | `cn_tower.vxl` |
| Flatiron Building | `flatiron-building/flatiron.glb` | `flatiron-building` | `flatiron_building.vxl` |
| Golden Gate Bridge | `golden-gate-bridge/goldengate.glb` | `golden-gate-bridge-authored` | `golden_gate_bridge.vxl` |
| Golden Temple | `golden-temple/goldentemple.glb` | `golden-temple` | `golden_temple.vxl` |
| Himeji Castle | `himeji-castle/himeji.glb` | `himeji-castle` | `himeji_castle.vxl` |
| Moscow Kremlin | `kremlin/kremlin.glb` | `kremlin` | `kremlin.vxl` |
| Leaning Tower of Pisa | `leaning-tower-of-pisa/pisa.glb` | `pisa` | `leaning_tower_of_pisa.vxl` |
| Louvre Pyramid | `louvre-pyramid/louvre.glb` | `louvre-pyramid` | `louvre_pyramid.vxl` |
| Marina Bay Sands | `marina-bay-sands/mbs.glb` | `marina-bay-sands` | `marina_bay_sands.vxl` |
| Merdeka 118 | `merdeka-118/merdeka118.glb` | `merdeka-118` | `merdeka_118.vxl` |
| Singapore Merlion | `merlion/merlion.glb` | `merlion-authored` | `merlion.vxl` |
| Notre-Dame de Paris | `notre-dame/notredame.glb` | `notre-dame` | `notre_dame.vxl` |
| Nyhavn | `nyhavn/nyhavn.glb` | `nyhavn` | `nyhavn.vxl` |
| One World Trade Center | `one-world-trade-center/onewtc.glb` | `one-world-trade-center` | `one_world_trade_center.vxl` |
| Parthenon | `parthenon/parthenon.glb` | `parthenon` | `parthenon.vxl` |
| Petronas Towers | `petronas-towers/petronas.glb` | `petronas-towers` | `petronas_towers.vxl` |
| St Basil's Cathedral | `st-basils-cathedral/stbasils.glb` | `st-basils` | `st_basils_cathedral.vxl` |
| Steinway Tower | `steinway-tower/steinway.glb` | `steinway-tower` | `steinway_tower.vxl` |
| Sydney Opera House | `sydney-opera-house/sydney_opera_house.glb` | `opera` | `sydney_opera_house.vxl` |
| Taipei 101 | `taipei-101/taipei101.glb` | `taipei-101` | `taipei_101.vxl` |
| The Gherkin | `the-gherkin/gherkin.glb` | `the-gherkin` | `the_gherkin.vxl` |
| The Shard | `the-shard/shard.glb` | `the-shard` | `the_shard.vxl` |
| Tower Bridge | `tower-bridge/towerbridge.glb` | `tower-bridge-authored` | `tower_bridge.vxl` |
| White House | `white-house/whitehouse.glb` | `white-house` | `white_house.vxl` |

Conversion preserves the scenes' authored metre scale except where a
model-specific fit is recorded in the converter. It restores the documented
Blender +X-east/+Y-north authoring orientation after glTF's Y-up conversion and
surface-voxelizes all seven renderer LODs. Angkor Wat and Himeji retain their
primary-monument authoring origin so their asymmetric precincts remain
correctly offset from the geodetic anchor. Each asset retains its documented
architectural origin; bridge, Merdeka, Merlion and Petronas +X longitudinal
axes are baked onto runtime +z so registry headings remain literal surveyed
bearings. Every named flat material is kept as an independent palette entry
with its authored display RGB value. This is especially important for St
Basil's per-face blue/white, green/yellow, red/white, green/red and green/white
dome patterns; Nyhavn's individual house facades; and the flags, glazing,
metals, stone bands and roof finishes across the other scenes. Runtime
placement uses a clean perimeter-derived terrain level beneath each
architectural footprint, with a feathered transition to the surrounding DEM,
and anchors the voxel asset to that same corrected elevation.

## Statue of Liberty

- Source: `public_dataset/Stanford3D/statue_of_liberty.obj` in
  [leihui6/BENBV](https://github.com/leihui6/BENBV)
- Source commit: `d361fd6e9c87f74ab61736dab7126d7a2929286a`
- Repository license: MIT
  ([`LICENSE` at the source commit](https://github.com/leihui6/BENBV/blob/d361fd6e9c87f74ab61736dab7126d7a2929286a/LICENSE)).
- Upstream notice: Copyright (c) 2024 Lehu Lee. The complete upstream license
  is retained in `statue_of_liberty-LICENSE.txt` beside this manifest and copied
  into macOS packages as `LICENSE-StatueOfLiberty-MIT.txt`.
- Output: `statue_of_liberty.vxl`
- Converter kind: `liberty`
- Conversion: Z-up to Low Poly Earth Y-up, centred horizontally, uniformly scaled to
  the monument's full 92.99 m height, solid voxelization at seven LODs.

## Sydney Opera House

- Authorship: project-owner-authored.
- Source: `models3d/sydney-opera-house/sydney_opera_house.glb`, the owner's
  Blender exterior reconstruction. The local package includes the reproducible
  `build_sydney_opera_house.py` builder and independent export checks.
- Source GLB SHA-256:
  `b1c31271f0d1b91c5044f0ff3fc7cc3703ba295a3481c2177bdb99a223b0f534`.
- Derived output terms: the repository's MIT `LICENSE`. No third-party mesh is used in this output.
- Output: `sydney_opera_house.vxl`
- Output SHA-256: `29598bb099664791e0419b9e57a27af69a20151fd681c7a5236c4c4ffcb4ec4e`.
- Converter kind: `opera`
- Conversion: preserves the authored 120 × 183 × 67 m envelope and sea-level
  origin. glTF -Z (toward the harbour) becomes runtime +z; the registry applies
  the 20° harbour bearing. Surface voxelization builds seven LODs without
  globally filling the overlapping shells and hollow halls, with independent
  granite, paving, two ceramic finishes, concrete
  ribs, bronze glazing, mullions, joints and steel-rail palette entries. RGB
  tints match the builder's sRGB colors. The authored 3.1 m concourse is aligned
  with clean perimeter terrain using a -3.1 m model offset; DEM cleanup and
  ordinary-building suppression cover the full podium width.
- The source package's architectural drawings and photographs are research
  references with their own credits and rights, recorded in its `README.md`.
  They are not textures and are not embedded in the GLB or bundled voxel asset.

Rebuild this source with the indexed exporter above and
`--only sydney-opera-house`. Source transformations and material palettes are
in `tools/landmark_sources.py`.

## Taj Mahal (not linked)

The original source file is unavailable; the model and its terrain suppression
are excluded from the runtime registry. Attribution is retained below.

- Source: `Taj_Mahal_BB.stl` from
  [bbinnard on Thingiverse](https://www.thingiverse.com/thing:859377)
- Source file ID: `1363311`
- License: Creative Commons Attribution-ShareAlike 4.0
- Output: `taj_mahal.vxl`
- Converter kind: `taj`
- Derived output license: Creative Commons Attribution-ShareAlike 4.0
- Conversion: Z-up to Low Poly Earth Y-up, centred, scaled to 73 m tall and a 105 m
  outer footprint, solid voxelization at seven LODs. At runtime eight clean
  perimeter elevation samples replace DEM-baked mausoleum mass beneath the
  116 m architectural pad, with a feathered join to the surrounding garden;
  that corrected level also anchors the voxel asset.

## Sagrada Família (not linked)

The original source file is unavailable; the model and its terrain suppression
are excluded from the runtime registry. Attribution is retained below.

- Source: `sagradafamilia_fixed.stl` from
  [nelcats on Thingiverse](https://www.thingiverse.com/thing:505726)
- Source file ID: `1059989`
- License: Creative Commons Attribution-ShareAlike 4.0
- Output: `sagrada_familia.vxl`
- Converter kind: `sagrada`
- Derived output license: Creative Commons Attribution-ShareAlike 4.0
- Conversion: Z-up to Low Poly Earth Y-up, centred, scaled to 172.5 m tall and a
  100 m outer footprint, solid voxelization at seven LODs with detached source
  debris removed.

## Christ the Redeemer

- Source: [Christ the Redeemer, Rio de Janeiro, Brazil](https://sketchfab.com/3d-models/christ-the-redeemer-rio-de-janeiro-brazil-90d53b3cb4b042ce845a24baa679109e)
  by Brian Trepanier, archived as Objaverse UID
  `90d53b3cb4b042ce845a24baa679109e`
- License: Creative Commons Attribution 4.0
- Output: `christ_the_redeemer.vxl`
- Converter kind: `christ`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: isolated the complete statue and pedestal from the surrounding
  photogrammetry terrain, centred, uniformly scaled to 38 m tall, then retained
  the grounded connected component at all seven LODs.

## Brooklyn Bridge

- Source: project-owner-supplied `brooklyn.glb` from
  `models3d/brooklyn-bridge`.
- Output: `brooklyn_bridge.vxl`
- Converter kind: `brooklyn-bridge-authored`
- Conversion: preserves the authored real-metre 1,133.8 × 44.2 × 85.3 m
  anchorage-to-anchorage structure, maps its +X crossing axis onto runtime +z,
  and retains granite, cable, steel, roadway and timber-promenade groups at all
  seven LODs. The fixed sea-level base and OSM-derived 316.2° bearing avoid
  terrain/approach drift.

## London Eye

- Source: [London Eye](https://sketchfab.com/3d-models/7fc09e8da98d4f77a92dcb9f1aba0628)
  by Brian Trepanier, archived as Objaverse UID
  `7fc09e8da98d4f77a92dcb9f1aba0628`
- License: Creative Commons Attribution 4.0
- Output: `london_eye.vxl`
- Converter kind: `london-eye`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: cropped the complete wheel, A-frame and boarding base from its
  South Bank scan, normalized it to a 135 m height, and surface-voxelized
  seven LODs with steel, glazing, concrete and trim colors.

## Singapore Merlion

- Source: project-owner-supplied `merlion.glb` from `models3d/merlion`.
- Output: `merlion.vxl`
- Converter kind: `merlion-authored`
- Conversion: preserves the watertight 8.6 m authored statue and ivory finish,
  maps its documented +X facing direction onto runtime +z, and surface-
  voxelizes seven LODs. Its ladder begins at 0.3 m rather than the standard
  0.6 m, letting the compact sculpture use a local half-pitch world detail
  level. A small clean-perimeter fountain-pad correction keeps the model off
  DEM-baked sculpture mass while retaining terrain-relative placement.

## Palace of Westminster

- Source: project-owner-supplied `westminster.glb` from the local authoring
  package `models3d/palace-of-westminster` (the source package is not stored
  here)
- Output: `parliament.vxl`
- Converter kind: `parliament`
- Conversion: removed only the scene's presentation ground, river plane,
  lawns, trees and detached yard boundary walls. Preserved the real-scale
  complete complex: Palace ranges, riverside terrace and retaining wall,
  Elizabeth Tower, Victoria Tower, Central Tower and Westminster Hall. The
  authored limestone, stone trim, weathered stone, two roof finishes, glazing,
  gilt, clock and Union flag materials remain distinct across seven LODs. Their
  voxel tints are calibrated against the supplied model's rendered appearance
  so its scene-linear Blender colors do not become saturated flat display
  colors. The horizontal placement is fitted to the OSM Elizabeth and Victoria
  Tower footprints while preserving the source's correct vertical scale. Big
  Ben is not emitted as a separate stamp, preventing overlap with this model.
  Runtime terrain correction uses clean perimeter samples beneath the full
  architectural footprint and shares that corrected base with the voxel asset.

## Space Needle

- Source: [Space Needle](https://sketchfab.com/3d-models/a0cbf7ba6d6342f4983b86ea20144ace)
  by EDXtawny, archived as Objaverse UID
  `a0cbf7ba6d6342f4983b86ea20144ace`
- License: Creative Commons Attribution 4.0
- Output: `space_needle.vxl`
- Converter kind: `space-needle`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: centred and normalized to 184 m, then surface-voxelized at seven
  LODs with separate steel, glazing, saucer cladding, roof and footing colors.

## Giza Necropolis

- Source: [Plateau de Gizeh](https://sketchfab.com/3d-models/b30a5047bd1e4b28a10e438df2bc1428)
  by pierre391, archived as Objaverse UID
  `b30a5047bd1e4b28a10e438df2bc1428`
- License: Creative Commons Attribution 4.0
- Output: `giza_necropolis.vxl`
- Converter kind: `giza`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: independently cropped Khufu, Khafre and Menkaure from the plateau
  scan, discarded surrounding terrain, normalized each to surveyed footprint,
  height and relative placement, rebased the retained stone to y=0, and
  surface-voxelized seven LODs with casing, course and shadow colors. At
  runtime the raw DEM is replaced only under each pyramid by a level derived
  from eight clean perimeter samples, with a feathered transition back to the
  real plateau; the same corrected level anchors the voxel model.

## Colosseum

- Source: [Colosseum](https://sketchfab.com/3d-models/e749705838044be78ced42205f9f9dda)
  by Carlos.Maciel, archived as Objaverse UID
  `e749705838044be78ced42205f9f9dda`
- License: Creative Commons Attribution 4.0
- Output: `colosseum.vxl`
- Converter kind: `colosseum`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: independently normalized to 190 × 158 × 49 m and
  surface-voxelized at seven LODs with travertine, exposed-core, arcade,
  arena and vegetation colors. A buried elliptical footing follows the real
  footprint and absorbs local terrain variation without changing visible size.

## Brandenburg Gate

- Source: [Brandenburg Gate](https://sketchfab.com/3d-models/f0501d1cc3d04c78a0bb8efd6058cb90)
  by Furry12355, archived as Objaverse UID
  `f0501d1cc3d04c78a0bb8efd6058cb90`
- License: Creative Commons Attribution 4.0
- Output: `brandenburg_gate.vxl`
- Converter kind: `brandenburg`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: removed the presentation plinth, normalized the gate to
  68 × 24 × 32 m, and surface-voxelized seven LODs with sandstone, colonnade,
  relief, quadriga and foundation colors.

## Arc de Triomphe

- Source: [Arc de Triomphe Mini](https://sketchfab.com/3d-models/arc-de-triomphe-mini-1ad3fc2ded834e7ba0fa54f09d661583)
  by Le FabShop, archived as Sketchfab UID
  `1ad3fc2ded834e7ba0fa54f09d661583`
- License: Creative Commons Attribution 4.0
- Output: `arc_de_triomphe.vxl`
- Converter kind: `arc`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: preserved the clean isolated architectural geometry, normalized
  it to the surveyed 45.1 × 22.2 × 50 m envelope, solid-voxelized seven LODs,
  and assigned separate limestone, cornice, relief, vault-shadow and buried
  footing colors.

## Tower Bridge

- Source: project-owner-supplied `towerbridge.glb` from
  `models3d/tower-bridge`.
- Output: `tower_bridge.vxl`
- Converter kind: `tower-bridge-authored`
- Conversion: preserves the authored 312 × 56.4 × 66.3 m water-level model,
  maps its northward +X crossing axis onto runtime +z, and surface-voxelizes
  seven LODs while preserving stone, bridge-blue/white steel, road, glazing,
  roof, flags, red and gilt groups. Runtime placement follows the 22.8° mapped
  bridge axis with a fixed water-level base.

## Golden Gate Bridge

- Source: project-owner-supplied `goldengate.glb` from
  `models3d/golden-gate-bridge`.
- Output: `golden_gate_bridge.vxl`
- Converter kind: `golden-gate-bridge-authored`
- Conversion: uses the authored 2,737 × 46 × 229.1 m full-scale structure.
  Source +X maps to runtime +z toward Marin on the surveyed 353.7° axis.
  International-orange steel,
  concrete, road, lane and walkway groups survive all seven LODs; the model
  remains fixed to sea level rather than inheriting contaminated approach DEM.

## Empire State Building

- Source: [Empire State Building V2](https://sketchfab.com/3d-models/empire-state-building-v2-20378f7cc4e0467aaf797c42e9fe4a23)
  by myyko, archived as Sketchfab UID
  `20378f7cc4e0467aaf797c42e9fe4a23`
- License: Creative Commons Attribution 4.0
- Output: `empire_state_building.vxl`
- Converter kind: `empire-state`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: uses the clean isolated architectural model, independently
  normalized to 124 × 64 × 443.2 m, and surface-voxelized at seven LODs
  with limestone shade, window rhythm, setback cornices, crown, stainless mast
  and beacon colors.

## Eiffel Tower

- Source: [Eiffel Tower](https://sketchfab.com/3d-models/0cee46511c5d42b49efcc173127c2ca8)
  by Flower_gamet, archived as Objaverse UID
  `0cee46511c5d42b49efcc173127c2ca8`
- License: Creative Commons Attribution 4.0
- Output: `eiffel_tower.vxl`
- Converter kind: `eiffel`
- Derived output license: Creative Commons Attribution 4.0
- Conversion: centred, normalized to a 125 m footprint and 330 m height, and
  surface-voxelized seven LODs while retaining the four legs, arches, lattice,
  platforms and antenna with multiple iron shades, glazing and beacon colors.

## Tokyo Tower

- Source: project-owner-supplied `tokyotower.glb` from the local authoring
  package `models3d/tokyo-tower` (the source package is not stored here)
- Output: `tokyo_tower.vxl`
- Converter kind: `tokyo-tower`
- Conversion: preserved the source's real-metre 81 m footprint, 332.9 m
  height, Y-up orientation, four open lattice legs, two observation decks and
  authored international-orange, white, and glass material assignments.
  Surface-voxelized at seven LODs. At runtime a clean perimeter-derived terrain
  level replaces DEM-baked tower mass beneath the footprint and anchors the
  model to the same corrected base.

## Washington Monument

- Source: exact offline construction from published surveyed dimensions:
  169.294 m total height, 16.764 m base width, 152.4 m shaft and 16.894 m
  pyramidion. No external mesh is stored or used.
- License: no external source-model license (the geometry is authored by this
  project from published dimensions)
- Output: `washington_monument.vxl`
- Converter kind: `washington`
- Conversion: built the exact tapered four-sided shaft and pyramidion directly
  in metres, then solid-voxelized seven LODs with the historical lower/upper
  marble color break, façade shade, entrance, eight observation windows,
  pyramidion and aluminium apex.

## Bundled-output integrity

The 45 `.vxl` files total 124,929,731 bytes. To verify the set without adding
45 per-file digests to this manifest, hash the sorted SHA-256 listing:

```sh
(cd crates/vearth-gen/assets/landmarks && \
  shasum -a 256 *.vxl | LC_ALL=C sort) | shasum -a 256
```

Expected digest: `61a67ae584c880d0358c4c4cd73cfb251b24c64e67392d3205d5701c769e315c`.

## Island ensembles

`island-surfaces/catalog.json` contains Bled Island's church, freestanding tower,
houses, chapels and stone stairways. Mapped footprints and shoreline, frozen
production terrain, and photographic estimates are recorded in
`tools/heritage_catalog/references/islands/`. Relief reconstructs the island's
missing hill inside its shoreline; source ownership clears architecture and
approaches while preserving wooded banks. The existing lake level is retained.

```sh
python3 tools/build_island_catalog.py
python3 tools/build_island_catalog.py --check
```
