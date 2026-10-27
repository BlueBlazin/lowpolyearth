# Regional visual grammar sourcebook

`architecture_styles.rs` supplies reusable geometry profiles and named-place
palettes. `architecture_resolver.rs` selects one compatible local, named-place,
subdivision or national recipe before resolving its surfaces. Style profiles
cannot repaint a design after selection. A style key is not evidence that every
building at that location belongs to a historical tradition; named-place entries
remain subject to the same visual re-audit as national and regional recipes.

## Admission and composition

- Preserve observed footprints, height, building use and roof geometry. Wall/roof
  materials and colors are currently inferred, not ingested per-building observations.
  Regional expression supplies finish and facade information; it must fit the
  existing building envelope. Courtyard styles do not invent holes in mapped footprints.
- The selected recipe owns its alternatives. Country and local mixtures use variable
  relative weights; named-place palette slices have no fixed cardinality. Weights,
  dimensions, influence radii and expression strengths are authored interpretations,
  **not measured stock proportions or conservation-area boundaries**.
- A local accent may inform compatible untagged buildings. Sparse heritage tags are not
  the only way to acquire local character. Source use and scale determine recipe
  eligibility before finishes are sampled; domestic features attenuate within their
  declared bounds.
- Separate wall, trim, joinery, roof and dark glass colors. Country-wide saturation,
  grime or poverty presets are not appropriate. Regional color does not mean every wall
  is the same color or every opening has the same ornament.
- Distinguish ordinary upper windows from entrances. North African street facades often
  need modest rectangular windows and a more expressive doorway; repeated arches on
  every opening would overstate ornament.
- Roof shape and roofline are separate. A Huizhou stepped gable wall hides a pitched
  roof; a southern Moroccan stepped parapet can surround a flat roof. Source roof
  observations win over both inferred priors. Silhouette and large facade volumes
  belong in the base surface; tiny mullions and louvers may simplify with distance.

