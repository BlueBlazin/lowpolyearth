# Architectural place atlas

`places.json` is the editable source-identified catalog. The generator verifies
the source coordinates and produces `places-generated.rs`, consumed by
`architecture_places.rs`. `places-manifest.json` records the catalog/output
checksums and exact coverage. The catalog supplements the exhaustive country and
subdivision construction catalog; it is not an exhaustive inventory of historic
districts or buildings.

## Interpretation and units

Every envelope is explicitly `authored-art-influence`. Its central point is an
observed geographic identifier, the documented LPC-derived focus, or an explicit
`influence_center` supported by a source URL and interpretation. A historic-center
focus does not replace the pinned search identity: the compiler verifies that
identity and preserves its original coordinates separately. **Its radii, visual confidence, scale hints and
infill probability are authored game art direction**. None is a surveyed heritage
boundary, measured building age, census frequency or legal protection boundary.
The `art_interpretation` and `evidence_urls` on every entry explain the intended
visual inference and its architectural references. `evidence_scope` distinguishes
place-specific character from a compatible regional transfer where a direct
domestic inventory is unavailable; the latter carries weaker detail confidence
and more ordinary infill.

Separated neighborhoods use `influence_areas`, each with its own documented
center and authored radii. Runtime influence is the maximum of these areas;
it does not connect them with a larger circle or count overlapping areas twice.
The spatial index includes every area but stores each identity once per bucket.
An entry uses one representation: separated circular `influence_areas`, a single
`influence_center`, a connected `influence_corridor`, or a pinned `influence_polygon`.

`influence_corridor` requires:

- `points`: at least two `{latitude_deg, longitude_deg}` coordinates in route order.
- `core_half_width_m`, `outer_half_width_m`: finite authored widths, with
  `0 <= core < outer <= 20000`.
- `source_url` and `note`: the street/canal source and interpretation of its line.
- Optional paired `source_snapshot` and `source_sha256`: a relative file under
  this architecture asset directory and its lowercase SHA-256. The compiler
  rejects traversal, escaping symlinks, missing files and checksum mismatches.

Corridors follow finite shorter spherical arcs with round endcaps; bends do not
fill the space between their arms. Consecutive points must be distinct and
non-antipodal. Each segment's half-length plus outer width must fit the 20 km
index-cap limit. Circular radius fields are ignored for corridor entries.
Widths remain art direction, not surveyed boundaries. `representative_center()`
provides a point on the envelope centerline for staging; verified source identity
coordinates remain separate. Corridors do not represent polygon boundaries or holes.

`influence_polygon` requires `source_url`, `note`, `source_snapshot`,
`source_sha256`, and a positive `boundary_falloff_m` in metres. Its pinned WGS84
GeoJSON must be a Polygon/MultiPolygon geometry or one Feature containing it;
ambiguous collections and legacy CRS declarations are rejected. Rings must be
finite, closed and non-degenerate. All source vertices, holes and disconnected
components are retained. Each component must fit a conservative 20 km spherical
cap; this representation supports local districts, not continental boundaries.

Polygon influence is zero outside its exterior, in holes and on boundaries.
Smoothstep fades inward over `boundary_falloff_m`, measured in the compiled local
gnomonic plane; shorter great-circle edges are straight in that plane. Sparse
indexing covers each complete cap, including interior cells and antimeridian
wrap. Runtime rejects queries outside the cap before testing edges and allocates
nothing. A compiled interior point supports staging without filling holes or
connecting components. Boundary membership constrains the recipe's geographic
scope; it does not classify every building as historic.

The shared resolver uses geodetic radians and spherical surface distance in
metres, a constant inner plateau, and smoothstep falloff to zero at the outer
radius or corridor half-width. `scope_tier` is `district` or `city`; omission defaults to `district`
for existing catalogs. It describes the researched recipe's geographic specificity,
not the source identifier type or radius size. Add `scope_tier: "city"` explicitly
for ordinary city stock; retain narrower district identities independently.

Queries reserve two district candidates and one city fallback. Districts precede
the city even when their falloff weight is weaker. Each tier ranks by strength,
then source ID and style key. Empty slots are skipped; a city-only query remains
reachable through `influence_at`. The resolver applies its existing source-scale
compatibility and probabilistic admission in that order, so a rejected district
can reach the city. Capacity is bounded: a third overlapping district or second
city is omitted. Catalog authors should avoid redundant overlapping identities;
the city fallback remains available if both retained districts are rejected.

