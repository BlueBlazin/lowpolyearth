# Bundled global surface appearance

`surface-01.vsp` through `surface-12.vsp` are deterministic, offline-prepared
monthly global appearance packs. The application reads them directly from
their bundled bytes and performs no HTTP requests for global terrain colour.
The simulated UTC month selects the corresponding atlas; only tiles sampled
by the current view are decoded.

## Source

- Dataset: NASA Blue Marble: Next Generation, all twelve monthly 2004
  surface-only global mosaics.
- Source resolution: 21,600 × 10,800 (NASA labels the global product
  2 km/pixel; the underlying monthly product is derived from 500 m MODIS data).
- URL pattern:
  `https://assets.science.nasa.gov/content/dam/science/esd/eo/images/bmng/bmng-base/{month-name}/world.2004{month-number}.3x21600x10800.jpg`
- Source SHA-256, January through December:
  - `c2f2c80a352518cfea23d462dad6f6e778375ddd74bbec5786bcfc85cc49c61d`
  - `8800081f38b53bfa1e3f0c2fa17be694b7646942fe5add009ea0dcd64c03e70b`
  - `af37f61a9715c59dc86763df19f588a20c9d92b3d0a8034a0b00acd6b97ccb7d`
  - `3bd13f173555eb24451c45d3a83ca177baca02e9a8bdd9fa3e4989f7b496ce6b`
  - `0de40c868680ab9e86fef42406d76386b243b32444ff50edfd406b296638c206`
  - `282608580a290723655bfbe917e69ea2f93f4da068dfa96ff4a152b1c6ec36be`
  - `dea8b4dc8a4f93f5f8bce0c8c85a508a178e7901e9ed8e6bf86e6ce7ef6d61e2`
  - `926be7253be90a1fd57071a33afbfe7a712a2dfe0d5b5b4602bc5dfbcfdc2c89`
  - `7cf788e13a3a7b4a926b524f8f71d635c56a18c48ba0bfe35695b05a305db3f3`
  - `bf568ceca2ad55e9c8dd7bcedfd0e419201d03bb560206a7bfa2cc57c1e04c64`
  - `1959bbe72897d3e9526c440dae3ec04cfdbdc4080c8ff0b14632925ae31167b1`
  - `b0ffc9d12205689aeaec66116bbae2c4c263fcd98a0cbadba8d5488567b53278`
- Attribution: NASA Earth Observatory / Blue Marble: Next Generation.
- License/status: public domain U.S. Government work.

The surface-only product is deliberate. The renderer supplies terrain relief
and lighting dynamically, so a version with baked topography or bathymetry
would double-light slopes and treat shadow as albedo.

## Preparation

Each monthly pack contains a 5,400 × 2,700 coarse level and the full 21,600 ×
10,800 regional level. All levels are split into row-major 512 × 512 JPEG tiles
(edge tiles are cropped), encoded at quality 64, indexed, and concatenated.
Independent tiles keep runtime decoded residency bounded instead of expanding
even one source JPEG to roughly 667 MiB at startup. Separate monthly files
avoid decoding unrelated months and keep asset granularity bounded.

```sh
cargo run -p vearth-prep --release -- \
  pack-surface \
  --input-dir /path/to/monthly-blue-marble \
  --quality 64 \
  --output-dir crates/vearth-data/assets
```

Output SHA-256, January through December:

- `3e91ac682d7d39a498bd52a5bb3042db3777ac5a6b9072bce505280a086bead1`
- `db5aca08e30dd8fdad742611acafc360076f3b64aa083df1f26ffa574bc3878f`
- `dae3e197e00b186a53d8bbbbf465db4cb951e57578e59f980a3ac20ee8bee2ea`
- `46a0a37277114341c268e8176c745c3ae5e42445696d28035e4c8f2bc138d5f5`
- `fb835ced719d2141b46512696813242a19523306da76f85dada38982129d89e4`
- `468c7d89fad1dd8a98187626af5e12a306c4d65e2e26a5af52e8576232bdaedf`
- `4fde8e8d4488176c047c28ce1525db6ae8d7bb1ca9d4c4424b01753a6311107f`
- `c7e08f3c1b5cb8d7b1d711722969c034a6ffc35dc65ec9069ee933b91fec338d`
- `bb93f69b94f7d32ffdd2a8de566f55c0fd8350faecba8f17463c6a0a34cceeca`
- `7e03db48b9eeb1d145bc3ea8e7f55e81b4da030702c9b8bc70785c3013de9cbc`
- `d9c0c2e29fae8ef1ff9c131608721f60345ba4eac84d7ad2f83040ef8f4180dc`
- `ebeeffcec2e3967ed0fba0b8ad245a673803f6d9e5197ebb2a5a60701ac7dd85`

Combined output size: 198,844,444 bytes (189.63 MiB).
