# Administrative region atlas provenance

`admin-regions-v1.bin.z` is a deterministic offline point-in-polygon index
derived from Natural Earth 5.1.1 Admin-1 states and provinces.

- Source: Natural Earth 1:10m Cultural Vectors, Admin-1 States and Provinces.
- Project: https://www.naturalearthdata.com/
- Repository:
  https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_10m_admin_1_states_provinces.geojson
- License: public domain.
- Runtime fields: preferred English region name with the source name as a
  fallback, country name, Natural Earth stable identifier, bounding box and
  polygon rings. The asset contains 4,589 records across 253 country and
  territory names; these are source categories, not a count of sovereign states.
- Source SHA-256:
  `22d0e3ad85eb3e27f17cabf8ba2d50e554fbc27a87796ff891d958185da62fb5`.

Rebuild:

```sh
python3 tools/build-admin-regions.py \
  /path/to/ne_10m_admin_1_states_provinces.geojson \
  crates/vearth-data/assets/admin-regions-v1.bin.z
```

The runtime uses this atlas for geographic naming and regional generation.
The borrowed context API exposes source metadata; the location-label API keeps
country names internal for suppression and context matching. A memory-only
two-degree index narrows candidates before exact polygon tests. Uncovered
coastlines and ocean have no containing context; they are not assigned to an
unbounded nearest country. This is not a legal or cadastral boundary authority.

Output SHA-256:
`9881d8f2097f937f0946232efef3d6d12b49702bb7f3ee24f952a8a21e006b06`.
