# Distinctive water optical masks

Geometry © OpenStreetMap contributors, under the Open Database License
(ODbL) 1.0. Attribution and terms: <https://www.openstreetmap.org/copyright>.
The pinned source responses, feature URLs, versions and uncompressed SHA-256
hashes are recorded in `catalog.json`. Derived masks are also ODbL 1.0.

`tools/build-water-catalog.py` joins multipolygon members, retains holes and
simplifies footprints in local metres. `--check` verifies source hashes,
closure, simplification error and the generated Rust data. Source responses
are retained in `sources/` so regeneration needs no network access.

Optical palettes and coefficients are project-authored approximations. Their
references establish color character; no reference photographs or article text
are redistributed. The catalog's `temporal` fields qualify representative
appearances that vary in the real world.

- Hutt Lagoon: [Tourism Australia](https://www.australia.com/en/things-to-do/nature-and-national-parks/australias-pink-lakes.html).
- Laguna Colorada: [NASA Earth Observatory](https://science.nasa.gov/earth/earth-observatory/laguna-colorada-86135/).
- Lake Natron: [NASA Earth Observatory](https://science.nasa.gov/earth/earth-observatory/lake-natron-tanzania-6446/).
- Lake Pūkaki and Tekapo: [Te Ara, New Zealand](https://teara.govt.nz/en/photograph/10732/glacial-lakes).
- Moraine Lake and Lake Louise: [Parks Canada](https://parks.canada.ca/pn-np/ab/banff/visit/les10-top10/louise).
- Blue Lake/Warwar: [South Australian Tourism Commission](https://southaustralia.com/guides/limestone-coast-natural-wonders-bucket-list).