This treatment follows the emphasis on ordinary urban fabric and architectural layers
in the [UNESCO Urban Heritage Atlas](https://whc.unesco.org/en/activities/1211/) and the
[NPS guide to architectural character](https://www.nps.gov/orgs/1739/upload/preservation-brief-17-architectural-character.pdf).
Neither source establishes the numerical game mixtures.

## Evidence and discriminating ingredients

The table records architectural ingredients that can be checked against sources. It
is not a catalog of copied buildings. Palettes are authored approximations, and the
renderer simplifies carving, curved roofs, joinery and ornament into readable forms.

| Style keys | Source-supported ingredients and scope |
| --- | --- |
| `contemporary`, `contemporary-brick`, `contemporary-tropical`, `contemporary-asian` | Ordinary recent masonry, brick, shade galleries, balconies and shopfront variants keep every regional mix contemporary. These are deliberately broad compositional grammars. Stock evidence is tracked in [Regional art direction](REGIONAL_ART_DIRECTION.md), including [TABULA building typologies](https://episcope.eu/building-typology/) and [US Census cladding data](https://www.census.gov/library/visualizations/interactive/new-single-family-homes-2021.html). They do not establish a single appearance for modern Asia or the tropics. |
| `british-terrace` | Narrow repeated brick bays, tall openings and restrained lintel/sill treatment. Terraces span several periods and sizes, rather than one universal Georgian house. [Historic England](https://heritagecalling.com/2019/11/15/a-brief-introduction-to-terraced-housing/). |
| `paris-mansard` | Stone street wall, differentiated shop ground floor, upper balconies, cornice and steep attic roof. The Paris source describes the hierarchy of floors; this is a metropolitan historical accent, not all French housing. [City of Paris](https://www.paris.fr/pages/haussmann-l-homme-qui-a-transforme-paris-23091). |
| `classical-limestone` | Pale masonry, regular tall bays, shop ground row and clear sill/cornice/pilaster hierarchy, without an imposed Parisian attic roof. Bordeaux and Fremantle have different histories and roof traditions but compatible classical facade ingredients. [UNESCO Bordeaux](https://whc.unesco.org/en/list/1256/), [Fremantle West End design policy](https://www.fremantle.wa.gov.au/document/lpp-3-21-west-end-heritage-area-2/), [city limestone guide](https://www.fremantle.wa.gov.au/document/looking-after-limestone-walls-in-heritage-buildings/). |
| `maltese-gallarija` | Warm limestone facade and an enclosed projecting timber balcony with glazed panels, colored framing and independent parapet. The balcony's form, rather than generic shutters, supplies local identity. [University of Malta balcony research](https://www.um.edu.mt/library/oar/handle/123456789/81340), [illustrated Maltese balcony study](https://www.um.edu.mt/library/oar/bitstream/123456789/37461/1/il-gallerija%20maltija%20images.pdf). |
| `luso-whitewash` | Sobrado-derived rhythm with whitewashed wall body and colored joinery; Paraty is not assigned Salvador's uniformly polychrome wall mix. [Brazil Tourism Ministry illustrated creative cities guide](https://www.gov.br/turismo/pt-br/centrais-de-conteudo-/publicacoes/copy_of_CidadesCriativas.pdf). |
| `dutch-gable` | Narrow masonry frontage, vertical bays and a shaped front crown. The documented Amsterdam stock includes stepped, neck, bell and later straight cornice facades; the game abstracts these, without reproducing every gable type. [UNESCO Amsterdam atlas](https://whc.unesco.org/en/urban-heritage-atlas/Amsterdam). |
| `half-timber` | Visible timber grid with infill and projecting upper bays; not generic exposed beams pasted onto every central European building. [Historic England traditional buildings](https://heritagecalling.com/2022/08/25/a-guide-to-traditional-english-buildings/). |
| `alpine-gallery` | Deep pitched shelter, timber upper treatment and sheltered gallery. Ballenberg records squared-timber house bodies with galleries under the roof edge; precise gallery frequency is an authored prior. It must not transfer into Iranian mountains solely because they share the `Alpine` construction family. [Swiss Ballenberg museum](https://ballenberg.ch/en/themes/house-styles/7-central-switzerland/751/). |
| `nordic-clapboard` | Painted timber/cladding, simple front gables and strong trim contrast. Bryggen documents timber bodies, clad facades and galleries alongside modern surroundings; not all Nordic stock is timber or historic. [UNESCO Bryggen](https://whc.unesco.org/en/list/59/). |
| `balkan-oriel` | White masonry lower body, projecting timber-framed upper rooms, roof overhang and brackets. This is an Ottoman domestic bay grammar, not a mosque roof. [Turkish Ministry of Culture, Safranbolu houses](https://www.ktb.gov.tr/EN-113773/samples-for-the-safranbolu-houses.html). |
| `mediterranean-shutter`, `iberian-tile` | Tall shuttered openings, lime/render or masonry, clay roof and framed bays. These are compatible shared ingredients rather than a claim that Iberia, Italy and the Balkans are interchangeable. Local sources such as [Amalfi's built environment](https://www.unescoamalficoast.it/the-territory-and-its-people/the-built/prestigious-buildings/?lang=en) support shaded domestic forms; precise tile/trim distributions remain authored. |
| `amalfi-terrace` | Light or softly colored rendered volumes, parapeted terrace edge, loggia/gallery shade and arched openings. The local heritage source also documents extrados vaults, pergolas and courtyard houses; these more complex elements are not all reproduced by this grammar. Sloped site placement comes from world terrain and mapped footprints. [Local cultural heritage source](https://www.unescoamalficoast.it/the-territory-and-its-people/the-built/prestigious-buildings/?lang=en), [UNESCO evaluation](https://whc.unesco.org/document/154393). |
| `bahia-sobrado` | Polychrome plaster, tall shuttered upper openings, shallow balconies, framed bays and floor/crown bands. Salvador's historic sobrados provide the reference, including simpler houses alongside ornate ones; modern Salvador is not uniformly colonial. [UNESCO Salvador](https://whc.unesco.org/en/list/309/), [IPHAN architectural atlas](https://portal.iphan.gov.br/uploads/publicacao/ColObrRef_AtlasMonumentosHistoricosArtisticosBrasil.pdf). |
| `hispanic-patio`, `andean-gallery` | Masonry/render, tiled shelter, framed entrances and selected upper timber balconies. Cuenca's history documents adobe patio houses and later brick/European-influenced facades; Cusco's inventories record timber balconies, stone bases and tiled eaves. These shared ingredients preserve mapped courtyards and do not recreate a whole colonial mansion. [UNESCO Cuenca study](https://whc.unesco.org/document/126549), [Peru Culture Ministry photographic inventory](https://repositorio.cultura.gob.pe/bitstream/handle/CULTURA/1311/02%20Imagenes%20del%20Cuzco%20LEV%202005.pdf?isAllowed=y&sequence=3), [Cusco domestic house archive](https://bvirtual.culturacusco.gob.pe/items/show/53). |
| `caribbean-gallery` | Tall shuttered openings, shaded gallery and porch, independent joinery color. New Orleans conservation drawings distinguish Creole cottage, townhouse, shotgun and later styles; this is one transferable tropical gallery grammar, not the complete Caribbean catalog. [New Orleans architectural styles](https://nola.gov/nola/media/HDLC/Guidelines/03-Architectural-Styles.pdf), [Vieux Carré glossary](https://vieuxcarre.nola.gov/). |
| `willemstad-gable` | Dutch-derived front crown combined with Caribbean gallery and vivid plaster. The historical sequence includes later restrained straight gables as well as curved examples. [ICOMOS Willemstad evaluation](https://whc.unesco.org/document/154358). |
| `north-american-porch`, `north-american-bay` | Siding/brick, pitched roof, porch or projecting bay, strong casing and base. These are common compositional ingredients, not measured nationwide proportions. [New Orleans conservation typologies](https://nola.gov/nola/media/HDLC/Guidelines/03-Architectural-Styles.pdf) and [Census material variation](https://www.census.gov/library/visualizations/interactive/new-single-family-homes-2021.html) establish varied ordinary stock; source data must distinguish regional materials. |
| `pueblo-terrace` | Earth-toned plaster, flat parapet and deep small openings. The NPS source documents stepped massing and timber beam ends; the game retains source massing and simplifies beam detail. It is a southwest local accent, not all dry North America. [NPS Pueblo architecture](https://home.nps.gov/subjects/architecture/pueblo.htm). |
| `chilean-cladding` | Colored clad body, projecting bay and pitched metal roof, suited to compatible southern and port-town buildings. [UNESCO Valparaíso](https://whc.unesco.org/en/list/959/) supports heterogeneous hillside housing; the exact cladding palette and bay distribution are authored, not derived from census shares. |
| `tunisian-medina` | Quiet warm-white render, modest upper openings, expressive doorway, occasional screened bay and colored wood. [UNESCO Tunis](https://whc.unesco.org/en/list/36/), [Tunisian National Tourist Office](https://www.discovertunisia.com/decouvrir/carthage-et-sidi-bou-said) document door forms, colored joinery, frames and moucharabiehs. |
| `sidi-bou-said` | White walls with blue shutters/screens and doorways, distinct from a blue-washed wall palette. Reused where this component grammar is compatible, including Essaouira. [Tunisian National Tourist Office](https://www.discovertunisia.com/decouvrir/carthage-et-sidi-bou-said), [Moroccan National Tourist Office, Essaouira](https://www.visitmorocco.com/en/travel/essaouira-mogador). |
| `tetouan-medina` | White plaster, green wood/tile accents and restrained Andalusi domestic openings. The local source explicitly contrasts the medina with adjacent European-influenced urban fabric. [Moroccan National Tourist Office](https://www.visitmorocco.com/en/travel/tetouan/medina). |
| `moroccan-white-blue` | Chefchaouen-inspired blue and white washes with modest openings and blue wood; this key is not a Morocco-wide color preset. [Moroccan National Tourist Office](https://www.visitmorocco.com/en/blog/blues-chefchaouen). |
| `moroccan-red-medina` | Red/ochre render, sparse street openings, shaded/screened bays and emphasized entrance. Marrakech is the local red-city reference; not all Moroccan medinas share it. [UNESCO Marrakech](https://whc.unesco.org/en/list/331/). |
| `atlas-kasbah` | Earthen body, small openings, strong vertical relief and upper parapet pattern. Aït-Ben-Haddou includes modest dwellings and larger fortified houses; ordinary houses must not all become castles. [UNESCO Aït-Ben-Haddou](https://whc.unesco.org/en/list/444/). |
| `sahel-relief` | Earthen finish, strong entrance composition, vertical pilaster/buttress rhythm and restrained upper edge. Djenné's domestic facades, not its mosque's silhouette, are the reference; modern masonry and metal alterations coexist. [UNESCO Djenné](https://whc.unesco.org/en/list/116/). |
| `west-african-veranda` | Deep gallery/veranda, louvered shade and metal-roofed masonry. Grand-Bassam's gallery buildings coexist with simpler local compounds, so one colonial type cannot represent the whole city or West Africa. [UNESCO Grand-Bassam atlas](https://whc.unesco.org/en/canopy/grand-bassam/), [evaluation](https://whc.unesco.org/document/116989). |
| `swahili-stone-town` | Limewashed masonry, carved-door framing, timber joinery and selected upper balconies. Zanzibar's stone houses coexist with smaller Swahili dwellings and bazaar shops. [UNESCO Stone Town](https://whc.unesco.org/en/list/173/). |
| `cape-gable` | White plaster, shaped front gable and dark timber trim. Cape Dutch is one layer among Georgian, Victorian and later housing, rather than a continental default. [South African Heritage Resources Agency, Bo-Kaap](https://www.sahra.org.za/the-bo-kaap/), [Cape domestic heritage documentation](https://whc.unesco.org/document/6806). |
| `malagasy-veranda` | Brick/render body with timber veranda and pronounced pitched roof. Madagascar's culture ministry documents timber, brick and stone multistorey veranda houses and layered influences in Antananarivo; transfer beyond the highlands is uncertain. [State Party inventory](https://whc.unesco.org/en/tentativelists/6078). |
| `asmara-modernist` | Broad horizontal openings, balcony/slab bands, stepped parapet and restrained geometric accents. Asmara includes domestic and commercial modernism alongside earlier eclectic facades; these are not merely beige flat boxes. [UNESCO Asmara atlas](https://whc.unesco.org/en/urban-heritage-atlas/asmara/), [facade documentation](https://whc.unesco.org/document/155767). |
| `hejazi-roshan` | Projecting stacked timber screen bays on a rendered tower body. Jeddah's roshan houses have outward expression distinct from courtyard-focused domestic traditions elsewhere. [UNESCO Jeddah](https://whc.unesco.org/en/list/1361/), [recording document](https://whc.unesco.org/document/167414). |
| `sanaani-tower` | Brown brick body, light gypsum opening surrounds and strong horizontal/vertical decoration. This does not describe Shibam's smoother earthen towers; do not transfer it merely because both are Yemeni tower towns. [UNESCO Sana'a](https://whc.unesco.org/en/list/385/). |
| `persian-courtyard` | Quiet street facade, warm masonry, deep openings and framed entrance. A courtyard-centered grammar preserves mapped enclosure. Iranian regular courtyard houses and Central Asian accreted compounds differ; this is not permission to add mosque domes or force all Bukhara housing into an Iranian model. [Uzbekistan's makhalla documentation](https://whc.unesco.org/en/tentativelists/6578/), [Bukhara courtyard house recording](https://whc.unesco.org/document/156894). |
| `jaisalmer-haveli` | Golden sandstone, projecting jharokha bays, deep relief and lattice-inspired shadow. Wealthy historic havelis are an expressive reference, not a template for every Rajasthan house. [Rajasthan tourism](https://www.tourism.rajasthan.gov.in/jaisalmer.html), [India Tourism, Patwon ki Haveli](https://www.incredibleindia.gov.in/en/rajasthan/jaisalmer/patwon-ki-haveli). |
| `jodhpur-haveli` | Blue/lime wall mixture, contrasting stone surrounds and projecting bays. Local old-town identity is allowed to influence compatible houses; blue is not assigned to every building in Jodhpur or Rajasthan. [Rajasthan tourism](https://www.tourism.rajasthan.gov.in/jodhpur.html). |
| `jaipur-bazaar` | Rose plaster, street arcade, pilasters, upper arches, repeated cornice and parapet rhythm. Jaipur's evaluation documents multiple facade periods, including later Art Deco and modern growth beyond the old city. [ICOMOS Jaipur evaluation](https://whc.unesco.org/document/176193). |
| `gujarati-pol` | Timber bays, brackets, framed infill, small porch and tall close-set openings. Ahmedabad's domestic composite timber/brick-lime construction informs the grammar; retained courtyard data establishes layout. [UNESCO Ahmedabad atlas](https://whc.unesco.org/en/urban-heritage-atlas/ahmedabad/). |
| `kerala-veranda` | Broad clay-tile shelter, shaded timber-column veranda and restrained render. Nalukettu mansions are one historic type; the local source explicitly notes modern houses often retain only parts of it. [Kerala tourism](https://www.keralatourism.org/kumarakom/nalukettus-kumarakom.php), [architectural overview](https://www.keralatourism.org/faq/what-makes-keralas-traditional-architecture-unique). |
| `bhutanese-rabsel` | Projecting timber window groups, dense frames, contrasting bands, substantial cornice and overhanging roof. The government diagrams distinguish components and proportions; do not substitute a generic chalet. [Bhutan Architectural Guidelines](https://www.moit.gov.bt/wp-content/uploads/2010/11/Bhutan-Arch-Guidelines-final-2014.pdf). |
| `himalayan-timber` | Timber-framed openings and bands within a brick/masonry body with deep roof edge. Kathmandu's domestic record distinguishes carved traditional windows and lower storeys from simpler, taller recent reconstruction. This shared grammar is not a complete inventory of Newar, Tibetan, Kashmiri or Himalayan traditions. [FAO Bhaktapur domestic construction](https://www.fao.org/4/l2680e/l2680e01.htm), [Kathmandu neighborhood assessment](https://whc.unesco.org/document/180606), [UNESCO reconstruction review](https://whc.unesco.org/document/158654). |
| `chinese-courtyard` | Gray masonry, dark tile roof, modest timber openings and deep eaves. Pingyao conservation documents street, side and principal house relationships; the game retains source layouts. This avoids applying Anhui horse-head gables across China. [Pingyao conservation guidelines](https://globalheritagefund.org/images/uploads/docs/PingyaoFinalDesignEN20140612.pdf), [original dwelling study](https://doi.org/10.3390/buildings14103266). |
| `huizhou-gable` | White walls, dark tile, small street-facing openings and stepped gable/firewall silhouette. Xidi and Hongcun are the Anhui reference; it is not a generic East Asian roof. [UNESCO Xidi and Hongcun](https://whc.unesco.org/en/list/1002/), [nomination](https://whc.unesco.org/uploads/nominations/1002.pdf). |
| `kyoto-machiya` | Dense timber lattice, low upper-floor proportions, continuous near eave and shop/residence ground row. This is an urban merchant-house grammar, not a temple facade. [Kyoto municipal machiya resource](https://kyomachiya.city.kyoto.lg.jp/utilize/ryukoku-en/), [measured heritage examples](https://kyomachiya.city.kyoto.lg.jp/en/study/vr/). |
| `korean-courtyard` | Visible timber framing, sheltered raised hall/veranda, low body and tile roof. The city source explicitly distinguishes regional layouts and ordinary houses from wealthy multi-building compounds. [Seoul Hanok structure](https://hanok.seoul.go.kr/front/eng/info/infoHanok.do?tab=2). |
| `straits-shophouse` | Tall shuttered upper windows, pilasters/bands and covered ground arcade/five-foot way. URA describes several periods from restrained early forms to ornate later ones; the profile is a compatible expressive variant, not all Singapore buildings. [URA shophouse anatomy](https://www.ura.gov.sg/conservation/conservation-resources/understanding-the-shophouse/), [conservation guide](https://www.ura.gov.sg/-/media/Corporate/Guidelines/Conservation/20260408_Your-Shophouse-Do-It-Right.pdf). |
| `hoi-an-merchant` | Restrained yellow/render and timber frontage, broad wood openings, low tiled shelter and shop ground row. It is distinct from the taller stuccoed Straits facade. [UNESCO Hoi An](https://whc.unesco.org/en/list/948/), [Hoi An heritage authority](https://hoianheritage.net/en/specialised-exchanges/research-exchange/three-bay-house-with-a-projecting-front-room-a-house-of-memory-and-nostalgia-17.html). |
| `vietnamese-tube` | Narrow bay rhythm, stacked balcony and shop/residence composition represent an authored contemporary urban counterpart to the historic merchant houses. A footprint remains as mapped; no synthetic parcel subdivision is claimed. Hoi An's historic record alone does not establish contemporary nationwide tube-house prevalence. |
| `malay-veranda` | Timber/clad body, shaded veranda, ventilation and pitched shelter. Contemporary Malaysian construction has diverged substantially, so city mixtures retain modern stock. Tile/metal roof substitutions are game-compatible finishes, not claims that traditional thatch was tile. [Malaysia CIDB](https://www.cidb.gov.my/eng/exploring-the-green-architecture-of-traditional-malay-houses/). |
| `indonesian-tiered` | A domestic Joglo-inspired central hip with broad lower slopes, timber columns and front terrace. It has no pagoda finials or detached temple roofs and is not a universal Indonesian form. [Yogyakarta heritage authority, documented house](https://kebudayaan.kemdikbud.go.id/bpcbyogyakarta/rumah-joglo-peniung/), [Indonesian culture ministry on Betawi roofs](https://kebudayaan.kemdikbud.go.id/ditwdb/arsitektur-rumah-betawi-2/). |
| `philippine-bahay` | Solid masonry lower storey, lighter timber upper body, wide sliding/shuttered openings and strong eave. Capiz itself is approximated as muted panels; roof repair materials can differ from historic clay. [Vigan nomination](https://whc.unesco.org/uploads/nominations/502rev.pdf), [Philippine cultural registry](https://talapamana.ncca.gov.ph/index.php/component/content/article/tluz?Itemid=101&catid=9). |
| `queenslander`, `australasian-cottage` | Timber cladding, metal roof, generous veranda and raised base. Queensland's examples include broad galleries, whereas the cottage grammar is restrained; neither inherits North American asphalt shingle defaults. [Brisbane heritage: Shire Clerk's cottage](https://heritage.brisbane.qld.gov.au/heritage-places/1907), [modern reinterpretation](https://heritage.brisbane.qld.gov.au/heritage-places/2821). |
| `pacific-veranda` | Restrained timber/clad, metal-roofed veranda grammar, used as compatible ordinary shelter rather than a ceremonial Pacific building. Regional settlement/material evidence is in [Regional art direction](REGIONAL_ART_DIRECTION.md). This is not a complete inventory of island-specific architecture. |

## Local catalog and shared geometry

The source-identified [place catalog](../crates/vearth-gen/assets/architecture/places.json)
records each locality, covered quarter, interpretation and architectural references.
Its [manifest](../crates/vearth-gen/assets/architecture/places-manifest.json) gives
verified coverage counts. Local profiles reuse compatible construction operations;
their geographic anchors, proportions, finish roles and detail choices remain explicit.

- `alberobello-trulli` uses a bounded sixteen-face conical roof with a flat apron,
  fitted inside the observed footprint and height. [UNESCO](https://whc.unesco.org/en/list/787/).
- `sibiu-eyebrow` uses at most four shallow lens dormers on suitable untagged roofs,
  omitted when their height is unresolved. [Sibiu Tourism](https://www.sibiu-turism.ro/Ce-vizitam-Destinatii-Orase-istorice-Sibiu-Puncte-de-atractie-Ochii-orasului.aspx).
- `shirakawa-ogimachi` uses steep inferred gables and a shared thatch material.
  [UNESCO](https://whc.unesco.org/en/list/734/).
- `kurashiki-bikan` uses a dark lower wall with raised diagonal lime joints in the
  shared namako material. [Okayama Tourism](https://www.okayama-japan.jp/en/spot/10736).

Measured roof geometry and explicit roof shapes suppress inferred roof substitutions.
These features use the existing projected-size gates, detail budget, batched surfaces
and source-envelope collision. Thatch fibers and namako joints are material texture,
not repeated small mesh elements. Place queries use precompiled typed styles in the
existing sparse geographic index and return at most two district influences plus
one city fallback.

Inspect a local style with the production generator and renderer:

```sh
cargo run --release -p vearth-app --bin vearth-app -- \
  --stage buildings:place:alberobello-trulli
```

Each view uses four independently generated, ordinary untagged source buildings
at the catalog anchor. It retains geographic selection and stable infill; it is
an inspection fixture, not a reconstruction of four observed buildings.

## Review checklist

- [ ] For a new local signature, identify everyday domestic/commercial evidence, not
  just the most photographed palace or religious monument.
- [ ] Record source location, scope and compatible construction; explain cross-region
  borrowing and distinguish observed geography from authored influence strength.
- [ ] Check facade proportion, roof silhouette, ground-floor treatment, material and
  palette roles together. A new name with the same box and windows is not new coverage.
- [ ] Keep modern and restrained variants. Check inner city, outer city, town and rural
  cases, including territories and subdivisions crossing construction traditions.
- [ ] Verify source roof/use overrides and inferred palette consistency, nearby-building variation, scale
  attenuation and deterministic style selection across adjacent streamed cells.
- [ ] Check the intended character in ordinary in-game views, including source
  footprints whose proportions do not fit the ideal reference building.

Run focused contracts with `cargo test -p vearth-gen architecture_styles` and place
coverage with `cargo test -p vearth-gen architecture_places`. Geometry tests live
with the surface generator; complete the repository-wide checks before shipping.
