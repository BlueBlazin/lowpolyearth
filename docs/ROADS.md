# Roads and paving

Protomaps geometry supplies alignment and functional subtypes. Authored widths
and grading in `road_profile.rs` are shared by road ribbons, source-way bridge
decks, terrain conditioning, ecological clearance and driving guides. Prepared
bridge catalogs retain measured deck dimensions. Ground tunnels stay excluded;
unrecognized transportation families do not become streets.

`road_materials.rs` resolves mapped route function, indexed ground evidence,
a continuous settlement-development cue, annual climate and `road_catalog.rs`. Ordinary woodland
footways remain natural; mapped pavement excludes earth and gravel. Dedicated
cycleways and crossings stay smooth. Rounded cobbles are restricted to suitable
shared routes and documented local treatments. Regional palettes affect walking
and shared routes without repaving the global highway network.

The catalog covers every country and subdivision in the bundled administrative
atlas through an exhaustive geographic partition. Its paving recipes are
independent of building forms, architectural palettes and heritage influence
regions. Unknown geography receives a climate-conditioned contemporary recipe.
The climate cue is annual, so weather and seasons do not change paving choices.
These are authored material priors: Protomaps route function and mapped ground
constrain them, but neither region nor density establishes an individual road's
surveyed material.

Normalized route names supply stable style identity. Merged Protomaps feature
IDs are not treated as individual road identities. Unnamed routes use the
dominant contextual finish rather than fragment-wise random selection. Render
cell, camera distance and feature order never seed material choice. Context is
queried at a direction-stable arc-length midpoint on the source polyline; source clipping, generalization and actual
context boundaries can still change the evidence available to a fragment.

Local treatments require both a small geographic envelope and a matching route
name. A cached spatial index bounds lookup as the catalog grows. Each record
carries its primary source; matching also checks route function. New entries
must name the supported paving treatment and its geographic scope, rather than
infer paving from historic buildings nearby.

| Local treatment | Reference |
| --- | --- |
| Amsterdam canal walking brick | [Municipal public-space policy](https://www.amsterdam.nl/publish/pages/869895/visie_openbare_ruimte_2025.pdf), old-center brick sidewalks. |
| York College Street setts, Stonegate flags and Fossgate setts/flags | [Streetscape guidance](https://www.york.gov.uk/downloads/file/1747/sd109-city-of-york-streetscape-strategy-and-guidance-2014-) distinguishes rounded river cobbles, cut setts and sandstone flags; [Fossgate design](https://democracy.york.gov.uk/mgAi.aspx?ID=34368) distinguishes carriageway and footway. |
| DUMBO Belgian block and Pearl Street Plaza granite | [NYC restoration](https://www.nyc.gov/site/dep/news/25-029/city-completes-108-million-historic-street-restoration-upgrade-dumbo) documents restored street blocks and smooth granite paths. |
| Soweto memorial walking brick | [Johannesburg's red-brick route](https://joburg.org.za/play_/Pages/Play%20in%20Joburg/Culture%20and%20Heritage/Links/Soweto%27s-red-brick-route-to-16-June-1976.aspx) documents the walking treatment. |
| Paraty irregular stone streets | [IBGE geographic account](https://agenciadenoticias.ibge.gov.br/agencia-noticias/2012-agencia-de-noticias/noticias/23992-nomes-geograficos-identificam-lugares-e-ajudam-a-contar-suas-historias) identifies the irregular stone street paving; the override excludes separate footways and cycleways. |
| Melbourne Hardware Lane bluestone | [Municipal renewal](https://participate.melbourne.vic.gov.au/hardware-lane/streetscape-renewal) replaces the former red brick with bluestone. |
| Kyoto Hanamikoji, Miyagawacho and Tatsumi stone | [Municipal works program](https://www.city.kyoto.lg.jp/gyozai/cmsfiles/contents/0000172/172480/zenntai.pdf) identifies stone-paving repairs on these routes. |

Regional recipes extrapolate conservatively from material families rather than
claim measured installation shares. Useful design references include [CROW cycle
surfaces](https://kennisbank.crow.nl/public/gastgebruiker/WOBU/Ontwerpwijzer_fietsverkeer/Keuze_van_de_verhardingssoort/33008),
[TfL concrete and natural stone](https://content.tfl.gov.uk/streetscape-guidance-2022-revision-2.pdf),
[India's walking-surface guidance](https://www.mohua.gov.in/upload/uploadfiles/files/NMTGuidanceFINAL.pdf),
[South African pavement guidance](https://www.hssonline.gov.za/assets/docs/SECTION%20I_TRANSPORTATION%20AND%20ROAD%20PAVEMENTS.pdf)
and [Hong Kong's concrete block standard](https://www.hyd.gov.hk/en/technical_references/standard_drawings/hyd_standard_drawings/doc/section_5/pdf/h5126c.pdf).
The rectangular unit texture also serves gray concrete blocks; its material ID
does not imply fired clay.

`atlas.rs` generates asphalt, concrete, rectangular brick/block paving, cut stone
setts, compacted gravel, earth, rounded cobbles and stone slabs. Muted albedo,
matte roughness and metric relief retain the engine's lighting response. Mips
remove unresolved relief; fixed world phase keeps displaced paving joints from
crossfading across material variation. Individual stones add no geometry,
collision obstacles, texture downloads or per-stone draw calls.

Inspect with the production renderer and validate the coverage/role invariants:

```sh
cargo run --release -p vearth-app --bin vearth-app -- \
  --stage roads:materials --screenshot output/road-materials.png
cargo test -p vearth-data --all-features --lib road
cargo test -p vearth-gen --all-features --lib road
cargo test -p vearth-app --all-features --lib atlas
```

Use the pipeline and replay checks in [PERFORMANCE.md](PERFORMANCE.md) for
matched generation, mesh, cache and frame-time comparisons. Optional art must
not remove mandatory road geometry or bypass source-completeness checks.
`surface-baseline --roads` supplies dense walking/cycling probes; add
`--roads-without-context` for identical-width baseline materials. Geometry counts
should remain bounded while material fingerprints intentionally differ.
