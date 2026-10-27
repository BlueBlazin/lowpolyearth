# Great Wall network

© OpenStreetMap contributors. The pinned source database and derived `network.gwl`
are available under [ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/).
[OpenStreetMap copyright](https://www.openstreetmap.org/copyright).

`osm-relation-318110.json.gz` retains the recursive source extract, including its
OSM timestamp, tags, source IDs and geometry. `osm-nearby-towers.json.gz`
adds defensive towers within 35 m of those routes, including towers omitted
from relation membership. `manifest.json` records the exact
query, source/output hashes, coverage and dimension/material inference rules.
The source is the nationwide [Great Wall relation](https://www.openstreetmap.org/relation/318110),
including disconnected branches and remains. It does not establish exhaustive
coverage of every historical kilometre. Ditches are not turned into walls.

Rebuild: `python3 tools/build_great_wall.py`. Verify without writing:
`python3 tools/build_great_wall.py --check`. To deliberately replace the pinned
source, pass `--import-source PATH_TO_OVERPASS_JSON` and review the generated
manifest and geometry changes. `--import-towers PATH_TO_OVERPASS_JSON` updates
the companion tower extract. Retain this attribution with derived databases.

The generator stores WGS84 coordinates and metre-scale dimensions. Runtime
spatial bins select nearby records; canonical terrain stations, mesh clipping
and collision share one geographic geometry. No global bounding-box terrain
flattening, national LOD lock or animation is used. Polygon tower dimensions
come from their mapped footprints; point-only towers and architectural details
are approximations. Narrow wall-area outlines are reduced to paired-boundary
centre lines; wider fortifications retain their perimeter.

Masonry defaults are informed by the
[Beijing tourism description of Badaling](https://english.visitbeijing.com.cn/article/47OOiKK1Kj8):
6–9 m height, 6.5–7.5 m base and 4.5–5.8 m top width. Tagged dimensions take
precedence within sanity bounds. The distinction between eastern masonry and
western earth construction is described in the
[Chinese heritage authority's UNESCO submission](https://whc.unesco.org/document/157507).
Untagged historical condition and dimensions remain approximations, not surveys.

Visual verification: `cargo build --release -p vearth-app --bin vearth-app --bin city-survey`,
then `python3 tools/capture_great_wall.py`. Captures retain camera coordinates,
source/binary hashes and the application's streaming report beside each PNG.
