# Architecture recipe contracts

`tools/build-country-architecture.py` validates country, subdivision and named
place recipes and compiles complete static archetypes. Geography chooses one
compatible provider before any finish is resolved. Source footprints, heights,
use and observed roofs constrain admission; mesh and shader consumers cannot
choose another architectural family or repaint the result.

## Optional facade admission

Each weighted construction alternative may declare:

```json
{
  "applicability": {
    "uses": ["Commercial", "Unknown"],
    "min_height_m": 36,
    "max_height_m": 200,
    "min_area_m2": 400,
    "max_area_m2": 5000
  },
  "facade": "StoneOffice"
}
```

Uses are observed source categories. `Commercial` includes offices, hotels and
retail; it does not establish curtain-wall construction. Unknown remains
unknown even when a reviewed district prior chooses a facade representation.
Explicit applicability replaces ordinary-stock admission and requires a nonempty
use list; omitted scale bounds are zero/infinite. Industrial source features
retain their dedicated envelope path.

Without applicability, ordinary stock admits residential/unknown buildings and
small commercial/civic street fabric below 19 m and 900 m². It cannot supply a
housing palette to a large commercial envelope. Ordinary stock also cannot own
a high-rise envelope (36 m classification height); that needs explicit
applicability. Whole buildings use their occupied vertical span; parts use the
source top height, with at least 2.4 m of occupied span. Existing named-place scale
limits still apply. A regional profile with no compatible alternative falls
through to a compatible broader provider, then a generic complete envelope.
The high-rise fallback owns the selected system and its entire wall/glass/frame
palette. It retains the established seeded office/residential family mix for
unknown tall sources without changing their observed use. Explicit residential
use never receives an office system; industrial and roof-only sources are excluded.

The Canary Wharf office recipe combines a verified dock/street polygon with
explicit commercial/unknown use and scale admission. Its boundary excludes
Wood Wharf and surrounding housing. Named-place height limits may extend to
1000 m in the compiler; each recipe must still declare its own researched limit.
Those limits admit source heights and never generate or increase them.

An authored facade requires explicit applicability and a mineral-compatible
form. Available systems are `ClearOffice`, `SilverOffice`, `BronzeOffice`,
`StoneOffice`, `HorizontalOffice`, `Residential`, `RecessedResidential` and
`ConcreteFrame`. Office systems cannot admit observed residential use.
The existing highrise geometry applies from 36 m; shorter, roof-only, industrial
and expressive heritage envelopes retain their ordinary geometry constraints.

The selected system owns its fixed glass palette and facade proportions. The
same archetype owns opaque wall material/pigment and frame/joinery pigment.
Physical glazed construction does not independently imply office use. CPU
resolution records the system once; near panels, coarse integrated surfaces
and distant grammar consume that decision. Panel RGB means opaque pigment;
packed variant 1 means the linear-light wall/glass area mean. These encodings
reuse existing materials and vertex fields.

## Check

```sh
python3 tools/build-country-architecture.py --check
python3 tools/test-country-architecture.py
cargo test -p vearth-gen architecture_resolver --lib
cargo test -p vearth-gen highrise --lib
cargo test -p vearth-app opaque_fragment --lib
```
