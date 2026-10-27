# Low Poly Earth

The whole Earth in 3D, built from open map and elevation data. Pan and zoom
from orbit to street level, fly over any place on the planet, and photograph
it at any date, time and weather.

![The Thames at sunset, London](screenshots/london-thames-sunset.webp)

## Download

**[Download Low Poly Earth for Mac](https://github.com/BlueBlazin/lowpolyearth/releases/latest/download/Low-Poly-Earth-Apple-Silicon.dmg)**
(free, about 640 MB)

- Requires a Mac with Apple silicon and macOS 13 or later.
- Open the disk image and drag Low Poly Earth to Applications. The app is
  signed and notarized by Apple.
- An internet connection is needed for full detail. Buildings, roads, water and
  fine terrain stream from online map and elevation sources.

Release notes and checksums are on the [releases page](https://github.com/BlueBlazin/lowpolyearth/releases).

## Screenshots

Every image is an unedited capture from the app.

<table>
  <tr>
    <td><img src="screenshots/rio-sunset.webp" alt="Guanabara Bay at sunset, Rio de Janeiro"><br>Rio de Janeiro, Brazil</td>
    <td><img src="screenshots/amsterdam-canals-topdown.webp" alt="Canal rings seen straight down, Amsterdam"><br>Amsterdam, the Netherlands</td>
  </tr>
  <tr>
    <td><img src="screenshots/matterhorn-winter.webp" alt="The Matterhorn in winter"><br>Matterhorn, Switzerland</td>
    <td><img src="screenshots/tromso-aurora.webp" alt="Aurora over Tromsø at night"><br>Tromsø, Norway</td>
  </tr>
  <tr>
    <td><img src="screenshots/machu-picchu-clouds.webp" alt="Machu Picchu ridge above valley cloud"><br>Machu Picchu, Peru</td>
    <td><img src="screenshots/cesky-krumlov-snow.webp" alt="Český Krumlov river bend in snow"><br>Český Krumlov, Czechia</td>
  </tr>
  <tr>
    <td><img src="screenshots/dubrovnik-morning.webp" alt="Dubrovnik's walled old town in the morning"><br>Dubrovnik, Croatia</td>
    <td><img src="screenshots/monument-valley.webp" alt="Monument Valley buttes at sunset"><br>Monument Valley, United States</td>
  </tr>
</table>

More photographs are on [the website](https://lowpolyearth.pages.dev/screenshots.html).

## What it does

- **Explore:** search for any place by name or pick it on the world map, and
  the camera flies there.
- **Fly:** press P to take off in one of the flyable vehicles.
- **Photograph:** set the date, time and weather, and save the view as a 4K
  image.
- **Light and weather:** the sun, moon, stars and aurora follow the real date,
  time and location, with simulated clouds, rain, snow and fog.
- **Landmarks:** 231 landmarks are modelled individually.

## Feedback

Found a bug or a place that looks wrong? [Open an issue](https://github.com/BlueBlazin/lowpolyearth/issues)
with the location and, if you can, a screenshot. You can also write to
contact.urbanphilosophy@gmail.com.

## Data sources

Elevation: [Mapterhorn](https://mapterhorn.com/attribution) (Copernicus GLO-30
and national elevation models), [USGS 3DEP](https://www.usgs.gov/3d-elevation-program),
[Environment Agency LiDAR](https://environment.data.gov.uk/) (Open Government
Licence v3). Map data © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright)
(ODbL), via [Protomaps](https://protomaps.com/). Imagery: [NASA Blue Marble](https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-map/),
[NSW Spatial Services](https://www.spatial.nsw.gov.au/) (CC BY 4.0). Also
GeoNames, Wikidata, Natural Earth, WorldClim, JRC GHS-SMOD, Timezone Boundary
Builder, ESA Hipparcos (CC BY-NC 3.0 IGO) and ESO/S. Brunier.

The app lists every source and licence in its attribution window, and ships
full third-party notices inside the app bundle.

Low Poly Earth is free, and its source code is in this repository.

## Source code

Low Poly Earth is a native Rust application for exploring a streamed,
multiresolution surface planet. It combines geospatial elevation, imagery, and
map features with deterministic procedural generation, then renders the result
with `wgpu` from globe scale to ground level.

The application includes place search and map navigation, an astronomical
day/night cycle, procedural weather, still-image capture, and three playable
vehicles. Global terrain and surface colour have bundled fallbacks; network
providers supply streamed terrain and map features.

### Build and run

The workspace uses Rust 2024 and requires Rust 1.95 or newer. From the
repository root, first provide the generated offline place index at
`crates/vearth-data/assets/place-search-v1.bin`. The file is intentionally
ignored because of its size; its source manifest records the canonical hash
and rebuild inputs. See [Development](docs/DEVELOPMENT.md#required-local-asset)
for the clean-checkout bootstrap.

Then build and run:

```sh
cargo build --workspace
cargo run --release -p vearth-app
```

The native viewer requires a graphics adapter supported by `wgpu`. Cargo needs
network access when dependencies are not already cached, and the running app
uses network sources where available.

Run the standard validation suite with:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features
```

See [Development](docs/DEVELOPMENT.md) for capture, replay, data-preparation,
and packaging workflows.

### Controls

- Drag to pan, scroll to zoom, and hold Command or Control while dragging to
  orbit and tilt.
- Use WASD or the arrow keys to pan, `+`/`-` to zoom, `Q`/`E` to rotate, and
  `R`/`F` to tilt.
- Press `/` to search, Enter to open or close the map, and `T` to show the
  current location title in explore or play mode. `[` and `]` move simulation
  time by one hour.
- Press `P` to enter or leave play mode. In play mode, use WASD to move,
  Shift to boost, `V` to cycle vehicles, or `1` through `3` to select one
  directly. Space and `C` control vertical movement where the vehicle permits
  it.

The in-app settings and HUD expose weather, visual, data-source, storage, and
photo controls.

### Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Development](docs/DEVELOPMENT.md)
- [Data and attribution](docs/DATA_AND_ATTRIBUTION.md)
- [Performance](docs/PERFORMANCE.md)
- [Environment visual improvements checklist](docs/VISUAL_IMPROVEMENTS.md)

Dataset-level provenance is kept beside the assets it describes:

- Application assets: [fonts](crates/vearth-app/assets/fonts/README.md),
  [map](crates/vearth-app/assets/mercator-map-SOURCE.md), and
  [sky](crates/vearth-app/assets/sky/SOURCES.md)
- Data assets: [administrative regions](crates/vearth-data/assets/admin-regions-SOURCES.md),
  [climate](crates/vearth-data/assets/climate-SOURCES.md),
  [place search](crates/vearth-data/assets/place-search-SOURCES.md),
  [monthly surface imagery](crates/vearth-data/assets/surface-SOURCES.md),
  [terrain](crates/vearth-data/assets/terrain-SOURCES.md), and
  [timezones](crates/vearth-data/assets/timezones-SOURCES.md)
- Generated landmarks: [source manifest](crates/vearth-gen/assets/landmarks/SOURCES.md)

## License

The project-authored code, documentation, and assets are released under the
[MIT License](LICENSE). Included datasets and third-party assets keep their own
terms, recorded in [Data and attribution](docs/DATA_AND_ATTRIBUTION.md) and the
source manifests beside each asset.
