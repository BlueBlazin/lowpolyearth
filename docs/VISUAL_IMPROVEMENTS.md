# Environment visuals

This document records the current environment representation, its visual
invariants and its remaining limits. Source code, production fixtures and
captures determine whether those invariants hold.

## Scope and constraints

- Preserve mapped geography, building footprints, land use and habitat evidence.
  Inferred decoration must not claim unsupported geographic information.
- Generate world surfaces directly from source data. Shared terrain samples,
  semantic masks and spatial addresses may remain grids; visible structures,
  collision and persistent caches must not reconstruct occupied voxel columns.
- Apply ordinary-world behavior globally. Prepared catalogs can enrich known
  features, but do not gate continuous terrain, roads or bridge geometry.
- Keep tree geometry static. Wind-driven trunk, branch, crown and leaf
  deformation is excluded. Vehicles are outside this environment document.
- Check movement, cell boundaries and LOD transitions as well as still images.
  Optional detail must remain bounded and must not remove mandatory geometry.

## ENV-01 — Buildings, roads and bridges

Buildings retain their source polygon outlines, holes, parts and orientation.
Full-feature ground datums align foundations, eaves, roofs and storeys across
cells. Measured surface data supplies absolute roof authority where accepted.
Regional facades retain resolvable glazing and night windows in the base mesh.
Minute panes, relief and plain parapet recesses follow sampling resolution at
every detail level, preserving source outlines and shaped regional rooflines.
Elevated mapped parts preserve real parent bodies without inventing support
beneath standalone parts.

Roads use metric widths, connected ribbons and the graded terrain surface.
[Road profiles and paving](ROADS.md) retain functional proportions, lighter
trail grading and inferred regional finishes.
Cataloged bridges use mapped alignments and structural detail. Uncataloged bridge
ways also produce continuous decks and approaches with shared clearance; no
voxel fallback is available. Landmark replacement follows source-class rules
and retains neighboring source geometry.

Check source coverage through dense-budget retries, courtyard and underpass
collision, sloping foundations, arbitrary orientations and adjacent cells.
Incomplete required-source output must never become a complete cache entry.

Entry points: [building envelopes](../crates/vearth-gen/src/building_surface.rs),
[road surfaces](../crates/vearth-gen/src/road_surface.rs),
[bridge surfaces](../crates/vearth-gen/src/bridge_surface.rs),
[bridge source ways](../crates/vearth-gen/src/bridge_fallback.rs),
[landmark geometry](../crates/vearth-gen/src/landmarks/surface.rs).

## ENV-02 — Terrain shading and material continuity

Shared source elevations drive direct terrain triangles and their collision.
Corner normals and appearance blend rolling ground while retaining supported
cliffs, rock breaks and mapped boundaries. Mapped cover and observed climate
condition meadow, heath, scrub and woodland palettes without claiming finer
ecological or geological knowledge. The global elevation provider
interpolates across source tile edges.

Check shared borders independently of art changes. Repeated grid patterns,
lighting seams and stepped tile edges are defects; limited source resolution
cannot be repaired by merely adding triangles. Terrain skirts close mixed-LOD
rendering seams without becoming physical walls.

Terrain pigment bakes structure finer than the imagery into corner tints:
moisture patches and equator-facing dryness on vegetation, soil and sand
patches, fall-line rills and cliff runnels, and folded altitude bedding on
rock. Bedding strength follows rock chroma (warm sedimentary colours layer,
neutral granite-like rock does not). It is one global heuristic of the
corner's family, normal, altitude and ECEF position, with no place rules or
new sources. Cultivated fields keep their tint and take a softer per-corner
ratio. Octaves fade below the corner spacing, and altitude bands below the
vertical corner step, so coarse cells keep the source colour. It adds no
render cost; compare against a build without it using `--fresh-chunks`.

Entry points: [terrain surfaces](../crates/vearth-gen/src/terrain_surface.rs),
[source appearance](../crates/vearth-gen/src/terrain_appearance.rs),
[terrain pigment](../crates/vearth-gen/src/terrain_pigment.rs),
[terrain materials](../crates/vearth-app/src/terrain_material.wgsl),
[global elevation](../crates/vearth-data/src/terrarium.rs).

## ENV-03 — Cloud shadows

Low clouds cast filtered sunlight patches onto terrain, structures, vegetation
and water using the visible cloud field, wind and extinction. A 128-square
light-space map covers 48 km and stores cumulative transmission at four cloud
heights; receivers above the cloud top remain sunlit. Twelve density probes per
texel refresh the map each active frame, including accelerated weather time.
Map edges, low Sun and orbital views fade to the existing weather lighting.
Sky fill and terrain shadow geometry remain independent. Mid/high cloud layers
and atmospheric light shafts are not resolved by this map.

Set `VEARTH_DISABLE_CLOUD_SHADOWS=1` for a comparison with the existing lighting.
Generation skips disabled clouds, zero low-cloud coverage, night and orbit.
Its GPU cost is included in the world timing interval. Check identical capture
inputs with the switch on and off, plus cloud-base flight and moving weather.

