# Bled Island

`bled-plan.json` records OpenStreetMap footprints, stair centerlines and island
way 8149592 in metres east/north of 46.36245, 14.09015. Attribution:
© OpenStreetMap contributors, ODbL 1.0. The source URL and XML hash are embedded.
`bled-buildings.json` freezes the production vector provider's building rings
for replacement tests, including duplicated and clipped tile features.

`bled-ground.json` freezes unmodified production terrain queried with
`city-survey 46.36245 14.09015 RESULT --ground-only --ground-points POINTS`.
POINTS is the Cartesian product of the recorded `z_m` and `x_m` arrays, ordered
northing first. Do not pass `--landmark-ground`: this datum precedes the model.
The source DEM misses the island's hill. A photographic relief estimate raises
only the mapped island interior, reaching approximately 18.5 m above the DEM.
Triangulated grades meet the existing shoreline and leave lake heights alone.
Foundations and stair treads use those same triangles; raw survey interpolation
is clamped at the small northern tip beyond the sampled grid.

Architectural evidence:

- [Island heritage guide](https://www.blejskiotok.si/en/attractions/): separate
  bell tower, 52 m ascent, 99 southern breccia steps, house identities and chapels.
- [Church and ensemble](https://www.blejskiotok.si/en/attractions/cultural-heritage/):
  Baroque church, side chapels and fenced summit terrace.
- [Southern stairs](https://www.blejskiotok.si/app/uploads/2021/02/blejski-otok-stopnice.jpg)
  and [tower details](https://www.blejskiotok.si/app/uploads/2021/03/zvonik-blejski-otok-500.jpg).
- [Aerial roof layout](https://www.fotoobrazy.cz/photos/kostel-na-jezere-bled-1237.jpg)
  and [southern aerial](https://en.bcdn.biz/Images/2024/10/14/9ee91a11-6c79-43e2-b232-f28ad3fa9f27.jpg).

Mapped tower height is 52 m; the northern staircase has 78 steps in OSM.
Other heights, cornices, window spacing, roof profiles, colors and relief are
original photographic approximations, not surveyed measurements. References
are not embedded textures or redistributed images.

Bake and verify:

```sh
python3 tools/build_island_catalog.py
python3 tools/build_island_catalog.py --check
cargo test -p vearth-gen bled_island
cargo test -p vearth-gen indexed_island_grades
```

Native capture (use a fixed copy of a current app build):

```sh
python3 tools/capture_heritage_catalog.py --catalog crates/vearth-gen/assets/landmarks/island-surfaces/catalog.json --only bled-island --app /tmp/bled-bin/vearth-app --survey /tmp/bled-bin/city-survey --target-xz -5 -25 --target-height 23 --distance 240 --azimuth 150 --elevation 25 --output output/bled-island
```
