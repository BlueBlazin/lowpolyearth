# Regional art direction

## Production ownership

`architecture_resolver.rs` is the sole building appearance resolver. It reads
stable source location, use, height, footprint scale and settlement context,
then selects exactly one compatible provider:

1. A photographed local fabric within its authored influence, for small dwellings.
2. A named district recipe, then a broader city recipe, within their declared scale and geographic influence.
3. An explicit subdivision recipe compiled from the country catalog.
4. The explicit national recipe.
5. A complete high-rise fallback for eligible towers, otherwise generic construction.

Industrial buildings select a dedicated envelope. Other providers must admit the
source use and scale. Ordinary stock admits small shops and civic buildings in
the same street fabric, but excludes large non-domestic envelopes and high-rises.
A tower recipe needs explicit use/scale admission; unscoped country housing
palettes cannot replace high-rise finishes. An authored
facade system requires explicit use and scale bounds; observed residential use
cannot acquire an office system. Known non-domestic buildings without a compatible
recipe use a generic mineral or glazed envelope. At high-rise scale, a seeded
procedural prior selects among the existing facade families and owns their full
palettes. Unknown towers at or above 90 m retain the mixed office/residential
prior, alongside the existing minority glazed prior for urban sources from 60 m;
their source use stays unknown. Explicit recipes take precedence over
this fallback, including explicitly scoped facades without high-rise panels. Source roof shape overrides the selected recipe's
suggestion **before** that provider chooses a roof covering. Providers return a
complete owned design; they do not edit a design produced by another provider.
Geometry consumes that design without consulting country or style palettes again.
`DesignSource` records the selected entry and local fabric for review.

National and subdivision entries use variable-length weighted construction and
finish alternatives. The compiler resolves partial subdivision definitions into
complete immutable archetypes. Each archetype binds its own wall, roof, trim,
joinery and roof-shape choices; runtime selection performs no palette merging.
Source-scale eligibility filters the archetype distribution before selection.
When no alternative is compatible, the explicit masonry infill is checked against
the same admission rules before the resolver proceeds to a broader provider.
The older broad construction-family tables remain in use for landscape and road
classification; their obsolete building palettes, stock mixtures and style selectors have been removed.

Reviewed named places use the same complete archetype representation, compiled
from `place-finishes.json` and bound to a verified identity in `places.json`.
Style profiles supply geometry grammar; the selected archetype supplies all
finishes. Unreviewed named places retain a legacy recipe adapter. A local fabric
study owns its named identity's geographic extent as well as its finishes: the
older broad named envelope cannot reappear outside that study.

Named geography has an explicit district/city tier. Selection checks every
candidate in the spatial bucket, filtering use/scale before evaluating detailed
boundaries. It retains the best compatible, admitted provider without allocating
a candidate list. Districts precede cities; geographic strength and stable
identifiers break ties within each tier. The three-entry inspection API is not
used for production selection. Compiler-generated edge rows bound containment
and inward-fade queries without simplifying polygon holes, edges or disconnected
components. The index has no per-query allocation or mutable runtime cache.

## Evidence and acceptance

Country JSON `review_status`, `weight_basis`, and `visual_references` distinguish
pending work, inventory research, and inspected photographs. A compiled entry
is not visual acceptance. Administrative scope is not parcel-level truth, and
material census categories do not establish visible paint or roof-color shares.
A concrete roof slab can carry ceramic tiles; timber framing can carry plaster.

The resolver selects the high-rise facade system once. Every geometry and shading
path consumes that contract without reclassifying source use or repainting the
wall from a second palette or an aerial roof image. Opaque bands retain the
selected wall pigment; the facade system supplies a separate glass response. Distant whole-wall faces integrate
the actual wall/window area in linear light before encoding their vertex color.
They retain a bounded average glass response for reflections and night emission.

Review actual mapped in-game scenes and controlled distributions separately.
Supplementary finish studies use 36 varied source specimens, plus four close-up specimens;
these are sample counts, not limits on catalog alternatives or measured local
height distributions. Compare coarse color families, material combinations and
prevalence with ordinary housing references. The primary acceptance gallery consists of actual mapped-world screenshots. User approval is the visual gate.

Mapped footprints, holes, parts, height and observed roof shape remain authoritative.
Catalogs infer missing finishes; they do not manufacture missing source buildings.
Climate may modulate finish exposure, never infer neglect from nationality.
Stable source seeds and anchors keep appearance independent of camera and cell cuts.

## Evidence catalog

These sources constrain the original recipes and visual rules. Census dates and
population scopes matter; older construction accounts establish that a technique
exists, not its present-day prevalence. No reference images, models or census
tables are copied into the game by this catalog.