Entry points: [cloud sunlight map](../crates/vearth-app/src/cloud_shadows.rs),
[cloud atmosphere](../crates/vearth-app/src/atmosphere.wgsl),
[world lighting](../crates/vearth-app/src/shader.wgsl),
[flora lighting](../crates/vearth-app/src/flora.wgsl).

## ENV-04 — Water, shorelines and clouds

Typed water triangles retain mapped shorelines, holes and admitted river/canal
widths. Ocean surfaces follow ellipsoid sea level. Inland water uses a shared,
continuous DEM-derived datum; its level and optical depth are inferred. Ground
must meet water without burying small ponds or narrow channels inside an
unchanged terrain quad. Complete empty inland vector coverage keeps below-sea
land dry. Contact queries must follow the visible water as cells refine.

Water uses filtered, continuous waves, restrained material-atlas gradients,
Fresnel and sunlit crest scattering. Source classes shelter lakes and channels
from ocean-scale roughness. Optical depth and climate palette weights transition
continuously across their classification thresholds. Linear-light spectral
attenuation gives clear shallows mint/turquoise hues and deeper water blue
tones. Geographic sand/rock appearance varies at fixed physical scales, fades
with optical depth, and filters out before coarse mesh sampling. Broad colours
are baked into vertex tint; a bounded shader field adds stationary benthic
detail to visible ocean shallows and fades with pixel footprint. Natural bank faces retain
their substrate hue with a narrow wet band around the waterline; pavement,
fields, snow and ice remain unchanged.

The [distinctive-water catalog](../crates/vearth-gen/assets/water/README.md) supplies named lakes
with their own shallow and optically thick palettes, extinction and filtered
pigment variation. Its geographic masks only color existing inland water.
Check adjacent lakes and ocean, mask edges, and both close and aerial LODs;
catalog colors must not create wet surfaces or alter contact depth. References
and temporal qualifications live with each catalog entry.

Low cloud bases sit at a condensation level above the regional ground (bundled
terrain averaged over about 40 km), so valleys stay clear while peaks can pierce
the layer. Visible low clouds use level bases, per-cell rounded crowns,
mesoscale clustering, coverage calibrated to the shape texture's measured
deciles, and edge detail that displaces the surface (wisps under the base,
lobes above), with transmittance-dependent phase lighting. Sunlight inside
clouds combines a near self-shadow probe with the cached sunlight columns, which
also dim the sky light reaching cloud undersides. The weather's cloud optical
depth, which includes rain, raises water content, so raining decks turn grey
through extinction rather than a tint.
Density and temporal reprojection share the same ECEF wind. Zero low-cloud
coverage produces no low-cloud volume. Existing half-resolution temporal history,
interleaved updates and bounded ray counts remain in use. Stable, valid history
averages stochastic samples; cumulative weather and lighting changes invalidate
it. Rays cover the encountered layer within their fixed step ceiling. Distant
clouds use filtered mips of the same shape texture and atmospheric depth cues.
Local compositing uses scene and cloud depth at every camera height, retaining
clouds in front of land and water while rejecting those hidden behind it. The
64-square sky reflection cache holds the sky only: broken cloud lightens it and
raining decks darken it through coverage and optical depth, without reflected
cloud bodies. Water and glazing sample one environment lookup per pixel.

Water remains opaque and depth tinted. Foam and its shoreline-processing cost
are deliberately excluded. Scene-object reflections, refraction,
measured navigation depth and directional river flow are not implemented. Do not
infer waterfalls, beaches or downstream direction from unsupported map data.
Check adjoining source tiles and LODs, calm and windy water, cloud visibility,
day/night transitions, and motion through cloud base.

Coastal elevation reconciliation remains unresolved: mapped land can have
negative DEM heights, and mapped shoreline can cross elevated DEM terrain,
producing holes, steps or tall banks. Reproduce on the small island east of
(-16.507, -151.781) at Bora Bora and the coast near (-4.372, 55.828) at La Digue.
A correction must distinguish explicit beaches from sand/dunes, retain real
cliffs and dry below-sea land, and share one source authority and elevation
sampler across terrain, vegetation, structures and neighboring LODs. Local
flattening or terrain-only displacement does not satisfy that contract.

