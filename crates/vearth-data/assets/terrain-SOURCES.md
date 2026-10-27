# Bundled terrain provenance

`terrain-native.vtp` is the application's network-free global elevation
fallback. It is derived from [Mapterhorn](https://mapterhorn.com/) terrain
tiles in Terrarium encoding, release 0.0.13, retrieved on 2026-09-26 from
`https://tiles.mapterhorn.com/{z}/{x}/{y}.webp`.

- License and attribution: Mapterhorn's planet archive merges Copernicus
  GLO-30 with regional open elevation models. Each source's licence and credit
  is listed in [Mapterhorn's attribution](https://mapterhorn.com/attribution)
  and its machine-readable
  [attribution.json](https://download.mapterhorn.com/attribution.json).
- Copernicus GLO-30: © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH
  2014-2018, provided under COPERNICUS by the European Union and ESA; all
  rights reserved. Distributed under the Copernicus full, free and open
  licence.
- Runtime coverage: complete Web Mercator tiles from zoom 0 through zoom 3
  (85 tiles of 512×512). Mapterhorn omits tiles with no land; `3/0/0`,
  `3/7/0` and `3/1/5` were absent upstream and are encoded as open sea.
- Mapterhorn has no bathymetry. Open sea, stored upstream as exactly 0 m, is
  encoded at the runtime's nominal sea floor (−30 m) by the same decoder the
  app uses.

The checked-in pack uses 512×512 tiles, one-metre integer height quantization,
row delta coding, and independent zlib-compressed payloads. Finer terrain is
resolved through the normal provider chain and is not stored in this file.

Rebuild into a local cache, then pack:

```sh
cargo run -p vearth-prep --release -- --max-zoom 3 --cache-dir cache
cargo run -p vearth-prep --release -- pack-terrain --cache-dir cache
```

`terrain-native.inputs.sha256` lists the SHA-256 of every input tile
(`cache/mapterhorn/{z}/{x}/{y}.webp`) and of the empty `.sea` markers written
for absent tiles. The tile endpoint serves Mapterhorn's current release, so a
rebuild after a newer release can differ; compare against that list first.

Output size: 9,797,585 bytes. Output SHA-256:
`0fd1ddbfd2f41141915b7087bbb94a5fc54e1266b96e4bc43a8c24cb035afe33`.
