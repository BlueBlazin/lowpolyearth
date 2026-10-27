# Global settlement atlas provenance

`settlement-2020-v1.bin` losslessly repacks the **GHS-SMOD R2023A V2-0**
settlement classification for **2020**, published by the European Commission,
Joint Research Centre. It retains all 649,476,000 pixels of the native
36,082 × 18,000, 1 km **ESRI:54009 Mollweide** grid. Coordinates are projected
at lookup; the data are neither resampled nor interpolated.

## Attribution and terms

© European Union. Licensed under
[Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/).
See the [GHSL use conditions](https://human-settlement.emergency.copernicus.eu/GHSLhowToCite.php).
Vearth changes storage encoding only. No endorsement by the European Commission
or the authors is implied.

Required dataset citation:

Schiavina, M.; Melchiorri, M.; Pesaresi, M. (2023): **GHS-SMOD R2023A —
GHS settlement layers, application of the Degree of Urbanisation methodology
(stage I) to GHS-POP R2023A and GHS-BUILT-S R2023A, multitemporal (1975–2030)**.
European Commission, Joint Research Centre (JRC).
[DOI: 10.2905/A0DF7A6F-49DE-46EA-9BDE-563437A6E2BA](https://doi.org/10.2905/A0DF7A6F-49DE-46EA-9BDE-563437A6E2BA).

Required release publication:

Pesaresi, M., Schiavina, M., Politis, P., Freire, S., Krasnodębska, K.,
Uhl, J. H., et al. (2024). **Advances on the Global Human Settlement Layer
by joint assessment of Earth Observation and population survey data**.
*International Journal of Digital Earth*, 17(1).
[DOI: 10.1080/17538947.2024.2390454](https://doi.org/10.1080/17538947.2024.2390454).

## Source and interpretation

- [Official archive](https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/GHSL/GHS_SMOD_GLOBE_R2023A/GHS_SMOD_E2020_GLOBE_R2023A_54009_1000/V2-0/GHS_SMOD_E2020_GLOBE_R2023A_54009_1000_V2_0.zip).
- Archive SHA-256:
  `5a81c3827c9bbc9109159b4c3f92ac7722705944e43038219f142b410431a852`.
- Source GeoTIFF SHA-256:
  `1e2d7d7a6994b0458fc5d6ff985fb872f27841162b1808a3d52e32ae1a4953a0`.
- Output SHA-256:
  `c1a0829bcc916f41307346537befa1b2cfab5d7df9fc359a5c1464d9ebe63e4a`.
- Machine-readable identity, dimensions and exhaustive category counts:
  `settlement-2020-v1.source.json`.

The [JRC definitions](https://human-settlement.emergency.copernicus.eu/degurbaDefinitions.php)
classify density, settlement size and contiguity. The runtime groups codes
11/12 as rural, 13 as village, 21 as suburban, 22/23 as town and 30 as city.
Water (10), no-data (-200), invalid coordinates and points outside the source
polar extent yield unknown context. Raw code access retains all source values.
The atlas does not identify building use, material, construction year, condition,
household income or a particular architectural tradition. It is a dated
kilometre-scale settlement cue, not a parcel-level assertion.

## Rebuild and verify

Use Python with `numpy`, `rasterio`, and `pyproj` in an isolated environment;
`pyproj` is needed only by the independent source verifier. Extract the official
archive, then run:

```sh
python3 tools/build-settlement-atlas.py \
  /path/to/GHS_SMOD_E2020_GLOBE_R2023A_54009_1000_V2_0.tif \
  crates/vearth-data/assets/settlement-2020-v1.bin \
  --manifest crates/vearth-data/assets/settlement-2020-v1.source.json
python3 tools/test-settlement-atlas.py \
  --source /path/to/GHS_SMOD_E2020_GLOBE_R2023A_54009_1000_V2_0.tif
cargo test -p vearth-data --release settlement
```

The builder verifies every encoded row against its input. The source verifier
checks hashes, the GDAL/PROJ reference locations and 10,000 deterministic global
samples. Rust tests validate all row-run boundaries and category totals, and
compare native geographic lookups to the independent reference fixture in
`testdata/settlement-reference-samples.json`. Runtime searches the immutable
row index directly, without allocating or decompressing a dense global raster.
