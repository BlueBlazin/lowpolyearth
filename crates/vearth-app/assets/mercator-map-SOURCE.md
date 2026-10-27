# Mercator map asset

`mercator-map.bin` is built from Natural Earth's public-domain 1:50m vector
data:

- [Admin 0 — Countries](https://www.naturalearthdata.com/downloads/50m-cultural-vectors/50m-admin-0-countries-2/)
- [Lakes](https://www.naturalearthdata.com/downloads/50m-physical-vectors/50m-lakes/)
- [Natural Earth terms of use](https://www.naturalearthdata.com/about/terms-of-use/)

The source geometry is projected by `tools/build_mercator_map.py` into an exact
Web Mercator world spanning ±85.05112878° latitude. The compact binary contains
normalized polygon paths rather than a pre-rasterized image. The app
tessellates those paths into static GPU buffers, then pans and zooms by updating
a transform uniform.

Rebuild from the repository root after obtaining or converting the two source
datasets to GeoJSON:

```sh
python3 tools/build_mercator_map.py \
  /path/to/ne_50m_admin_0_countries.geojson \
  /path/to/ne_50m_lakes.geojson \
  crates/vearth-app/assets/mercator-map.bin
```

The exact input GeoJSON hashes were not retained for the current output. The
output is verifiable, but this record alone does not guarantee a byte-identical
rebuild if the upstream downloads or conversion change.

Output SHA-256:
`8eafcb2a1a28a1071bbcc3d7fc45f064d56a9fabdcdf68f5180e5e1fca02d369`.
