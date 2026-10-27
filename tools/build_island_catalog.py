#!/usr/bin/env python3
"""Bake mapped island ensembles; --check verifies geometry and provenance."""
import argparse
import json
from pathlib import Path

import build_city_catalog  # Configure shared geometry import paths.
from build_heritage_catalog import bake, registry
from heritage_catalog import islands

ROOT=Path(__file__).resolve().parents[1]
OUTPUT=ROOT/'crates/vearth-gen/assets/landmarks/island-surfaces'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    outputs={};entries=[]
    for entry in islands.ENTRIES:
        payload,record=bake(islands,entry);name=entry['name'];entries.append(record)
        outputs[OUTPUT/(name+'.lms')]=payload
        outputs[OUTPUT/(name+'.source.json')]=(json.dumps(record,indent=2)+'\n').encode()
        print(name,record['levels'],flush=True)
    code=registry(entries).replace(b'build_heritage_catalog.py',b'build_island_catalog.py').replace(b'HERITAGE_MODELS',b'ISLAND_MODELS').replace(b'/heritage-surfaces/',b'/island-surfaces/')
    outputs[ROOT/'crates/vearth-gen/src/landmarks/island_generated.rs']=code
    outputs[OUTPUT/'catalog.json']=(json.dumps(entries,indent=2)+'\n').encode()
    for path,data in outputs.items():
        if args.check:assert path.exists() and path.read_bytes()==data,f'stale island asset: {path}'
        else:path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)


if __name__=='__main__':main()
