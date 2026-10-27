# Data and attribution

Low Poly Earth combines bundled offline datasets with network providers. Data
licenses are independent of the repository's [MIT License](../LICENSE), which
covers project-authored code, documentation, and assets.
**Settings > Data sources** lists provider controls and their enabled state.
The separate attribution window lists all registered providers plus bundled
sky, map, climate, region, settlement, timezone, and place-search sources. Disabled
providers remain visible; this document covers both default and optional
sources.

## Runtime providers

| Layer | Source | Use | Terms |
| --- | --- | --- | --- |
| Global elevation | [Mapterhorn](https://mapterhorn.com/) | Bundled coarse fallback and streamed Terrarium tiles | Copernicus GLO-30 and national open-data elevation models; per-source licences and credits in [Mapterhorn's attribution](https://mapterhorn.com/attribution) and the terrain manifest |
| Global vectors | [OpenStreetMap](https://www.openstreetmap.org/copyright) via [Protomaps](https://protomaps.com/) | Buildings, roads, water, land cover, and map context | © OpenStreetMap contributors; ODbL 1.0 |

The bundled global elevation, monthly surface imagery, and place index retain useful offline
behavior, while fine vector and terrain detail require their providers.

Prepared bridge catalogs derive from raw OpenStreetMap extracts under the same
ODbL attribution. `vearth-prep pack-bridges` takes `--cache-dir`, `--build-tag`
and raw `--osm` inputs; cached zoom-14 vector tiles supply water masks. The
runtime reads `vectors/<pinned-build>/bridges.vbp` beneath its source cache.
Regional catalogs own only matched crossings; unmatched map ways retain their
fallback. Keep the extract identity, source hashes and OSM attribution beside
any generated catalog distributed separately.
The small Pont Neuf regression extract retains its source identity, license and
hashes in the [fixture provenance](../crates/vearth-data/testdata/pont-neuf-bridges.provenance.json).

## Bundled datasets

- **Distinctive water masks:** pinned OpenStreetMap lake outlines under ODbL
  1.0, paired with project-authored optical palettes. See the
  [water catalog sources](../crates/vearth-gen/assets/water/SOURCES.md).

- **Terrain:** a zoom 0–3 pack of 512 px tiles derived from Mapterhorn. See the
  [terrain manifest](../crates/vearth-data/assets/terrain-SOURCES.md).
- **Surface appearance:** all twelve 2004 monthly NASA Blue Marble: Next
  Generation mosaics, a public-domain U.S. Government work. See the
  [surface manifest](../crates/vearth-data/assets/surface-SOURCES.md).
- **Climate:** WorldClim 2.1 normals under CC BY 4.0, NOAA/NCEP Reanalysis
  public-domain normals, and a NASA Black Marble public-domain emissions
  proxy. See the
  [climate manifest](../crates/vearth-data/assets/climate-SOURCES.md).
- **Place search:** GeoNames data under CC BY 4.0 plus MIT-licensed
  project-authored metadata. See the
  [place-search manifest](../crates/vearth-data/assets/place-search-SOURCES.md).
- **Architectural place influences:** GeoNames coordinates under CC BY 4.0,
  CC0 Wikidata structured records, NYC LPC district data, and pinned OpenStreetMap
  street/canal geometry (© OpenStreetMap contributors, ODbL 1.0), with
  Westminster boundaries under the Open Government Licence v3.0,
  Amsterdam municipal protected-townscape data under the source's
  Creative Commons attribution terms, Lisbon planning boundaries under CC0,
  Madrid Centro, Milan PGT and Helsinki neighborhood boundaries under CC BY 4.0
  (Data ja kartat (c) Helsingin kaupunki, kaupunkimittauspalvelut.), and
  project-authored styles and transition widths. See the
  [architectural place manifest](../crates/vearth-gen/assets/architecture/places-SOURCES.md).
- **Administrative regions and map overlay:** Natural Earth public-domain
  Admin-1 and 1:50m map geometry. See the
  [region manifest](../crates/vearth-data/assets/admin-regions-SOURCES.md) and
  [map manifest](../crates/vearth-app/assets/mercator-map-SOURCE.md).
- **Settlement context:** European Commission/JRC GHS-SMOD R2023A V2-0,
  epoch 2020, under CC BY 4.0. The native kilometre grid is losslessly encoded;
  it describes settlement density, not building condition or household income.
  Dataset and release-paper citations are retained in the
  [settlement manifest](../crates/vearth-data/assets/settlement-SOURCES.md).
- **Timezones:** Timezone Boundary Builder under ODbL 1.0. See the
  [timezone manifest](../crates/vearth-data/assets/timezones-SOURCES.md).
- **Night sky:** the ESA Hipparcos catalogue under CC BY-NC 3.0 IGO
  (Credit: ESA; retained for noncommercial use), NASA Black Marble 2016 as a
  public-domain U.S. Government work, and the ESO/S. Brunier Milky Way panorama
  under CC BY 4.0.
  See the [sky manifest](../crates/vearth-app/assets/sky/SOURCES.md).
- **Landmarks:** a mixture of project-authored assets, owner-supplied source
  models, MIT material, CC BY 4.0 models, and CC BY-SA 4.0 models. Attribution,
  authorship, transformations, and derived-output terms are recorded per model
  in the [landmark manifest](../crates/vearth-gen/assets/landmarks/SOURCES.md).
  The Sydney Opera House source model is project-owner-authored and covered by
  the repository's MIT License. Runtime landmark assets are indexed `LMS1` triangle
  meshes. Their [source manifests](../crates/vearth-gen/assets/landmarks/surfaces/)
  record source identities, hashes, transformations and output hashes. Original
  source geometry supplies the exported LODs; the documented Merlion exception
  is a permanent offline preservation bake of its verified shipped archive.
  Handcrafted city models have [separate source manifests](../crates/vearth-gen/assets/landmarks/city-surfaces/catalog.json)
  for the checked-in geometry generators. Their references supply dimensions and
  placement, with no external geometry or image imports. Finite facade fixtures
  retain [OpenStreetMap placement attribution](../crates/vearth-gen/assets/cities/signs.json).
  Runtime has no voxel landmark decoder. Taj Mahal and Sagrada Familia are not
  in the active registry; their recorded notices are retained where applicable.

- **Great Wall:** the pinned nationwide OpenStreetMap relation and derived geographic
  network are under ODbL 1.0. The [source manifest](../crates/vearth-gen/assets/great-wall/SOURCES.md)
  records reproducibility, coverage, dimension/material inference and attribution.

## Fonts and website code

The application embeds IBM Plex and Noto Sans CJK fonts under the SIL Open Font
License 1.1. Their source revisions and license-file mappings are recorded in
the [font manifest](../crates/vearth-app/assets/fonts/README.md). The website
serves a Latin subset (WOFF, via `pyftsubset`) of the pinned Archivo Black
arrival-title font, with its OFL text at [`site/fonts/archivo-black-OFL.txt`](../site/fonts/archivo-black-OFL.txt).

## Release artifacts

The macOS packaging script copies the repository's MIT `LICENSE`,
the Statue of Liberty source's MIT license, both font OFL texts, and a generated
`THIRD_PARTY_NOTICES.md` into the app's `Contents/Resources` directory. Package
verification byte-compares the license files and regenerates the notice
document from the source-local asset manifests and the checked-in
[Cargo dependency notices](../packaging/licenses/CARGO_DEPENDENCY_NOTICES.md).
Those notices cover the locked Apple silicon app's default features, including
build dependencies and embedded dependency fonts and data. Packaging rejects
stale dependency notices. See [Development](DEVELOPMENT.md) for regeneration.

The intended app release is noncommercial. Hipparcos is retained under ESA's stated
CC BY-NC 3.0 IGO terms. Preserve its credit, license link, and adaptation notice.
Commercial use requires separate permission or a replacement dataset. Any
distribution terms or technical restrictions must preserve the licensed data's
rights; recording these notices does not establish App Store terms compatibility.

Arrival-title fonts are full, unmodified SIL Open Font License files pinned in
`crates/vearth-app/assets/fonts/titles/manifest.json`. Verify them with
`python3 tools/verify-title-fonts.py`; macOS packages include every family license
in `Resources/TitleFontLicenses/`. Regional art direction and native-name fallback
rules are documented in `docs/LOCATION_TITLES.md`.
