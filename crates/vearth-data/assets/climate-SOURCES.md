# Climate Atlas v2 provenance

`climate-atlas-v2.bin.z` is a deterministic, derived one-degree monthly
climatology. The application ships this asset and makes no runtime weather,
climate, or air-quality requests.

## Inputs

### WorldClim 2.1

- Release: WorldClim 2.1, 10-minute monthly climate normals (1970–2000).
- Variables: average temperature (`tavg`), vapour pressure (`vapr`) and total
  precipitation (`prec`).
- URLs:
  - `https://geodata.ucdavis.edu/climate/worldclim/2_1/base/wc2.1_10m_tavg.zip`
  - `https://geodata.ucdavis.edu/climate/worldclim/2_1/base/wc2.1_10m_vapr.zip`
  - `https://geodata.ucdavis.edu/climate/worldclim/2_1/base/wc2.1_10m_prec.zip`
- License: [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/).
- SHA-256: `5e567dcfe94379b94229492849ce91078b1c6e5210aaf435fba449fae6b95405`
  (`tavg`), `0501b5b5cbbc6e2fc2a3801efa9a126a5f9669f0f9a00642d2519c2324ce726d`
  (`vapr`), `1090f578f6402672a2b6438e252156f6cf867a25002a8ad6ffcbfa19e516ddc6`
  (`prec`).

Land values are aggregated by the mean of each 6×6 source block. Missing
ocean cells receive broad continuous ocean normals before coastal runtime
interpolation.

### NOAA/NCEP reanalysis normals

- Release: NCEP/NCAR Reanalysis 1, 1991–2020 monthly long-term means,
  distributed by NOAA Physical Sciences Laboratory.
- Variables: 10 m U/V wind; 850, 700 and 300 hPa U/V wind; 850, 700 and
  300 hPa relative humidity.
- URLs:
  - `https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/surface/uwnd.10m.mon.1991-2020.ltm.nc`
  - `https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/surface/vwnd.10m.mon.1991-2020.ltm.nc`
  - `https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/pressure/uwnd.mon.1991-2020.ltm.nc`
  - `https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/pressure/vwnd.mon.1991-2020.ltm.nc`
  - `https://downloads.psl.noaa.gov/Datasets/ncep.reanalysis.derived/pressure/rhum.mon.1991-2020.ltm.nc`
- NOAA data are public domain; attribution to NOAA PSL and NCEP/NCAR
  Reanalysis is retained here.
- SHA-256, in URL order:
  - `45b9eadc8181b29cb9ebcbcc21503867bc69ec1272ac925d9dcf7f3b1cc18875`
  - `78130fd7e5c6f5dcec0ce0009d93639afd9645d77ace60d14c13581ac763fd0b`
  - `39996c7f68ebbe852109990bacda3af5a9df61d398d52b579fcec63727c60b16`
  - `7ca58b72fc39bce5754c9dca4f90cd1f58830f4edf82bd1e54f8f22af4c28915`
  - `ac53cc64674bacdb36d3a468df5d68c831d35f931452a9d487df7ee9114acfbc`

Source grids are bilinearly resampled in latitude/longitude with periodic
longitude wrapping. Pressure levels are selected by their coordinate value,
not array position.

### NASA Black Marble proxy

The already bundled and attributed NASA Black Marble 2016 upward-emission
proxy is used only to distribute a broad anthropogenic aerosol baseline. It is
not treated as measured aerosol optical depth. SHA-256 of the input asset:
`2c96dd8c098aa378d6f878bc32f960c356fefc2510e8fdfee24fff52b56385ed`.

## Derived fields and limitations

Cloud fractions are calibrated from pressure-level humidity and monthly
precipitation normals. Wet-hour probability is calibrated from monthly
precipitation normals, lower-tropospheric saturation, and a bounded empirical
occurrence curve; it is a climatological frequency model, not a claim that
WorldClim contains hourly observations. The instantaneous deterministic storm
field preserves that monthly frequency and derives its conditional rate so
long-term accumulation remains tied to the monthly precipitation normal.

Aerosol is an intentionally broad art-direction baseline using night-light
emissions, dry mineral-dust susceptibility, and regional pollution domes. It
is suitable for persistent differences such as Sydney versus the
Indo-Gangetic Basin, not live AQI or a historical reconstruction.

## Rebuild

Run `tools/build-climate-atlas.py` with Python 3 plus NumPy, Pillow and h5py:

```sh
python3 tools/build-climate-atlas.py \
  --worldclim /path/to/worldclim-unpacked \
  --uwnd-surface /path/to/uwnd.10m.mon.1991-2020.ltm.nc \
  --vwnd-surface /path/to/vwnd.10m.mon.1991-2020.ltm.nc \
  --uwnd-pressure /path/to/uwnd.mon.1991-2020.ltm.nc \
  --vwnd-pressure /path/to/vwnd.mon.1991-2020.ltm.nc \
  --rhum-pressure /path/to/rhum.mon.1991-2020.ltm.nc \
  --night-lights crates/vearth-app/assets/sky/black_marble_2048x1024.bin \
  --output crates/vearth-data/assets/climate-atlas-v2.bin.z
```

Output SHA-256:
`7da00684f1a649c1677a3b84be56ebb08d7fd9694c05d1bab238f5f1cb0a6116`.
