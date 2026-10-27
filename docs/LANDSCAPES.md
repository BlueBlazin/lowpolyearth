# Regional landscape palettes

`crates/vearth-gen/src/landscape_catalog.rs` is the authoring authority:
35 reusable profiles and 97 geographic rules. Profiles specify forest-floor,
herbaceous and scrub sRGB anchors, a dormant herbaceous anchor, seasonal response,
small substrate offsets and maximum influence. Rules name their scope and select
a profile with a geographic footprint and an observed-climate/elevation gate.
These are illustrative palette priors, not land-cover or lithology maps.

## Selection and ownership

`regional_landscape.rs` compiles a five-degree candidate index into the binary.
Generation evaluates only a bin's candidates at the exact geodetic sample;
it never samples the bin's centre for color. Each elliptical footprint has full
weight inside half its radius and a smooth fade to zero at its outer radius.
Climate/elevation gates also fade continuously. Compatible overlaps share a
normalized influence budget; no rule wins by table order. The residual uses
`terrain_ecology`'s existing cover/climate palette. Unsupported locations,
submerged samples and incompatible climates retain that fallback.

Longitude wraps, and every footprint vanishes before the poles. Selection is
independent of chunks, LOD, cube face and floating origin. Scratch palettes are
resolved once per column/corner and reused for material endpoints. The renderer,
mesh format, atlas, material periods and existing 240/960 m variation are unchanged.
The prior contributes at most 58% of a vegetation anchor, reduced for ortho.
Observed imagery still passes through `imagery_surface`; orbital source pixels
retain their existing exact behavior. Soil/rock/sand offsets are at most a few
sRGB levels and cannot select basalt, sandstone, salt or any other material.
The existing imagery/slope classifier owns those decisions.

Mapped forest, natural grass and scrub each use their own anchor. Cultivated
corners and field surfaces bypass these priors, as do managed ground, roads,
buildings, water, snow, glaciers and wetlands. Regional barren cover can condition
an already inferred substrate, but cannot establish a geological formation.

## Annual appearance

`ColorSource::month` exposes the existing selected imagery month through the
registry. `ClimateAtlas::monthly_surface` reads the same monthly cells as weather
at **each sample's location**. It uses no camera weather and no hemisphere offset.
Terrain decodes only temperature, rainfall and humidity; annual sampling reuses
its spatial coordinates across months. Tests require exact weather-cell values.
Local temperatures naturally reverse thermal seasons between hemispheres;
rainfall cycles can be monsoonal, equatorial or bimodal. Missing calendar authority
retains the reference anchor. There is no new clock, daily invalidation or cache
variant. Existing month switches cancel generation, clear loaded terrain and
bypass the default generated cache; `GEN_VERSION` invalidates older base palettes.

| Response | Appearance cycle and authority |
| --- | --- |
| Temperate | Muted growth/dormancy transition across local monthly 2–15 °C. |
| Cool | Shorter cool-climate cycle across 0–11 °C; boreal forest-floor changes remain weak. |
| Short | Alpine/tundra growth across 1–9 °C; snow remains owned by imagery and renderer. |
| WinterRain | Dry herbaceous color requires low monthly rainfall and warmth; cool wet winters retain green scrub. |
| Rain | Local rainfall across 0.5–3.5 mm/day controls monsoon greening, without a fixed onset month. |
| Savanna | Rainfall across 0.3–2.8 mm/day separates green and straw herbaceous states. |
| Dry | Weak response to sparse rainfall; persistent dryland substrate remains stable. |
| Evergreen | Very weak dry-period response; humid tropical and evergreen woodland ground retains identity. |

These thresholds and RGB values are artistic choices. Monthly imagery already
contains seasonal greening and snow: the seasonal change only moves the authored
anchor, by at most 30% toward its dormant target for grass/scrub and 10.8% for
forest floor, before the regional influence and imagery blend. It adds no second
snow mask, leaf-fall paint or direct seasonal grade over the observed pixel.
Existing local wetness/drought shading, tree responses and crop/flora calendars
remain responsible for their dynamic effects. Evergreen means stable forest
character, not proof that every plant stays green. The one-degree climatology
cannot resolve a hillside's exact phenology or a particular year's drought.

## Evidence and scope

