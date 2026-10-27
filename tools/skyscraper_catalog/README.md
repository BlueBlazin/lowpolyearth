# Additional skyscraper catalog

`inventory.py` contains the 59 additional names. The baker rejects names already present in the built-in, city or heritage packs and requires the complete inventory for an unfiltered build or check.

```sh
python3 tools/build_skyscraper_catalog.py
python3 tools/build_skyscraper_catalog.py --check
PYTHONPATH=tools:tools/city_catalog python3 -m unittest tools/skyscraper_catalog/test_geometry.py
```

Each model has three LMS levels, a source manifest, mapped placement evidence and explicit grounding metadata. Rebuild after changing any generator, retained plan or material dependency. Source references distinguish published dimensions from modeling and terrain-datum estimates.

Facade skins use the production highrise material families and their normal night response. Glass and pier surfaces must partition the facade without overlapping backing walls. Keep dense window grids only when they define the building's structural form.

Capture a frozen production executable and inspect the landmark against its surroundings:

```sh
python3 tools/capture_skyscraper_catalog.py --only willis-tower --overview --app /absolute/path/to/frozen-vearth-app
```

Use `--target-xz`, `--target-height`, `--distance`, `--azimuth` and `--elevation` for entrance/support views. Capture records include executable, asset, source and image hashes. A mesh preview does not verify terrain contact or runtime materials.