| Area | Evidence and practical implication |
| --- | --- |
| Global methodology | [GEM exposure methodology](https://docs.openquake.org/global_risk_model/exposure.html) separates country/subdivision, settlement, occupancy, construction and height. Its exposure data are research-only here and are not bundled; their license is not the game's asset license. [JRC settlement definitions](https://human-settlement.emergency.copernicus.eu/degurbaDefinitions.php) describe a settlement continuum, not building material or household wealth. |
| Europe | [TABULA national residential typologies](https://episcope.eu/building-typology/) organize real stock by building age and size, supporting modern apartment and older low-rise alternatives. [Historic England on terraced housing](https://heritagecalling.com/2019/11/15/a-brief-introduction-to-terraced-housing/) supports repeated narrow-front masonry rhythms in appropriate stock, not universal detached historic houses. |
| US and Canada | [US Census Survey of Construction](https://www.census.gov/construction/chars/interactive/index.html) distinguishes contemporary exterior wall and building characteristics; new-home statistics are not whole-stock shares. [BC Housing's mid-rise guide](https://www.bchousing.org/publications/Mid-Rise-Best-Practice-Guide.pdf) supports substantial modern timber construction, including multifamily buildings, without implying exposed timber facades. |
| Roof coverings | [US EIA 2020 residential survey](https://www.eia.gov/consumption/residential/data/2020/hc/pdf/HC%202.9.pdf) records composition/asphalt shingles as a substantial existing-stock roof covering; it excludes larger apartment buildings from that question. [Historic England roof guidance](https://historicengland.org.uk/advice/your-home/maintain-repair/roofs/) distinguishes slate, stone and clay coverings and their geographic variation. [Australian Government construction guidance](https://www.yourhome.gov.au/materials/construction-systems) distinguishes tile and sheet roofing. These support independent roof-covering choices; precise local tile color and roof-cover shares remain uncalibrated. |
| Mexico | [INEGI 2020 complementary census results](https://www.inegi.org.mx/contenidos/saladeprensa/boletines/2021/EstSociodemo/ResComplCPV2020_Nal.pdf) report slab/beam-block roofs far more often than tile roofs, with metal sheet also significant. The Mexican recipes therefore retain slab and metal construction in towns and rural areas as well as cities. |
| Brazil and Chile | [IBGE housing material definitions](https://www.ibge.gov.br/apps/snig/v1/notas_metodologicas.html) distinguish covered/uncovered masonry and timber; covered masonry can include several different finishes. [Chile's public housing example in Curauma](https://serviuvalparaiso.minvu.gob.cl/02/05/2025/ministro-montes-entrega-conjunto-habitacional-en-curauma-a-200-familias-de-valparaiso/) grounds contemporary multifamily alternatives, not a national prevalence estimate. |
| India | [Census HL-03 wall/roof tables](https://censusindia.gov.in/nada/index.php/catalog/8147) separate states and rural/urban stock. [India Housing Report's newer housing analysis](https://indiahousingreport.in/outputs/data-tales/deconstructing-the-pucca-house/) supports substantial urban concrete-roof growth and northeast differences. [MoRD/UNDP PAHAL](https://www.undp.org/india/publications/pahal-prakriti-hunar-lokvidya-compendium-rural-housing-typologies) documents local construction/material/climate differences; its selected typologies are not a template for every building. |
| Pakistan and Nepal | [Pakistan census 2023](https://www.pbs.gov.pk/wp-content/uploads/2020/07/National-Census-Report-2023.pdf) distinguishes rural and urban roof construction; girders/T-iron describe structure and must not automatically become visible metal covering. [Nepal 2021 housing report](https://censusnepal.cbs.gov.np/results/files/result-folder/Final_Housing_Household_Report_website_12_2.pdf) records extensive galvanized sheet and concrete roofing with geographic differences. |
| Sri Lanka and Bengal delta | [Sri Lanka census 2024](https://www.statistics.gov.lk/Resource/en/Population/CPH_2024/CPH2024_Final_Eng.pdf) supports brick/block walls and mixed tile/sheet roofs. [Bangladesh Bureau of Statistics housing report](https://objectstorage.ap-dcc-gazipur-1.oraclecloud15.com/n/axvjbnqprylg/b/V2Ministry/o/office-bbs/2024/12/0ea8235233c143468f0c7053ad2cc7de.pdf) distinguishes roof and wall materials, but its combined tin/wood category cannot supply a metal-only visual frequency. |
| China | [National Bureau of Statistics rural housing summary](https://www.stats.gov.cn/xxgk/sjfb/zxfb2020/201712/t20171219_1767886.html) documents brick/concrete and brick/stone/wood dominance and macroregional variation. It describes structural construction; specific roof shapes/colors remain qualitative inference. |
| Japan and Korea | [Japan's Housing and Land Survey](https://www.stat.go.jp/english/data/jyutaku/results.html) separates building type and structure. [EERI's Japanese house account](https://world-housing.net/wp-content/uploads/WHEReports/wh100094.pdf) is an older technique reference. [Korea's 2024 census release](https://www.kostat.go.kr/boardDownload.es?bid=11747&list_no=439064&seq=1) establishes extensive apartment housing and regional variation; apartment unit shares are not tower polygon shares. |
| Southeast Asia | [Philippines census 2020 housing characteristics](https://psa.gov.ph/content/housing-characteristics-philippines-2020-census-population-and-housing) records extensive metal roofing and concrete/brick/stone walls, with most homes needing no or minor repair. [EERI Malaysian construction accounts](https://www.world-housing.net/category/asia/malaysia) include contemporary concrete frames and timber roof trusses. [URA's shophouse guide](https://www.ura.gov.sg/conservation/conservation-resources/understanding-the-shophouse/) concerns heritage stock; [HDB's town design guide](https://www.hdb.gov.sg/-/media/about-us/our-role/plan-and-design-towns/town-design-guides/Sembawang-Town-Design-Guide.pdf) supports restrained primary/secondary/accent color roles on modern blocks. |
| Central Asia | [World Bank multifamily housing study](https://documents1.worldbank.org/curated/en/401931598952181261/pdf/Earthquake-Risk-in-Multifamily-Residential-Buildings-Europe-and-Central-Asia-Region.pdf) and [EERI Kyrgyzstan masonry account](https://world-housing.net/wp-content/uploads/WHEReports/wh100067.pdf) establish masonry/precast/concrete apartments; monumental domes are not a residential prior. |
| West Asia | [Saudi housing survey 2024](https://www.stats.gov.sa/documents/20117/2435273/Housing%2BStatistics%2BBulletin%2B2024%2BEN.pdf/5460b186-50fc-9c5f-112b-0ca5e40fddef) describes strong reinforced-concrete use among Saudi households, not all households. [Iranica's Gilan housing account](https://www.iranicaonline.org/articles/gilan-xii-rural-housing/) grounds Caspian rain roofs and replacement by newer materials. [CRAterre/Shelter Cluster Yemen profile](https://www.sheltercluster.org/promoting-safer-building-working-group/documents/detailed-shelter-response-profile-yemen-local) documents geographic differences and warns that construction practice changes. |
| West and East Africa | [Ghana 2021 housing census](https://statsghana.gov.gh/gssmain/fileUpload/pressrelease/Volume%203K_Housing%20Characteristics_240222.pdf) and [Kenya 2019 housing report](https://www.knbs.or.ke/wp-content/uploads/2024/05/4.2019-Kenya-Population-and-Housing-Census-Analytical-Report-on-Housing-Conditions-and-Amenities-Vol.pdf) establish extensive sheet roofing, including rural stock. These findings rule out a rural-equals-thatch default. |
| Southern Africa, Egypt and Ethiopia | [South African household survey](https://www.statssa.gov.za/?p=17283) supports corrugated-sheet and tile roofs. [UN-Habitat Egypt housing profile](https://unhabitat.org/egypt-housing-profile) and [Ethiopian condominium study](https://unhabitat.org/condominium-housing-in-ethiopia) establish ordinary contemporary masonry/concrete housing. Informal tenure or development does not by itself establish a ruined appearance. |
| Australia and New Zealand | [Australian Government climate guidance](https://www.yourhome.gov.au/passive-design/design-climate) and [shading guidance](https://www.yourhome.gov.au/passive-design/shading) support climatic differences and useful shade but do not measure existing-stock frequencies. [Stats NZ Housing in Aotearoa](https://www.stats.govt.nz/assets/Uploads/Reports/Housing-in-Aotearoa-2020/Download-data/housing-in-aotearoa-2020.pdf) supports mixed timber and masonry cladding. |
| Pacific islands | [Pacific human settlements study](https://s3.eu-north-1.amazonaws.com/cdn.sheltercluster.org/public/docs/human_settlements_development_and_disaster_risks_in_pacific_island_countries.pdf) documents local diversity and modern timber/concrete/sheet construction; it is older qualitative evidence. [Fiji Shelter Cluster resources](https://www.sheltercluster.org/geographic-region/fiji) provide more recent local building guidance. Neither supports a universal ceremonial pavilion. |
| Visual grammar | [National Park Service architectural character guidance](https://www.nps.gov/orgs/1739/upload/preservation-brief-17-architectural-character.pdf) prioritizes shape, roofs, projections, openings and materials. [Insomniac's trim workflow](https://media.gdcvault.com/gdc2015/presentations/Olsen_Morten_TheUltimateTrim.pdf) supports reusable coherent material detail. [Hello Games' Frontiers](https://www.nomanssky.com/frontiers-update/) demonstrates richer buildings assembled from structured part families. These are craft references, not geographic evidence or copied assets. |

## Rebuild and verify

```sh
python3 tools/build-country-architecture.py
python3 tools/build-country-architecture.py --check
cargo test -p vearth-gen --lib architecture
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
git diff --check
```

The [material review checklist](ARCHITECTURE_MATERIAL_REVIEW.md),
[style sourcebook](ARCHITECTURE_STYLE_SOURCEBOOK.md), and
[place-source manifest](../crates/vearth-gen/assets/architecture/places-SOURCES.md)
record review requirements, legacy style sources, coordinates and attribution.
