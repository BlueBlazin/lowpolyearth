# Night-sky asset sources

The three runtime files in this directory are deterministic outputs of:

```sh
cargo run -p vearth-prep --release -- pack-sky-data \
  --hipparcos /path/to/hip_main.dat \
  --black-marble /path/to/BlackMarble_2016_01deg.jpg \
  --milky-way /path/to/eso0932a.jpg
```

## `hipparcos_mag78.bin`

- Source: ESA (1997), *The Hipparcos and Tycho Catalogues*, Hipparcos Main
  Catalogue `I/239/hip_main`.
- Archive: <https://cdsarc.cds.unistra.fr/ftp/cats/I/239/hip_main.dat>
- Field definition: <https://cdsarc.cds.unistra.fr/viz-bin/ReadMe/I/239>
- License: [CC BY-NC 3.0 IGO](https://creativecommons.org/licenses/by-nc/3.0/igo/),
  as stated on [ESA's catalogue page](https://www.cosmos.esa.int/web/hipparcos/catalogues).
- Required credit: **Credit: ESA**. Catalogue reference: ESA, 1997,
  *The Hipparcos and Tycho Catalogues*, ESA SP-1200.
- The bundled subset is adapted from the catalogue using the selection,
  coordinate conversion, default colour, and binary encoding described below.
  The source and derived data are retained for noncommercial use. Commercial
  distribution requires separate permission or replacement of this dataset.
  Preserve the credit, licence link, and description of changes; do not apply
  additional restrictions to the licensed data or imply ESA endorsement.
- Selection: every entry with Johnson V magnitude ≤7.8 and valid ICRS right
  ascension/declination (34,491 entries).
- Stored fields: inertial unit direction, Johnson V magnitude, Johnson B−V
  color. Missing B−V values use 0.65 (sun-like).
- Runtime size: about 674 KiB. Coordinates remain in the catalog's ICRS-like
  inertial frame and rotate into ECEF from simulation sidereal time. Entries
  are sorted by magnitude so the renderer can submit only the prefix that the
  current limiting-magnitude budget could reveal.
- Output SHA-256:
  `b7e8ada29ad887f46f61244f216cc1d06693ef4e60b7f45a4ba5f57f51944921`.

## `black_marble_2048x1024.bin`

- Source: NASA Earth Observatory, *Earth at Night (Black Marble) 2016 Color
  Maps*, produced from Suomi NPP VIIRS observations.
- Source page: <https://science.nasa.gov/earth/earth-observatory/earth-at-night/maps/>
- Input: the official 3600×1800 0.1-degree equirectangular JPEG.
- Processing: isolates neutral/warm upward emission from the presentation
  image's dim blue land/ice layer, applies a bounded logarithmic compression,
  then Gaussian-downsamples to a 2048×1024 unsigned-byte equirectangular grid.
- Runtime size: 2 MiB. The CPU samples a 20 km Gaussian neighborhood and eight
  azimuth sectors out to 100 km; the image is not uploaded as a GPU texture.
- License/status: public-domain U.S. Government work.
- Output SHA-256:
  `2c96dd8c098aa378d6f878bc32f960c356fefc2510e8fdfee24fff52b56385ed`.

NASA imagery should be credited to NASA Earth Observatory / VIIRS. The
satellite map is an emission proxy: VIIRS observes upward radiance, while the
runtime's conversion to observer sky brightness is an explicit visual
propagation model rather than a claim that the asset directly measures Bortle
class or limiting magnitude.

## `milky_way_1024x512.bin`

- Source: ESO/S. Brunier, *The Milky Way panorama* (`eso0932a`).
- Source page: <https://www.eso.org/public/images/eso0932a/>
- Input: ESO's official 6000×3000 large JPEG, a complete 2:1 Galactic
  panorama with the Galactic plane horizontal and centre at the image centre.
- Input SHA-256:
  `60400c92c54b7c1bd12299c69e83b16e5b6256e7dabacc478c021758ecd28179`.
- Processing: Gaussian-downsamples to 1024×512 RGBA and applies a small
  diffuse blur. This preserves the Galactic bulge, dust lanes, and satellite
  galaxies while softening photographic point stars beneath the authoritative
  Hipparcos pass.
- Runtime size: 2 MiB. The GPU samples it in Galactic coordinates after
  undoing sidereal rotation from ECEF.
- License: [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/).
  Required image credit: **ESO/S. Brunier**. The app presents this credit in
  its attribution window.
- Output SHA-256:
  `77717fbe629f33298ab46b2077b6f87e05b0005ccf9635592e17147f0db6854c`.
