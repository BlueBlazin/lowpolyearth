# Observed local building finishes

`regional_finishes::reviewed_finishes()` is the production inventory. Each local
study contains a variable number of weighted, linked wall/roof fabrics. A roof
choice carries its own pigment, material and surface variant; wall and joinery
choices use independent salted draws. There is no fixed palette-size quota.

The photographs below were visually inspected, including ordinary occupied
buildings around landmarks. They establish observed alternatives, not population
statistics. Relative weights are coarse authored prevalence judgments; one
photograph is not a housing census. Colors are stylized sRGB choices rather than
measured reflectance. Weather, white balance, age and facade orientation affect
photographs. Spatial ellipses are conservative art-direction scopes, not legal
heritage boundaries. Material ambiguity is called out instead of turning every
red roof into clay or every gray roof into slate.

The central architecture resolver selects one eligible catalog recipe. A local
`LocalRecipe` owns the complete sampled fabric and has no country, family or place
fallback mutation. Its roof suggestion is passed through the central source/use/
scale constraints once, then `resolve(final_roof, state)` constructs the matching
surfaces. Eligibility belongs to the central resolver; local code does not inspect
an existing design or independently gate heights and uses.

Fabric-specific restrained expression is authored for Bled ordinary plaster and
gallery houses, Okinawan housing and Bhaktapur infill. Other local finish studies
use restrained contemporary expression rather than inheriting unreviewed place
ornament. Missing roof suggestions conservatively default to gabled; unknown flat
coverings use neutral `FLAT_ROOF`, while unsupported source-pitched coverings use
neutral generic `ROOF`. These are explicit unknowns, not borrowed national colors.
Named landmarks use their own assets. `Building` does not contain observed color
or covering fields; sampled colors and materials remain priors.

Materials use existing semantic IDs and explicit shader variants. Corrugated
metal, flat ceramic and rubble variants share the same packed-vertex and texture
sampling path. No local profile adds geometry or texture samples. Original
photographs remain research references with their publishers’ rights; no source
photograph is embedded in the game’s texture assets.

## Named-place ownership

`regional_finishes::owns_place(source_id, style_key)` matches exact named catalog
identities, independently of a particular building's local admission. The resolver
must skip those older envelopes even outside the narrower study. A shared style
key by itself never suppresses another town's source record.

`NAMED_PLACE_OWNERSHIP` explicitly audits all 30 local studies. Bled, Keswick,
Penrith, Albi, Visby, Okinawa/Kitanakagusuku, San Diego, Bermuda and Feilding have
no matching named catalog identity in the current resolved place inventory.
The other 21 studies own their exact named entry. Okinawa's regional study does
not own a distant Okinawan town merely because it shares a prefecture. Atlas
Kasbah at Ait Ben Haddou does not own Taourirt's separate source identity.

## Narrow historic study extents

A local study covers the photographed fabric, not the entire municipality or
UNESCO buffer zone. The following ellipses are conservative sampling extents;
they are not claimed as cadastral boundaries.

