#!/usr/bin/env python3
"""Encode all Vatican LODs and verify source/terrain ownership contracts."""
import hashlib
import json
from pathlib import Path
import sys
import numpy as np
REF=Path(__file__).resolve().parent
TOOLS=REF.parents[2]
sys.path[:0]=[str(TOOLS),str(TOOLS/'city_catalog')]
from build_city_catalog import encode
from heritage_catalog.sacred_vatican import ENTRIES, build, BASE, FLOOR, POLYGONS
from heritage_catalog.materials import validate_runtime_ids


def main():
    validate_runtime_ids()
    source=REF.parents[1]/'sacred_vatican.py'
    report={'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'levels':[],'source_polygons':len(POLYGONS),'base_m':BASE}
    entry=ENTRIES[0]
    assert entry['heading']==0 and not entry['terrain_flatten'] and not entry['suppress_roads']
    for poly in POLYGONS:
        p=np.asarray(poly);assert p.shape==(3,2) and np.isfinite(p).all()
        assert abs(np.linalg.det(np.stack([p[1]-p[0],p[2]-p[0]])))>1e-8
    for lod in range(3):
        mesh=build('st-peters-basilica',lod);payload,counts,p=encode(mesh)
        assert abs(p[:,1].max()-133.3)<.01
        report['levels'].append({**counts,'encoded_sha256':hashlib.sha256(payload).hexdigest(),'minimum':p.min(0).tolist(),'maximum':p.max(0).tolist(),'materials':sorted(set(mat for _,_,mat in mesh.triangles))})
        print(lod,counts,flush=True)
    counts=[v['triangles']for v in report['levels']];assert counts[0]>counts[1]>counts[2]
    report['evidence_sha256']={p.name:hashlib.sha256(p.read_bytes()).hexdigest()for p in REF.glob('*.json')if p.name!='mesh-validation.json'}
    (REF/'mesh-validation.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
