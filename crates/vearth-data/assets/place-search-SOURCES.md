# Offline place-search sources

`place-search-v1.bin` is a deterministic derived database for the application's
offline place search with English-preferred names. The canonical build uses
the GeoNames exports dated 2026-07-31 and project-authored curated metadata.
The generated file is ignored because of its size but is compiled into local
application builds.

## License and attribution

- GeoNames data: © GeoNames, licensed under
  [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/).
  Source: <https://download.geonames.org/export/dump/>.
  Provider information: <https://www.geonames.org/about.html>.
- `curated-places.tsv`: project-authored metadata under the repository's MIT
  `LICENSE`. Rows that name a
  `source_landmark` take their coordinates directly from
  `vearth_gen::landmarks::LANDMARKS`.

The application displays “GeoNames (CC BY 4.0)” in its attribution panel.

## 2026-07-31 input checksums

| Input | SHA-256 |
|---|---|
| `cities500.zip` | `b7de544e4ac15e830c9128108386dec4fc94bee2a3ce8d81455b38e14c05acec` |
| `allCountries.zip` | `04f7455db02514b5fbe8a5aff3b9253182dfaa98200461a49bc70d7478972740` |
| `alternateNamesV2.zip` | `b32123ab3f9fee9f58a7133f7506a8ad05488676bfaef9fc4a2a5671f01f071d` |
| `countryInfo.txt` | `93bafc525813f22e4711ff9ed6d626343094ce48c26388dc7c49189b3d7d5512` |
| `admin1CodesASCII.txt` | `34784457b76b988a669dff7c3e4b104e4902c0875643cff019281ac79dfa2992` |

Output SHA-256:
`ad26b5cca5f9fcede3843ba34f35996da7fd76f2c1ca1ec3dbc76aff9849cb6d`.

## Selection and names

The full build includes:

- every `cities500` populated place;
- every feature-class `P` record with a nonzero population;
- every national or administrative seat (`PPLC`, `PPLA`, `PPLA2`, `PPLA3`,
  and `PPLA4`);
- every active `PPLL` inhabited locality and `PPLF` farm village;
- a deterministic 25% country-salted sample of the remaining active,
  zero-population `PPL` long tail; and
- all entries in `vearth_gen::landmarks::LANDMARKS` plus the checked-in
  manually curated rows.

`PPLQ` abandoned places, `PPLW` destroyed places, `PPLH` historical places,
and subdivision-only `PPLX` records are excluded. Street addresses,
businesses, and arbitrary OSM points of interest are not inputs.

English `alternateNamesV2` values are retained only for the rich tier
(`cities500`, population-bearing places, and seats). Historic and colloquial
names are discarded. A preferred English name is the display name; otherwise
the first ordinary English name is used. If neither exists, GeoNames' primary
name is used, falling back to the ASCII name only when the primary is empty.
Primary and ASCII forms remain searchable when they differ. The compact long
tail stores only primary and ASCII forms.

## Rebuild

Download the five inputs above, extract the three ZIP archives, then run from
the repository root:

```sh
cargo run -p vearth-prep --release -- pack-place-search \
  --cities /path/to/cities500.txt \
  --all-countries /path/to/allCountries.txt \
  --alternate-names /path/to/alternateNamesV2.txt \
  --country-info /path/to/countryInfo.txt \
  --admin1 /path/to/admin1CodesASCII.txt \
  --dataset-date 2026-07-31
```

Omit `--all-countries` for a smaller representative primary-tier artifact.
`--long-tail-sample-permyriad` changes the default `2500` sampling threshold;
`--include-subdivisions` opts into `PPLX`, and `--curated` selects a different
optional curated TSV. A missing curated TSV is nonfatal; the generator still
imports `LANDMARKS`. The encoder rejects output above 150 MiB.

On a fresh checkout, create an empty placeholder only to let the `vearth-prep`
dependency graph compile:

```sh
touch crates/vearth-data/assets/place-search-v1.bin
```

The `pack-place-search` command above must replace that placeholder before
building or testing the application.

## Canonical artifact

- Records: 1,777,018
- Search aliases: 1,816,848
- Installed bytes: 119,670,512 (114.13 MiB)
- Format: `VEARTHPS` version 1
- Coordinate encoding: signed degrees × 10⁷ (`i32`)
- Runtime ownership: immutable compile-time bytes; no dataset-sized heap copy
