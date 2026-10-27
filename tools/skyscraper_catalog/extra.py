"""Additional Sydney and United States towers using the native facade palette."""
from pathlib import Path
from skyscraper_catalog import extra_sydney_chicago, extra_us_west, extra_us_east

MODULES = (extra_sydney_chicago, extra_us_west, extra_us_east)
ENTRIES = [entry for module in MODULES for entry in module.ENTRIES]
BUILDERS = {entry['name']: module for module in MODULES for entry in module.ENTRIES}
DEPENDENCIES = [path for module in MODULES
                for path in (Path(module.__file__), *module.DEPENDENCIES)]


def build(name, lod):
    return BUILDERS[name].build(name, lod)