Confidence and infill values are fractions in `[0,1]`; height is metres
and footprint area square metres. The sparse two-degree index has no per-query
allocation or runtime network access. Longitude wraps across the antimeridian;
invalid coordinates produce no influence.

Consumers must preserve source footprints, courtyards, heights, use and measured
roof facts, attenuate incompatible scale/use, and retain contemporary infill.
Current source building metadata does not report individual facade color or wall
material: these finishes are inferred, not a match to an observed source finish.
An ordinary surrounding building can adopt a compatible local finish or modest
detail without being declared historic. The atlas does not insert missing
buildings, roads, terrain terraces, forts, temples or courtyards. Character-place
influence can overlap and does not replace the regional baseline outside it.

Source city points sometimes lie away from their best-known historic quarter.
The catalog therefore uses separate, correctly named Wikidata district/landmark
anchors for cases including Tunis medina, Pelourinho, Rajasthan's three distinct
cities, Fort Kochi, Kathmandu's Durbar Square and Cusco's Plaza de Armas. A fort
point may anchor surrounding domestic character; it does not assign fort form
to the buildings. Radius precision must never be interpreted as survey accuracy.

Style names describe reusable visual grammars. For example, `sidi-bou-said`
expresses white plaster with blue joinery and is also used for compatible
Essaouira domestic finishes; `bahia-sobrado` provides restrained Luso-Brazilian
street-front machinery for Olinda and Ouro Preto as well as Salvador. Such reuse
does not assert cultural identity or identical building stock.

## Sources and licenses

- Geographic city identities and coordinates: © GeoNames,
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), derived without changing
  coordinates from the existing bundled `place-search-v1.bin`. Its snapshot,
  checksums, filters and full rebuild procedure are in
  `crates/vearth-data/assets/place-search-SOURCES.md`. The builder verifies that
  artifact's SHA-256 and rejects synthetic project-curated search records as
  GeoNames sources. Exact source name, country and ID must match.
