# Timezone atlas provenance

`timezones-2026c.bin` is a deterministic, bounded-memory point-in-polygon
index derived from Timezone Boundary Builder release 2026c.

- Source: `timezones-with-oceans-1970.geojson.zip`.
- Project: https://github.com/evansiroky/timezone-boundary-builder
- Release: https://github.com/evansiroky/timezone-boundary-builder/releases/tag/2026c
- License: [Open Data Commons Open Database License (ODbL) 1.0](https://opendatacommons.org/licenses/odbl/1-0/).
- Runtime fields: IANA timezone identifier, bounding box, and polygon rings.
- Source SHA-256:
  `fbf725dec442c4f990f351800f6c450b06925bfb127672972a7e535fb0e59797`.

Coordinates retain the source's six decimal places. Polygon rings are delta
and varint encoded, then compressed independently. The application retains
only the small name/bounding-box index; a lookup decompresses candidate
polygon blocks without materialising the world atlas.

Rebuild:

```sh
python3 tools/build-timezones.py \
  /path/to/timezones-with-oceans-1970.geojson.zip \
  crates/vearth-data/assets/timezones-2026c.bin
```

Output SHA-256:
`6b3323de8cea5b012e5f4def0caf5aaaa58b66c0ca6424085400518bdabf1f2b`.
