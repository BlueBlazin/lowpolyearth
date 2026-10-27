#!/usr/bin/env python3
"""Production-encode every mosque LOD and validate narrow terrain ownership."""
import hashlib
import json
from pathlib import Path
import sys
import numpy as np

REF=Path(__file__).resolve().parent
TOOLS=REF.parents[2]
sys.path[:0]=[str(TOOLS),str(TOOLS/'city_catalog')]
from build_city_catalog import encode
from heritage_catalog.sacred_mosques import BUILDERS, ENTRIES, build, survey, floor_datum, Mesh, ranges, zayed_perimeter_cuts


def main():
    strip=Mesh();ranges(strip,[[[0,0],[2,1],[0,2],[-2,1]]],0,(100,100,100))
    normals=[np.cross(p[1]-p[0],p[2]-p[0])[1]for p,_,_ in strip.triangles]
    assert min(normals)>0 and abs(sum(normals)/2-4)<1e-8
    for cut in zayed_perimeter_cuts():
        p=np.asarray(cut['polygon_xz']);edges=np.roll(p,-1,axis=0)-p
        c=edges[:,0]*np.roll(edges[:,1],-1)-edges[:,1]*np.roll(edges[:,0],-1)
        assert (np.all(c>0) or np.all(c<0)) and np.isfinite(cut['plane']).all()
        assert 0<cut['feather_m']<=12
    source=REF.parents[1]/'sacred_mosques.py'
    report={'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'levels':{},'metadata':{}}
    for entry in ENTRIES:
        name=entry['name'];s=survey(name)
        assert s['ground_samples'] and all(np.isfinite(p['ground_m']) for p in s['ground_samples'])
        assert entry['base']==floor_datum(name) and entry['heading']==0 and not entry['terrain_flatten']
        for poly in entry['source_exclusion_polygons']:
            p=np.asarray(poly);assert p.shape==(3,2) and np.isfinite(p).all()
            assert abs(np.linalg.det(np.stack([p[1]-p[0],p[2]-p[0]])))>1e-9
        for cut in entry['terrain_cuts']:
            assert len(cut['plane'])==3 and np.isfinite(cut['plane']).all() and cut['feather_m']>0
        for cut in entry.get('water_cuts',[]):
            assert name=='sheikh-zayed-grand-mosque' and cut['plane']==[0,0,-.65]
            assert cut['feather_m']==.3 and np.asarray(cut['polygon_xz']).shape==(3,2)
        report['levels'][name]=[]
        for lod in range(3):
            mesh=build(name,lod);payload,counts,pp=encode(mesh)
            report['levels'][name].append({**counts,'minimum':pp.min(0).tolist(),'maximum':pp.max(0).tolist(),
                'encoded_sha256':hashlib.sha256(payload).hexdigest(),'materials':sorted(set(int(mat) for _,_,mat in mesh.triangles))})
        counts=[v['triangles'] for v in report['levels'][name]]
        assert counts[0]>=counts[1]>=counts[2]
        report['metadata'][name]={'source_polygons':len(entry['source_exclusion_polygons']),
            'ground_samples':len(s['ground_samples']),'base_m':entry['base'],
            'survey_sha256':hashlib.sha256((REF/(name+'-survey.json')).read_bytes()).hexdigest()}
        print(name,counts,flush=True)
    (REF/'mesh-validation.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
