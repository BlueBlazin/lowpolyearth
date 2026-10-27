#!/usr/bin/env python3
"""Bake the additional skyscraper catalog; --only supports incremental authoring.

The unfiltered --check requires every selected building, its evidence, and
all three runtime LODs. Existing catalog meshes are never rewritten here.
"""
import argparse
import importlib
import json
from pathlib import Path
import re
import subprocess
from types import SimpleNamespace

from build_heritage_catalog import bake, registry
from build_city_catalog import digest
from skyscraper_catalog.inventory import GROUPS, NAMES
from skyscraper_catalog import appearance

ROOT=Path(__file__).resolve().parents[1]
OUTPUT=ROOT/'crates/vearth-gen/assets/landmarks/skyscraper-surfaces'
REGISTRY=ROOT/'crates/vearth-gen/src/landmarks/skyscraper_generated.rs'


def runtime_module(module):
    def build(name,lod):
        mesh=module.build(name,lod)
        # The native lit family retains identical daytime material behavior.
        # Only occupied facade skins gain the engine's normal night response.
        mesh.triangles=[(p,c,material+8 if 103<=material<=110 else material)
                        for p,c,material in mesh.triangles]
        return mesh
    return SimpleNamespace(__file__=module.__file__,__name__=module.__name__,
                           DEPENDENCIES=module.DEPENDENCIES,build=build)


def existing_names():
    names=set(re.findall(r'name: "([^"]+)"',
        (ROOT/'crates/vearth-gen/src/landmarks/models.rs').read_text()))
    for pack in ['city-surfaces','heritage-surfaces']:
        names.update(e['name']for e in json.loads((OUTPUT.parent/pack/'catalog.json').read_text()))
    return names


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--only',nargs='+',choices=(*NAMES,*GROUPS))
    args=parser.parse_args()
    assert not set(NAMES)&existing_names(),'additional collection duplicates an existing landmark'
    selected=set(NAMES)if not args.only else {n for v in args.only for n in GROUPS.get(v,(v,))}
    outputs={};records={}
    for group,expected in GROUPS.items():
        wanted=selected&set(expected)
        if not wanted:continue
        module=importlib.import_module('skyscraper_catalog.'+group)
        entries={e['name']:e for e in module.ENTRIES}
        assert len(entries)==len(module.ENTRIES)and not set(entries)-set(expected),group
        assert not wanted-set(entries),f'unimplemented sites: {sorted(wanted-set(entries))}'
        module.DEPENDENCIES=[*getattr(module,'DEPENDENCIES',[]),Path(__file__),
            Path(appearance.__file__),*appearance.DEPENDENCIES,
            ROOT/'tools/skyscraper_catalog/geometry.py']
        for name in expected:
            if name not in wanted:continue
            payload,record=bake(runtime_module(module),{**entries[name],'night_eligible':True})
            outputs[OUTPUT/(name+'.lms')]=payload
            outputs[OUTPUT/(name+'.source.json')]=(json.dumps(record,indent=2)+'\n').encode()
            records[name]=record
            print(name,record['levels'],flush=True)
    for name in NAMES:
        p=OUTPUT/(name+'.source.json')
        if name not in records and p.exists():
            record=json.loads(p.read_text())
            assert digest((OUTPUT/(name+'.lms')).read_bytes())==record['output_sha256'],name
            records[name]=record
    entries=[records[n]for n in NAMES if n in records]
    if not args.only:assert len(entries)==len(NAMES)
    # The shared writer owns the LMS and terrain metadata schema. Only the
    # pack's Rust symbol, embedded asset directory, and provenance header differ.
    code=registry(entries)
    for old,new in [(b'build_heritage_catalog.py',b'build_skyscraper_catalog.py'),
                    (b'HERITAGE_MODELS',b'SKYSCRAPER_MODELS'),
                    (b'/heritage-surfaces/',b'/skyscraper-surfaces/')]:
        assert old in code
        code=code.replace(old,new)
    code=subprocess.run(['rustfmt','--edition','2024'],input=code,
                        capture_output=True,check=True).stdout
    outputs[REGISTRY]=code
    outputs[OUTPUT/'catalog.json']=(json.dumps(entries,indent=2)+'\n').encode()
    for p,data in outputs.items():
        if args.check:
            assert p.exists()and p.read_bytes()==data,f'stale generated skyscraper: {p}'
        else:
            p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data)


if __name__=='__main__':main()