- **Ait Ben Haddou:** centered on the [UNESCO property coordinate](https://whc.unesco.org/en/list/444/maps/)
  31.04722, -7.12889, with approximately 111 m north/south and 114 m east/west
  radii. UNESCO records a 3.03 ha property. This confines the earthen recipe to
  the ksar vicinity and excludes the new settlement southwest across the river.
  The old-town capture should target this point; a comparison at 31.0426,
  -7.1328 must not select this local recipe.
- **Hoi An:** the 30 ha [UNESCO property](https://whc.unesco.org/en/list/948/maps/)
  differs from its 280 ha buffer. The local study is centered at 15.8774,
  108.3285, near the [mapped ancient-town center](https://mapcarta.com/de/N4601279489)
  and the Tran Phu / Nguyen Thai Hoc merchant streets described by the
  [ACCU conservation report](https://www.nara.accu.or.jp/img/report/report2019.pdf).
  Approximate radii are 145 m north/south and 482 m east/west. Modern housing at
  15.886, 108.338 is outside this finish study.
- **Luang Prabang:** the photograph looks along the historic peninsula; the
  study samples its central street fabric around 19.8937, 102.1402, with
  approximately 267 m north/south and 209 m east/west radii. The nearby
  [documented Sakkarine Road lodging](https://www.travelfish.org/accommodation_profile/laos/northern_laos/luang_prabang/luang_prabang/all/1291)
  provides a street-coordinate anchor. This is a small subset of the wider
  [UNESCO property](https://whc.unesco.org/en/list/479/maps/). Both rendered and
  timber fabrics have explicit gabled and hipped priors; their 3:2 ratio is a
  qualitative interpretation of the roofscape, not measured stock statistics.

## Visual evidence and scope

| Study | Inspected reference | Observed stock and limitations |
| --- | --- | --- |
| bled | [Photographic source](https://www.e-bled.si/wp-content/uploads/2025/06/Stavbna-tipologija-Bled_elaborat_koncna.pdf)  | User town photograph: neutral/dark gray pitched roofs dominate, with conspicuous orange minority; predominantly white/off-white plaster houses, pale yellow minority, dark timber accents. Survey typology pp.11–12 corroborates post-1945 clay, plaster and dark timber. Gray coverings are not distinguishable individually: flat ceramic is a conservative visual approximation, not a census of roof products. Gallery expression is minor, not the default house. |
| keswick | [Photographic source](https://lakedistrict.gov.uk/planning/conservation/conservationareas/a-z-conservation-areas/keswickca/)  | National Park photograph of Victorian terraces: gray-green coursed/rubble stone, white sash frames and red stone surrounds; neighboring cream/white rendered terraces; dark slate coverings. Red ridge pieces do not imply whole orange roofs. |
| penrith | [Photographic source](https://www.westmorlandandfurness.gov.uk/sites/default/files/2026-03/Penrith%20Conservation%20Area%20CAMP%20Final%20Feb%202026%20accessible.pdf)  | Council appraisal photographs p.17: red/buff stone plus limewash and cream roughcast in occupied yards, blue and white joinery; dark slate and gray-brown stone/tile roofs. Material inventory distinguishes red sandstone, green Westmorland/Burlington and Welsh slate; yard photographs under-sample street-facing stone. |
| bath | [Photographic source](https://app.bathnes.gov.uk/docs/temp/Planning-Policy/Placemaking-Plan/city_centre_web.pdf)  | Council appraisal photographs p.28 show ashlar residential upper floors, pale honey/cream and weathered gray stone, white windows and dark gray roofs; materials discussion p.34 also records rendered rubble, clay and later concrete roof tiles. This is observed building fabric in a draft appraisal, not assumed future compliance. |
| albi | [Photographic source](https://www.albi-tourisme.fr/en/location/cathedral-apartment-amber/)  | Gîtes de France roofscape photograph published by Albi Tourism: adjacent residential long brick and gray-beige lime render; aged gray-brown clay tiles alongside new orange tiles; pale blue, sage and white shutters. Cathedral material is not generalized to every house. |
| porto | [Photographic source](https://www.ontheluce.com/getting-lost-in-portos-old-town/)  | Firsthand Ribeira roofscape: terracotta with darkened seams, pale render, blue ceramic-faced houses, yellow and red facades, weathered exposed granite. Uniform muted cream would erase observed color. Tiled facades use smooth painted masonry as a distant-view approximation; no invented azulejo pattern. |
| visby | [Photographic source](https://www.inkysquare.com/blog/2022/3/9/visby)  | Photographer street study: pink, ochre and cream roughcast with red/brown clay roofs, brown/white joinery; one timber roof is visible but is insufficient evidence for a town-wide shingle share. Painted timber alternatives retain separate material compatibility. |
| kyoto | [Photographic source](https://iyeya.jp/properties/1968/)  | Local property aerial near the Philosopher’s Path: dark and silver-gray ceramic roofs mix with standing-seam metal in gray and brown; pale rendered and darker boarded houses. Bright blue patches are construction tarps, not evidence for blue permanent roofs. |
| okinawa | [Photographic source](https://www.okinawa-realestate.com/adaniya/) [Saeki field photographs](https://kazusaeki.amebaownd.com/posts/14260705/) | Kitanakagusuku property aerial shows ordinary flat pale/gray concrete houses dominant, with red and gray pitched roofs. Saeki’s photographed southern Okinawan field series additionally shows both dark tile and red tile on ordinary low houses; these regional examples do not imply every Kitanakagusuku roof is traditional. |
| bhaktapur | [Photographic source](https://www.nepalviews.com/story/402909)  | Photographic aerial report of Durbar Square and surrounding occupied houses: exposed brick dominates street fabric, but many houses have flat concrete terraces and red/silver/blue sheet additions; traditional clay remains conspicuous. Pagoda temple roofs are excluded from the ordinary-house form prior. |
| hoi-an | [Photographic source](https://vinpearl.com/vi/kien-truc-pho-co-hoi-an-net-dep-doc-dao-luu-giu-tram-nam)  | Illustrated old-town street: mustard/ochre frontages, pale lime and gray upper party walls; weathered dark brown-gray clay tiles with a smaller reddish component, dark timber joinery. Gray weathered tile is still ceramic, not generic slate. |
| luang-prabang | [Photographic source](https://siamrat.blog/2023/06/24/modern-lao-history-through-three-luang-prabang-hotels/)  | Firsthand peninsula aerial: many white/cream rendered houses as well as timber; ordinary roofs often dark gray-brown aged ceramic and some reddish clay. Temple gold/orange roofs are not the domestic default. |
| singapore | [Photographic source](https://www.studiowattokeefe.com/project/singapore-shophouse)  | Architect’s Chinatown district aerial: dense red/orange ceramic shophouse roofs, white party walls, white/pale blue/lilac/yellow frontages and timber shutters; modern flat additions retained. Study moved to Chinatown coordinates instead of applying a Chinatown photograph to Katong. |
| santa-fe | [Photographic source](https://www.realtor.com/realestateandhomes-detail/1299-Canyon-Rd_Santa-Fe_NM_87501_M19205-54384)  | Property aerial includes neighboring homes: tan/rose earth-toned stucco, flat terraces, and a standing-seam metal pitched roof within the adobe context. The enclosing hue alone does not prove original adobe, so smooth stucco and earthen skins are separate alternatives. |
| san-diego | [Photographic source](https://www.zillow.com/apartments/san-diego-ca/charming-2-bedroom-bungalow-in-the-heart-of-historic-old-town-san-diego/CnPsJJ/)  | Occupied Congress Street courtyard photograph: cream stucco and clay tile beside a sage weatherboard bungalow with dark shingle roof. Mission Revival heritage evidence supports stucco/clay but does not justify replacing every nearby timber bungalow. |
| charleston | [Photographic source](https://www.redfin.com/SC/Charleston/65-Ashe-St-29403/home/188035717)  | Occupied Ashe Street aerial: white/pastel and blue-gray houses; substantial silver and red metal roof mixture, alongside dark shingles and flat infill. Source heritage inventories distinguish painted timber single houses, stucco and exposed brick. |
| quebec | [Photographic source](https://sothebysrealty.ca/en/property/quebec/region-capitale-nationale/la-cite-real-estate/2428356/10-rue-couillard/)  | Rue Couillard aerial: painted masonry, exposed gray stone party walls and brick; silver and blue sheet metal including mansards; white/green frames. Image cool cast is not a measured blue pigment census; supplied roof shapes remain authoritative. |
| bermuda | [Photographic source](https://www.islandcitylab.org/articles/lessons-from-bermuda-a-conversation-with-shaun-lavis)  | Harbor neighborhood photograph accompanying architect interview: white washed stepped roofs, white and pink houses plus turquoise and yellow, green shutters. Existing slate geometry/texture approximates the roof stone without adding modeled steps. |
| puebla | [Photographic source](https://www.eluniversalpuebla.com.mx/ciudad/cuantas-manzanas-tiene-el-centro-historico-de-puebla/) [Occupied street report](https://elpopular.mx/ciudadania-y-gobierno/2023/02/22/ayuntamiento-puebla-entrega-calles-juntas-auxiliares) | Historic-center aerial shows red-covered FLAT terraces and gray/white service roofs, not a city of pitched red tile. El Popular street photographs independently show painted render and exposed brick houses. Ochre/red/indigo wall alternatives follow the ministry’s documented palette. |
| zanzibar | [Photographic source](https://www.expertafrica.com/zanzibar/stone-town)  | Operator roofscape photograph: pale limewashed walls, gray corrugated metal, weathered red/brown iron and a small turquoise-painted component; carved brown timber balconies and doors. No blanket terracotta or thatch interpretation of red/brown coverings. |
| cape-town | [Photographic source](https://www.aventuraafrica.com/paises/sudafrica/actividades/bo-kaap)  | Bo-Kaap occupied street photograph: strongly painted yellow, purple, blue, lime and orange plaster with white trim and flat parapets. Scope is restricted to Bo-Kaap; this palette must not color all Cape Town, and concealed roof surfaces are not inferred from parapet color. |
| djenne | [Photographic source](https://www.fieldstudyoftheworld.com/living-heritage-earth-architecture-djenne/)  | Firsthand roofscape of occupied courtyards, rather than the Great Mosque: maintained sandy earth walls and flat earthen roofs, variable warm brown values, dark timber and occasional colored doors/shutters. No invented extra covering mixture within this narrow study. |
| ait-ben-haddou | [Photographic source](https://www.touristtotraveller.com/blog/ait-ben-haddou-morocco)  | Close inhabited ksar fabric: reddish earthen walls, flat earthen terraces, timber openings; minor visible stone bases do not make whole houses stone. Tight ksar study; not a prescription for the modern settlement across the river. |
| cuenca | [Photographic source](https://ecuadorrealestateagent.com/en/properties/2-story-family-house-near-machangara-river/) [Historic-center aerial](https://www.freepik.com/premium-video/drone-footage-cuenca-ecuador-showcasing-its-colonial-architecture-cobblestone-streets-red-tiled-roofs-aerial-views-historic-city-unesco-landmarks-stunning-andean-backdrop_5028095) | Occupied neighborhood aerial: clay tile and corrugated gray/green sheets, white/cream render and exposed brick infill. Historic-center aerial confirms greater clay prevalence and some flat roof additions. Gray corrugated sheets cannot all be identified as metal from the photo; an explicitly matte corrugated variant approximates their appearance without claiming their chemistry. |
| arequipa | [Photographic source](https://www.hotelariquepay.com/es/)  | Hotel’s roof view across historic street: pale sillar/render frontages and flat terraces; exposed modern brick/concrete additions behind. White sillar is not applied to every new concrete house, and distant mountainous suburbs are outside the study. |
| paramaribo | [Photographic source](https://mirador-blog.hu/post/paramaribo) [Neighborhood aerial](https://www.remax.sr/en/homes/homes-for-rent/hr711/albergastraat-12.html) | Firsthand Waterkant street: white painted clapboard houses, dark green shutters, red and silver sheet roofing; RE/MAX neighborhood aerial corroborates colored metal and rendered modern infill. Weathered and maintained houses both occur. |
| valparaiso | [Photographic source](https://ai.stanford.edu/~latombe/mountain/photo/chile-2015/valparaiso-2015.htm)  | Jean-Claude Latombe’s firsthand housing photograph: vertical corrugated painted wall cladding in blue, red, pink and yellow, rust-brown sheet roofs and white frames. Explicit corrugated wall variant avoids falsely drawing horizontal wooden clapboard. |
| brisbane | [Photographic source](https://www.planningapprovalgroup.com.au/character-heritage-property-brisbane)  | Planning practitioner’s occupied street photograph: cream weatherboard Queenslanders, blue-gray corrugated roofing and blue/white veranda trim. Council housing survey supports timber/iron alongside modern infill; painted green/red metal alternatives have lower authored weight. |
| feilding | [Photographic source](https://www.realestate.co.nz/property/11-montagu-street-feilding-manawatu-manawatu-whanganui/vghxqydj) [Community Trust housing photographs](https://manawatucommunitytrust.nz/unit-locations) | Montagu Street cottage photograph: white weatherboard and dark navy-gray corrugated metal; neighboring flat modern houses. Community Trust Derby Street units show pale block/render and corrugated or concrete-tile roofing, preventing a uniform cottage/wood assumption. |
| oamaru | [Photographic source](https://commons.wikimedia.org/wiki/File:Historic_Precinct,_Harbour_Street,_Oamaru.jpg)  | Photographer’s Harbour Street view: cream limestone commercial frontage, green/dark red joinery and red sheet roofs. Commercial heritage streets dominate this narrow study; weatherboard/render housing alternatives prevent wholesale limestone projection into adjoining streets. |

## Material identification sources

These sources support material identification and conservation context. They do
not establish the observed prevalence of today’s ordinary housing. Future design
rules are not substituted for the photographs above.

- bled: [Municipality, building typology](https://www.e-bled.si/wp-content/uploads/2025/06/Stavbna-tipologija-Bled_elaborat_koncna.pdf).
- keswick: [Lake District building traditions](https://lakedistrict.gov.uk/planning/conservation/building-traditions/).
- penrith: [Council conservation appraisal](https://www.eden.gov.uk/planning-and-building/conservation/conservation-areas/list-of-conservation-areas/penrith-conservation-area/).
- bath: [Council city-centre appraisal](https://app.bathnes.gov.uk/docs/temp/Planning-Policy/Placemaking-Plan/city_centre_web.pdf).
- albi: [ICOMOS evaluation](https://whc.unesco.org/document/152414).
- porto: [City cultural program](https://www.agenda-porto.pt/en/evento/centro-historico-do-porto-comemoracoes-dos-30-anos-da-classificacao-de-patrimonio-mundial/), [UNESCO restoration project](https://whc.unesco.org/document/185523).
- visby: [Region Gotland conservation guidance](https://gotland.se/bygga-bo-och-miljo/bygga-nytt-andra-eller-riva/bygglov-i-visby-innerstad).
- kyoto: [City landscape standards](https://www.city.kyoto.lg.jp/tokei/cmsfiles/contents/0000281/281401/HP-English.pdf).
- okinawa: [Official prefectural travel guide](https://visitokinawajapan.com/discover/traditional-culture/traditional-okinawan-houses/).
- bhaktapur: [UNESCO Kathmandu Valley](https://whc.unesco.org/en/list/121).
- hoi-an: [Town nomination dossier](https://whc.unesco.org/uploads/nominations/948.pdf).
- luang-prabang: [UNESCO town description](https://whc.unesco.org/en/list/479).
- singapore: [URA shophouse guidance](https://www.ura.gov.sg/conservation/conservation-resources/understanding-the-shophouse/), [URA restoration guidelines](https://www.ura.gov.sg/-/media/User-Defined/URA-Online/Guidelines/Conservation/Cons-GuidelinesDec2017.pdf).
- santa-fe: [NPS Pueblo Revival](https://www.nps.gov/subjects/architecture/pueblo.htm), [NPS adobe conservation](https://www.nps.gov/orgs/1739/upload/preservation-brief-05-adobe.pdf).
- san-diego: [NPS Mission Revival](https://home.nps.gov/articles/mission-revival-architecture.htm).
- charleston: [City preservation approvals](https://charleston-sc.gov/AgendaCenter/ViewFile/Agenda/_07292019-4796), [City repair guidance](https://charleston-sc.gov/DocumentCenter/View/37042/Resilience-Guidelines-for-property-owners?bidId=).
- quebec: [City building inventory](https://www.ville.quebec.qc.ca/citoyens/patrimoine/bati/thesaurus.aspx?id=202&type=2), [workers' houses](https://www.ville.quebec.qc.ca/citoyens/patrimoine/quartiers/saint_roch/interet/maison_d_ouvrier.aspx).
- bermuda: [Government roofing repair guidance](https://www.gov.bm/news/obtaining-slate-post-hurricane-repairs), [government maintenance specification](https://www.gov.bm/sites/default/files/East-End-Primary-School-External.pdf).
- puebla: [Mexico Ministry of Culture palette guide](https://www.gob.mx/cultura/prensa/nueva-guia-digital-brinda-a-propietarios-los-pasos-para-pintar-correctamente-sus-inmuebles-historicos-en-la-ciudad-de-puebla?idiom=es-MX).
- zanzibar: [UNESCO Stone Town](https://whc.unesco.org/en/list/173).
- cape-town: [City heritage maintenance guidance](https://resource.capetown.gov.za/documentcentre/Documents/Procedures%2C%20guidelines%20and%20regulations/Heritage_Caring_for_your_old_building.pdf).
- djenne: [UNESCO old towns](https://whc.unesco.org/en/list/116).
- ait-ben-haddou: [UNESCO ksar](https://whc.unesco.org/en/list/444).
- cuenca: [UNESCO earthen-city case study](https://whc.unesco.org/document/126549/).
- arequipa: [ICOMOS evaluation](https://whc.unesco.org/document/1076).
- paramaribo: [UNESCO historic town](https://whc.unesco.org/en/list/940).
- valparaiso: [UNESCO historic seaport](https://whc.unesco.org/en/list/959).
- brisbane: [Council traditional-housing guide](https://www.brisbane.qld.gov.au/content/dam/brisbanecitycouncil/corpwebsite/building-and-planning/documents/traditional-housing-alterations-and-extensions-design-guide.pdf.coredownload.pdf), [Council house inventory](https://heritage.brisbane.qld.gov.au/heritage-places/2821).
- feilding: [Heritage New Zealand, Broxt Cottage](https://www.heritage.org.nz/list-details/7682/Broxt-Cottage).
- oamaru: [Heritage New Zealand historic area](https://www.heritage.org.nz/list-details/7064/Oamaru-Historic-Area).

## Review

Run `cargo test -p vearth-gen regional_finishes --lib` for self-contained weighted
fabric validity, compatibility with the centrally selected roof, and Bled and
Bhaktapur finish mixtures. Central eligibility and source constraints are checked
by the production resolver tests. Deterministic checks do not establish visual
quality: compare production staging and geographic captures against the
photographic source links before accepting a palette change.

## Porto roof and wall pairing

[VisitPorto historic district panorama](https://visitporto.travel/en-GB/districts/historic) shows warm clay roof planes across rendered and stone buildings. Both local wall fabrics use the same warm clay covering prior, with modest age variation; granite walls do not imply predominantly gray-brown roofs. The local consumer uses plain contemporary geometry with zero facade expression. Ceramic-faced walls remain a plaster approximation.