- District/landmark points: Wikidata structured entity data,
  [CC0](https://www.wikidata.org/wiki/Wikidata:Licensing). Unmodified downloaded
  entity JSON is retained in `wikidata/`; the catalog lists its retrieval URL,
  revision, country entity and SHA-256. The builder extracts the non-deprecated
  `P625` Earth coordinate, prefers preferred statements, rejects ambiguity and
  verifies `P17` country. Batch snapshots retain multiple entities in one file;
  `snapshot` identifies that file. An explicit `coordinate_statement` selects a
  reviewed claim when an entity has multiple eligible points. `label_language`
  identifies the source label when English is unavailable. It does not read prose or images as geographic data.
- Willoughby–Hart district focus: © NYC Landmarks Preservation Commission,
  [historic building data](https://www.nyc.gov/site/lpc/about/maps.page), under the
  [NYC Open Data terms](https://data.cityofnewyork.us/stories/s/Terms-of-Use/k9k7-3cje/).
  The unmodified `nyclpc/LP-2683.geojson` contains the queried district's 143
  building footprints. `municipal_sources` records its query URL and SHA-256.
  The builder validates district membership and EPSG:4326 coordinates, then
  derives the focus from their bounding-box midpoint. These polygons identify
  the focus only; production building geometry still comes from ordinary map
  sources. The authored radius is not the designated district boundary.
- Nyhavn and Andrássy corridor lines: © OpenStreetMap contributors,
  [ODbL 1.0](https://www.openstreetmap.org/copyright). Unmodified API snapshots
  are pinned in `osm/6476337-v12.json` and `osm/18747738-v2.json`; each corridor
  records its URL, relative snapshot path and SHA-256. Entry notes identify
  the canal-bank midpoint derivation or selected road endpoints. The compiler
  verifies snapshot bytes; the line derivation and authored widths remain
  reviewable catalog interpretation rather than surveyed district boundaries.
- Pimlico polygon: Westminster City Council's
  [conservation-area data](https://www.westminster.gov.uk/planning-building-control-and-environmental-regulations-menu/digital-planning-data),
  record `CONARA/1530`. Contains public sector information licensed under the
  [Open Government Licence v3.0](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/).
  `westminster/boundaries.source.json` records the raw CSV and derived GeoJSON
  checksums, EPSG:27700 interpretation, pinned datum conversion and its stated
  2 m accuracy. Both components and all 197 coordinate pairs are retained without
  simplification. Verify with `python3 tools/convert-westminster-architecture-boundaries.py --check`.
- Kraków and Warsaw Old Town, Prague Old Town and Malá Strana polygons: © OpenStreetMap contributors,
  [ODbL 1.0](https://www.openstreetmap.org/copyright). `osm/boundaries.source.json`
  pins the raw relation snapshots and derived GeoJSON hashes. These are mapped
  municipal information-system or cadastral areas, not asserted UNESCO property boundaries.
  `python3 tools/convert-osm-architecture-boundaries.py --check` reproduces the
  polygons in memory by joining exact shared node identities, retaining holes,
  disconnected components and all coordinates. Open or ambiguous chains fail.
  Run `python3 tools/test-osm-architecture-boundaries.py` for topology checks.
  Stockholm's city mask uses an authored union of the mapped Norrmalm,
  Vasastaden, Östermalm, eastern Kungsholmen and Södermalm districts. The same
  manifest pins all five OSM relations. The converter cancels identical shared
  boundary ways and retains every remaining node coordinate; it does not perform
  approximate polygon overlays. Gamla Stan, western Kungsholmen and Djurgården
  are outside this mask. This scoped apartment-stock prior is not a municipal
  map or a legal conservation area.
  Copenhagen and Athens use the complete mapped municipality relations pinned
  in the same manifest. Copenhagen retains all disconnected components and the
  Frederiksberg enclave hole; Nyhavn's narrow district remains the more specific
  recipe. Athens excludes neighboring municipalities such as Piraeus. These
  city masks constrain ordinary-stock priors, not metropolitan or heritage claims.
  Warsaw's point registration is also recorded in
  `wikidata/poland-identities.source.json`; the atlas verifies its pinned CC0
  snapshot, revision, country and selected coordinate statement.
- Amsterdam Binnen de Singelgracht polygon: Gemeente Amsterdam, Datateam
  Stedelijke Ontwikkeling en Beheer. The municipal dataset specifies
  [Creative Commons, Naamsvermelding](https://api.data.amsterdam.nl/v1/docs/datasets/beschermdestadsdorpsgezichten@v1.html).
  `amsterdam/boundaries.source.json` pins the CRS84 response and selected feature.
  `python3 tools/convert-amsterdam-architecture-boundaries.py --check` verifies
  identity, source properties and every coordinate without simplifying the boundary.
- Porto historic-center polygon: © OpenStreetMap contributors,
  [ODbL 1.0](https://www.openstreetmap.org/copyright). The complete relation
  `9745063`, revision 22, is pinned in `osm/boundaries.source.json` and checked by
  `python3 tools/convert-osm-architecture-boundaries.py --check`. Its 118 coordinates
  retain the mapped core, bridge and Serra do Pilar extent. Comparison with the
  national heritage map supports its geographic scope; an OSM member explicitly
  notes approximate historic-wall alignment, so this is not legal cadastral precision.
- Lisbon Baixa Pombalina polygon: Câmara Municipal de Lisboa, Departamento de
  Planeamento / Divisão de Gestão de Informação Georreferenciada. The municipal
  [planning-boundary dataset](https://dados.cm-lisboa.pt/dataset/limite-de-planos-de-pormenor)
  specifies [CC0](https://creativecommons.org/publicdomain/zero/1.0/).
  `lisbon/boundaries.source.json` pins the raw ArcGIS GeoJSON and effective plan
  `COD_SIG=140`. `python3 tools/convert-lisbon-architecture-boundaries.py --check`
  verifies its identity and all 248 coordinates. Selection preserves the full
  safeguarding-plan boundary and source properties, without reprojection or
  simplification; it does not substitute the broader Lisboa Pombalina heritage area.
- The Chartrons and Grünerløkka outlines in `authored-districts/` are authored
  residential-character boundaries joining selected OpenStreetMap street vertices;
  they are not administrative or legal conservation boundaries. Data ©
  [OpenStreetMap contributors](https://www.openstreetmap.org/copyright), ODbL 1.0.
  The manifest retains retrieval URLs, lossless raw responses, node anchors and
  packed/uncompressed hashes. Run
  `python3 tools/check-authored-architecture-districts.py --check` to verify
  both outlines against their source nodes.
- Helsinki central and Etu-Töölö masks: Data ja kartat (c) Helsingin kaupunki,
  kaupunkimittauspalvelut. The City's
  [open geographic data terms](https://www.hel.fi/fi/paatoksenteko-ja-hallinto/tietoa-helsingista/kartat-ja-paikkatieto/paikkatiedot-tehokayttoon/avoimet-paikkatiedot)
  specify [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
  `helsinki/sources.json` pins the official administrative-division API URL,
  response checksum, selected neighborhood identities, lossless selected source
  features and derived masks. The central mask joins nine complete neighborhoods;
  the Etu-Töölö district retains neighborhood `91-013`. These are authored
  architectural scopes based on municipal boundaries, not conservation designations.
  Union removes interior borders and exact duplicate consecutive vertices without
  coordinate transformation or simplification; holes are retained. With Shapely 2
  installed, `python3 tools/convert-helsinki-architecture-boundaries.py --check`
  verifies source hashes and reproduces both masks. Normal atlas builds need no
  Shapely dependency. The separate western Katajanokka study remains independent.
- Radii, interpretation, style selection, confidence and infill values:
  project-authored material under the repository `LICENSE`.
- Architectural references: individually linked government, conservation and
  UNESCO descriptions are research citations for facts. No photographs, prose,
  UNESCO XML/KML export or restricted bulk inventory are
  redistributed by this atlas. The references do not confer a surveyed
  neighborhood boundary on the authored envelope.

The application already credits GeoNames. Preserve that attribution and this
source record and NYC LPC credit when redistributing the derived data. Wikidata CC0 does not impose
an attribution obligation, but its source identities and hashes remain useful
for audit and regeneration.

## Rebuild and validation

Madrid's `madrid/centro.geojson` is the complete Centro district Feature from
Ayuntamiento de Madrid, reused under its [CC BY 4.0/general reuse conditions](https://datos.madrid.es/pages/condiciones-de-uso).
Milan's `milan/central-naf.geojson` is the central Nucleo di Antica Formazione
Feature from Comune di Milano's current PGT R04 service, CC BY 4.0. Its pinned
service metadata records the license. Both selections preserve all original
coordinates and properties; the architectural interpretations are authored,
and neither municipality endorses them. Source URLs, hashes, feature selectors
and retrieval dates are recorded in each directory's `boundaries.source.json`.

```sh
python3 tools/convert-feature-architecture-boundaries.py --check \
  crates/vearth-gen/assets/architecture/madrid/boundaries.source.json \
  crates/vearth-gen/assets/architecture/milan/boundaries.source.json
```

From the repository root, with the verified place-search artifact present:

```sh
python3 tools/build-architecture-places.py
python3 tools/build-architecture-places.py --check
python3 tools/test-architecture-places.py
cargo test -p vearth-gen architecture_places
```

The builder needs Python's standard library and `rustfmt`; it makes no network
requests. New GeoNames entries must name an existing source ID and exact country
and display name. New Wikidata entries need an entity JSON snapshot, its revision
and checksum, and the matching country entity. Do not replace a source coordinate
with a visually convenient camera coordinate. Add an appropriate source anchor
instead. Every style key must exist in the typed `ArchitectureStyle` catalog.

Tests compare indexed queries with exhaustive spherical queries around every
authored envelope, including overlap and edge samples; synthetic polar and
dateline caps and finite corridors verify indexing independently of the current
place distribution. Corridor tests sample side edges, control points, bends and
round endcaps; runtime tests reject the off-route interior of a bent line.
Catalog tests verify source hashes/identities, numeric bounds, unknown-style
rejection, deterministic overlaps and distinct requested local styles.

The Merrion Square corridor uses `osm/merrion-frontage-20260916.json`, a pinned
OpenStreetMap North/East/South road-node chain. The open U excludes the western
institutional frontage; the compiler verifies its snapshot hash. © OpenStreetMap
contributors, ODbL. Its authored widths describe a frontage study, not a protected
area boundary.
