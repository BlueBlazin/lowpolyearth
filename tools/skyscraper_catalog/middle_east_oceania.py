"""Combine the independently authored Middle Eastern and Australian groups."""
from pathlib import Path
from skyscraper_catalog import middle_east, oceania

MODULES=(middle_east,oceania)
ENTRIES=[e for module in MODULES for e in module.ENTRIES]
DEPENDENCIES=[p for module in MODULES for p in [Path(module.__file__),*module.DEPENDENCIES]]

def build(name,lod):
    for module in MODULES:
        if any(e['name']==name for e in module.ENTRIES):return module.build(name,lod)
    raise KeyError(name)