Technique references: [Rare's water art](https://history.siggraph.org/wp-content/uploads/2022/09/2018-Talks-Ang_The-Technical-Art-of-Sea-of-Thieves.pdf),
[Epic's absorption and scattering model](https://dev.epicgames.com/documentation/en-us/unreal-engine/single-layer-water-shading-model-in-unreal-engine),
[GPU Gems wave derivatives](https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models),
[Guerrilla's cloud shaping](https://www.guerrilla-games.com/read/nubis-authoring-real-time-volumetric-cloudscapes-with-the-decima-engine),
and [Sucker Punch's atmosphere lighting](https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html).

Entry points: [water geometry](../crates/vearth-gen/src/water_surface.rs),
[water shading](../crates/vearth-app/src/water.wgsl),
[shared cloud field](../crates/vearth-app/src/cloud_common.wgsl),
[cloud atmosphere](../crates/vearth-app/src/atmosphere.wgsl),
[reflection environment](../crates/vearth-app/src/sky_environment.rs).

## ENV-05 — Vegetation and contextual ambience

Source-supported tree roots own direct mesh LODs and do not generate collision
geometry. Source and landmark exclusions retain their authority over tree
placement. Flora descriptors retain mapped habitat,
field identity and seasonal context. Optional detail must keep unsuitable
places and quiet vistas clear.

Further close blade, leaf and flower modeling, habitat-specific birds or
insects, local mist and optional grass/crop/flower motion remain separate work.
Tree geometry animation stays excluded.

Entry points: [tree surfaces](../crates/vearth-gen/src/tree_render.rs),
[tree placement](../crates/vearth-gen/src/tier0.rs),
[flora generation](../crates/vearth-gen/src/flora.rs),
[flora rendering](../crates/vearth-app/src/flora.wgsl),
[weather](../crates/vearth-app/src/weather.rs).

## ENV-06 — Regional art direction

Ordinary buildings use a global administrative construction catalog, meaningful
subdivision overrides and the bundled 2020 GHS-SMOD settlement classification.
Reported use and roof shape take precedence; the same source-geographic anchor
selects materials and detail across visiting cells. Contemporary housing,
commercial and industrial construction remain distinct from historic examples.

Visual grammars add coherent window/entrance shapes, shutters, projecting bays,
galleries, ground-floor frontage and roof silhouettes. A source-identified place
atlas strengthens researched local character through smooth authored influence
envelopes, with contemporary infill and source-scale limits. Finishes can retain
local identity when ornate domestic geometry is inappropriate. See the
[style sourcebook](ARCHITECTURE_STYLE_SOURCEBOOK.md) for ingredients and evidence.

Source envelopes receive fitted entrances, surrounds, shading, selected balcony
stacks, eaves and coping. Unmeasured flat roofs have recessed decks and shallow
parapets within their reported height. Construction materials include masonry,
plaster, cladding, siding, shingles and flat/pitched coverings with filtered
metric textures. Finish
variation is per building with a soft climate influence, independent of country
wealth. Vegetation and terrain keep their separate observed habitat/climate
rules. See the [coverage and validation checklist](REGIONAL_ART_DIRECTION.md)
for research, construction families and the limits of these inferred choices.

Landmark source shapes, calibration, materials and attribution remain part of
the art contract. Runtime uses indexed original-source meshes; each exported
asset carries source and output hashes. The permanent Merlion preservation bake
is recorded explicitly in its source manifest.

Entry points: [art direction](../crates/vearth-gen/src/art_direction.rs),
[regional construction](../crates/vearth-gen/src/regional_architecture.rs),
[global catalog](../crates/vearth-gen/src/architecture_catalog.rs),
[visual grammars](../crates/vearth-gen/src/architecture_styles.rs),
[local influences](../crates/vearth-gen/src/architecture_places.rs),
[regional vegetation](../crates/vearth-gen/src/vegetation.rs),
[landmark sources](../crates/vearth-gen/assets/landmarks/SOURCES.md).

## ENV-07 — Lighting and image stability

Direct daylight provides the bright surface key; fitted sky fill retains its
cooler colour at the same diffuse luminance. Natural grass pigment follows
growth, moisture and source blend weights, preserving source variation and
separate crop lifecycle colours. Highlight compression blends toward a shared
RGB gain to retain pigment colour; neutral exposure and the dark toe stay
unchanged. These responses use the existing surface and image-resolve passes.

Building emission remains on visible facade geometry. Optional trim admission
must not extinguish resolvable base windows, and coarse source buildings must not replace
lights with detached points. Spatial resolve and restrained contact shading
share the current scene depth.

Illuminating neighboring surfaces from entrances and windows, waterfront light
reflections, and temporal reconstruction for thin detail remain separate work.
Check motion for flicker, blur, ghosting and geometry changes at LOD handoffs.

Entry points: [world lighting and emission](../crates/vearth-app/src/shader.wgsl),
[image resolve](../crates/vearth-app/src/tonemap.wgsl),
[render pipeline](../crates/vearth-app/src/gfx.rs).

## Research references

These provide transferable principles, not exact implementations to copy or
claims that their full feature sets are absent here.

- [Nintendo: Tears of the Kingdom developer interview, Part 3](https://www.nintendo.com/en-ca/whatsnew/ask-the-developer-vol-9-the-legend-of-zelda-tears-of-the-kingdom-part-3/)
  — regional identity and keeping views readable.
- [Hello Games: Worlds Part I](https://www.nomanssky.com/worlds-part-i-update/)
  — water responding to weather and depth, and coherent environmental effects.
- [Sucker Punch: Ghost of Tsushima visual effects](https://blog.playstation.com/?p=345372)
  — habitat-specific ambience and shared environmental motion; tree sway remains
  excluded from this engine's checklist.
- [Sucker Punch: Real-Time Samurai Cinema](https://www.advances.realtimerendering.com/s2021/jpatry_advances2021/index.html)
  — coordinated materials, atmosphere, lighting and tonal control.
