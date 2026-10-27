# Architecture material acceptance

Country coverage, subregional accuracy and material quality are separate gates.
The source catalog and automated checks below are the authority for coverage;
counts alone do not establish fidelity to every street or building.

## Required coverage

| Gate | Checkable minimum |
| --- | --- |
| Countries and territories | Every exact country name in the bundled administrative atlas has an explicit specification in `crates/vearth-gen/assets/architecture/countries/`. No unnamed shared-family fallback. |
| Country specification | Named construction identity; urban, town and rural alternatives; variable-length weighted form and finish alternatives, with material-aware wall and roof roles; inspectable source URLs, evidence scope and limitations. |
| Shared specifications | Identical complete specifications require named counterparts and a material/construction justification. Similarity in climate or proximity alone is insufficient. |
| Subregions | Explicit, evidenced subdivision entries where national stock differs. Exact names resolve against the source atlas; one complete regional entry is selected instead of the national entry. |
| Local character | Re-audit the source-identified place atlas and photographed local finish studies. Distinguish published district boundaries from authored influence extents, and preserve their source provenance. |
| Material roles | Painted timber retains its local paint color and timber texture. Brick, exposed stone, render, sheet metal, slate, clay tile and flat membranes do not share one generic brown/gray treatment. |
| Source authority | Footprints, height, holes, roof shape, building use and landmark assets remain authoritative. No country recipe creates a building where no mapped footprint exists. |

Country pigments and relative weighted mixtures are artistic interpretations of the
cited evidence, not surveyed colors or census percentages. Heritage evidence
establishes compatible local alternatives, not a national claim that all homes
are historic. Uninhabited territories retain explicitly scoped compatibility
specifications for mapped infrastructure, not invented residential traditions.

## Resolution contract

`architecture_resolver.rs` selects one provider: compatible local fabric, named
district, broader city, explicit subdivision, national entry, or neutral missing-data fallback.
Industrial sources select dedicated construction. Eligible towers without an
explicitly applicable recipe select a complete high-rise fallback; ordinary
housing recipes cannot replace their facade palettes. Source roof shape is
resolved before the provider chooses its finish. No provider repaints another
provider's design, and broad family palettes are outside this production path.
The result records its selected `DesignSource` and archetype for inspection.
The compiler expands shared source defaults and per-archetype alternatives into
complete immutable building recipes. Each recipe binds compatible walls and
roof coverings; no runtime cross-product with a country-wide material list.
Shared static arrays avoid duplicating identical finish data.

Districts precede city fallbacks regardless of their relative geographic weights.
City fallbacks exclude rural and village settlement classes; a compatible named
district may still describe those settlements.
Production selection checks all indexed candidates for source compatibility
and geographic admission before ranking; the bounded three-entry list is for
inspection only. Narrow streets can use sourced corridor envelopes. Circles and
corridors are authored influence extents, not administrative boundaries. A
pinned district polygon preserves holes and disconnected parts, with an inward
transition that never applies the recipe outside the published boundary.

Custom landmark models retain their authored authority outside this pipeline.
Lighting, physical texture sampling and exposure render the selected finishes;
camera motion and cell clipping never reselect them.

## Material acceptance

Construction art includes sixteen base finishes and ten independently needed physical variants: corrugated metal sheets, plain tile, rubble, painted vertical boards, wood shingles, woven panels, corrugated cement sheets, turf, plain glazed ceramic wall tiles and patterned glazed ceramic wall tiles. Matte corrugated sheet and tile-stamped metal share existing profiles with distinct surface responses. Material count is an implementation result, not an acceptance target. See the [material authoring guide](../crates/vearth-app/assets/materials/README.md)
for the channel contract and primary art-direction references.

- The current atlas uses 128×128 linear RGBA tiles, 55 layers and eight mip levels,
  occupying 4,805,900 upload bytes. These are the current inventory and cost,
  not a cap on researched material coverage. Additional finishes need a visible
  material distinction and a measured memory/sampling budget.
- Texture variants share the existing atlas and sampling path. They add no shader
  samples, draw passes, per-building material instances or geometric detail.
- Make material character visible through broad pigment variation, unit-scale
  structure and restrained fine grain. Avoid uniformly noisy plaster, identical
  bricks, embossed black joints, baked lighting and photographic grunge overlays.
- Preserve distant mean color. Wrapped derivatives must remain integrable;
  unresolved relief contributes to roughness instead of shimmer.
- Review pale plaster, deep painted timber, oxide clay and dark/slate roofs under
  more than one sun angle and at near and far camera distances.

## Reproduce and review

```sh
python3 tools/build-country-architecture.py --check
python3 tools/test-country-architecture.py
python3 tools/preview-country-architecture.py
python3 tools/build-architecture-places.py --check
python3 tools/convert-westminster-architecture-boundaries.py --check
python3 tools/convert-osm-architecture-boundaries.py --check
python3 tools/convert-amsterdam-architecture-boundaries.py --check
python3 tools/convert-lisbon-architecture-boundaries.py --check
python3 tools/check-authored-architecture-districts.py --check
python3 tools/bake-construction-materials.py --check --preview output/construction-materials
python3 tools/build_island_catalog.py --check
cargo test -p vearth-gen --lib architecture
cargo test -p vearth-gen --lib regional_finishes
cargo test -p vearth-app --all-features --lib atlas
cargo test -p vearth-app --all-features --lib staging
```

The preview command writes `output/architecture-review/country-catalog.html`,
an offline searchable weighted palette reference with source notes and review status. It is not a
lighting simulation; use the mapped-world gallery for rendered acceptance.

Visual acceptance requires a large gallery of actual mapped-world screenshots and user approval. Staged building grids are supplementary diagnostics, not the requested acceptance gallery. Tests guard source constraints and rendering contracts; they cannot establish resemblance. The staging catalog uses the production renderer and live resolver; `buildings:finish:<key>` shows 36 varied source specimens, and `buildings:finish-detail:<key>` shows four close-ups. These counts never constrain catalog choices. Material studies hold geometry and physical scale fixed:

```sh
cargo run --release -p vearth-app -- --stage materials:plaster
cargo run --release -p vearth-app -- --stage materials:tile
cargo run --release -p vearth-app -- --stage buildings:finish:bled
cargo run --release -p vearth-app -- --stage landmark:bled-island
```

Other material IDs are `brick`, `stone`, `timber`, `slate`, `panel`, `metal`,
`siding`, `shingle`, `flat`, `thatch`, `namako`, `limestone`, `marble`, and `earth`.
Use `--screenshot output/view.png --size 1400x1000 --stage-sun-hour 15` for a
repeatable frame, and `--stage-distance` for near/far comparisons. Local finish
IDs come from `regional_finishes::reviewed_finishes()`.

Run the repository-wide format, check, test, documentation and diff checks
before accepting changes. Changes to generated building materials or palette
selection require a `GEN_VERSION` bump; atlas-only art changes do not change
source geometry. Keep render captures and timing reports in ignored `output/`.

## Mapped-world gallery

```sh
python3 tools/render-world-architecture-review.py \
  --binary target/release/vearth-app \
  --cameras tools/architecture-review-cameras.json \
  --output output/architecture-world-review
```

The camera list covers ordinary housing and surrounding context at researched
locations. Inspect each image for useful framing and source coverage; adjust a
camera when needed. The script records the executable hash, camera arguments,
streaming report and image paths. It refuses to mix executable revisions in an
existing capture directory. Staged specimens and material boards supplement
these world views, particularly when checking texture behavior close up.
