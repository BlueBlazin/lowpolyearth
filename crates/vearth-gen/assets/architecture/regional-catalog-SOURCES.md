# Regional construction references

Country and subdivision entries in `countries/*.json` own production stock and
finish choices. `architecture_resolver.rs` selects one complete compatible
provider; local fabrics and named-place recipes are alternative providers.
The administrative atlas supplies geographic context, not age, ethnicity,
visible materials or heritage designation. Source roof observations remain
constraints rather than catalog suggestions.

This reference index supplies research leads and documented material traditions.
It does not define active administrative assignments, stock frequencies or visual
acceptance. The exact evidence scope and unresolved gaps belong to each JSON
entry. Source photographs are not redistributed as game assets.

## Material references

| Geographic/material subject | Primary references |
| --- | --- |
| AtlanticSlate: western Britain and Scotland | [Cornwall Council](https://www.cornwall.gov.uk/planning-and-building-control/conservation-and-planning/advice-for-works-to-historic-buildings/); [Scottish Government](https://www.gov.scot/publications/guide-conversion-traditional-buildings-2/pages/11/) |
| AtlanticSlate: Brittany | [Brittany tourism authority](https://www.tourismebretagne.com/app/uploads/crt-bretagne/2020/05/my-travel-memo-a-colourful-escapade-on-the-pink-granite-coast.pdf) |
| NorthEuropeanBrick | [UNESCO, Stralsund and Wismar](https://whc.unesco.org/en/list/1067/) |
| JulianAlpine | [Bled municipal building-typology study](https://www.e-bled.si/wp-content/uploads/2025/06/Stavbna-tipologija-Bled_elaborat_koncna.pdf); [Triglav National Park](https://www.tnp.si/en/park/nature-and-culture/building-typology/) |
| Slovenian Alpine and Adriatic transitions | [Triglav National Park](https://www.tnp.si/en/park/nature-and-culture/building-typology/); [Slovenian tourism authority](https://www.slovenia.info/en/places-to-go/regions) |
| Adriatic: Croatia | [City of Dubrovnik](https://www.dubrovnik.hr/stradanje-grada/the-destruction-and-restoration-dubrovnik-1991-2000/devastation-restoration-roofs) |
| Aegean | [Greek National Tourism Organisation, Folegandros](https://www.visitgreece.gr/en/islands/Cyclades/Folegandros); [Cyclades Chamber](https://beta.e-kyklades.gr/travel/en/thalassa-paralies/gnoriste-tis-kyklades/) |
| Maltese | [Malta Planning Authority design guidance](https://www.parlament.mt/media/80420/06651.pdf) |
| PortugueseAtlantic and Azores | [Portuguese tourism authority, Guimarães](https://cdn.visitportugal.com/en/content/guimaraes); [Azores conservation provisions](https://www.culturacores.azores.gov.pt/ficheiros/legislacao/201511310285.pdf) |
| Pueblo | [US National Park Service](https://www.nps.gov/subjects/architecture/pueblo.htm); [NPS adobe preservation guidance](https://www.nps.gov/orgs/1739/upload/preservation-brief-05-adobe.pdf) |
| PeruvianCoast | [INEI housing-material survey](https://proyectos.inei.gob.pe/web/biblioineipub/est/lib0072/C5-2.htm); [2017 census summary](https://censo2017.inei.gob.pe/en-el-pais-existen-mas-de-diez-millones-de-viviendas-particulares-censadas/) |
| ChileanLakes | [Chile's Ministry of Culture](https://observatorio.cultura.gob.cl/index.php/2021/03/08/la-piel-de-chiloe-un-viaje-a-traves-del-diseno-de-la-tejuela/); [National Library, Casas de madera](https://www.memoriachilena.gob.cl/archivos2/pdfs/MC0031706.pdf) |
| Tamil | [Census of India roof-material table](https://censusindia.gov.in/nada/index.php/catalog/10042); [India's Chettinad nomination](https://whc.unesco.org/en/tentativelists/5920/) |
| Gujarati | [UNESCO/Ahmedabad Municipal Corporation](https://whc.unesco.org/en/urban-heritage-atlas/ahmedabad) |
| Bhutanese | [Bhutan's Department of Tourism](https://bhutan.travel/experiences-culture); [Tokyo National Research Institute for Cultural Properties field survey](https://www.tobunken.go.jp/materials/ekatudo/2059941.html) |
| Hui | [UNESCO's Anhui nomination](https://whc.unesco.org/uploads/nominations/1002.pdf) |
| Cape | [City of Cape Town maintenance guidance](https://resource.capetown.gov.za/documentcentre/Documents/Procedures%2C%20guidelines%20and%20regulations/Heritage_Caring_for_your_old_building.pdf); [Mamre guidance](https://resource.capetown.gov.za/documentcentre/Documents/Graphics%20and%20educational%20material/HAP15_MamreMissionVillage.pdf) |
| Tasmanian | [Heritage Tasmania's architectural styles](https://heritage.tas.gov.au/useful-resources/education-resources-for-schools/architectural-styles) |

## Ordinary housing evidence

Country and subdivision specifications in `countries/*.json` record their
evidence, inspected visual references, review status and relative-weight basis.
`pending` entries have not passed the ordinary-stock review; `stock-reviewed`
does not establish facade-color fidelity. Photo examples establish visible
alternatives, not measured national prevalence. Structural wall and roof
survey categories must not be mistaken for exposed finishes.

The [IEE Projects TABULA + EPISCOPE](https://episcope.eu/building-typology/country/)
national typologies provide ordinary building photographs and construction
descriptions by age and size. The exact inspected report pages are recorded in
each specification; source photographs are not redistributed as game assets.
The Dutch specification also uses the [RVO ordinary housing catalog](https://www.rvo.nl/sites/default/files/2023-01/brochure-voorbeeldwoningen-bestaande-bouw-2022.pdf).
Color and settlement weights are authored visual inferences unless the entry
identifies a compatible measured survey share.

## Validation contract

```sh
python3 tools/build-country-architecture.py --check
python3 tools/test-country-architecture.py
cargo test -p vearth-gen --lib architecture_resolver
cargo test -p vearth-gen --lib architecture_recipes
```

Validation checks source coverage, exact subdivision names, positive relative
weights, material roles, evidence status and immutable provider ownership.
Mapped-world screenshots and user approval establish visual acceptance; passing
these checks does not establish architectural resemblance.