All rule locations use the broad habitat distribution in the
[WWF terrestrial ecoregions map](https://files.worldwildlife.org/wwfcmsprod/files/Publication/file/o0z7qqe0l_Ecoregions_Map.pdf)
as geographic context, with the bundled
[WorldClim monthly normals](https://www.worldclim.org/data/worldclim21.html)
for local thermal/rainfall timing. Input provenance, resolution and licenses are
in `crates/vearth-data/assets/climate-SOURCES.md`. The footprints are authored
approximations, not a reproduction of WWF polygons. The existing
[NASA monthly imagery](https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-map/)
is the observed color authority; NASA describes its temperate green-up,
tropical wet/dry changes and seasonal snow. No RGB here is a measured reflectance.

| Profiles | Purpose and additional evidence |
| --- | --- |
| Atlantic, Heath, Continental | Maritime meadows/heath versus continental woodland; separate forest floor, grass and scrub. |
| Mediterranean | Olive scrub and pale dry herbs in Mediterranean-climate areas, including California and central Chile. |
| Alpine, Tundra | Cool meadows and muted low vegetation; [NPS tundra](https://www.nps.gov/subjects/nnlandmarks/tundra.htm) describes short growing seasons, mosses, lichens and shrubs. Elevation gates separate mountain rules from lowlands. |
| Boreal, Larch | Cool dark forest ground versus lighter continental larch-region understory; no canopy pigment or species replacement. |
| VolcanicHeath | Icelandic low vegetation against evidence-selected dark substrate; [NPS volcanic landforms](https://www.nps.gov/subjects/geology/volcanic-landforms.htm) provides geological context. Geography alone does not paint lava. |
| Prairie, Sagebrush, Steppe | Continental grasslands and muted dry scrub; mapped crops remain crops. |
| Sandstone, Desert | Colorado Plateau warm substrate relationships versus broader xeric landscapes; [NPS Colorado Plateau](https://home.nps.gov/articles/the-colorado-plateau.htm) documents sedimentary geology, but imagery retains local rock identity. |
| MexicanUpland, Puna | Rain-responsive Mexican upland vegetation versus short-season dry Andean/Tibetan highland ground. |
| HumidTropical, CloudForest | Stable humid lowland forest versus cooler humid upland ground; climate and altitude condition applicability. |
| Cerrado, Caatinga, Pampas, Patagonian | Seasonal tropical grass/woodland, dry northeastern Brazilian scrub, temperate grassland and cool dry steppe; [UNESCO Cerrado](https://whc.unesco.org/en/list/1035/) describes wet/dry seasons and plateau habitats. |
| Sahel, Savanna, Miombo, Highland | Dry herbaceous margins, rainfall-driven savannas/woodland and cooler African uplands. Actual local monthly rainfall determines timing, including East African bimodal cycles. |
| Fynbos | Muted fine-leaved Cape scrub; [UNESCO Cape Floral Region](https://whc.unesco.org/en/list/1007) documents Mediterranean-climate shrubland. |
| Monsoon, Ghats, Deccan | Rain-responsive Indian/Indochinese ground, stable humid Ghats forest and drier plateau vegetation; [UNESCO Western Ghats](https://whc.unesco.org/en/list/1342) documents evergreen forest and monsoon influence. |
| EastAsian | Temperate mixed woodland ground in East Asia, distinct from boreal Amur ground. |
| Eucalypt, Mallee, Spinifex | Muted evergreen eastern woodland, winter-rain southwestern scrub and persistent dry interior; [BOM climate zones](https://www.bom.gov.au/resources/learn-and-explore/climate-knowledge-centre/australian-climate-zones) and [rainfall maps](https://www.bom.gov.au/climate/maps/averages/rainfall/?period=an&region=ta) distinguish northern monsoon and southern rainfall regimes. |
| Tussock | Straw/green eastern New Zealand high country; [DOC drylands](https://www.doc.govt.nz/nature/habitats/drylands/) describes tussock habitats. Humid west and alpine rules blend according to climate/elevation. |

Coverage is broad and overlapping, not exhaustive. Small islands, narrow rain
shadows, local lithology and ecosystem boundaries often resolve through the
existing imagery/climate fallback. Climate gates deliberately reduce regional
influence where the approximate footprint crosses an incompatible landscape.

## Add or revise a region

1. Check existing profiles before adding one. Name a material/vegetation
   relationship that the new profile needs, rather than a country tint.
2. Add geographic context and seasonal evidence here. Distinguish observations
   from authored colors, thresholds and footprint extents.
3. Add a rule with a conservative footprint and an appropriate continuous gate.
   Reuse a profile across disconnected regions when the relationship is shared.
4. Run `cargo test -p vearth-gen regional` and `cargo test -p vearth-data`.
   Check candidate-index density, overlap, semantic ownership, seasonal direction
   and dateline continuity. Run repository-wide checks from `AGENTS.md`.
5. Use `docs/PERFORMANCE.md` capture/replay and surface-baseline commands. Preserve
   baseline binaries/images separately. Compare fixed local/vista cameras in
   January, April, July and October and local wet/dry dates; include fields,
   transitions, snow and uncovered locations. Measure cold generation, movement
   and month changes without concurrent builds. Keep results in ignored output.
6. Bump `GEN_VERSION` when baked appearance changes. No material-asset rebake or
   provider request is required for catalog edits.
