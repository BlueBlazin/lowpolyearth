"""Validate all39 castle meshes and their encoded production LMS payloads."""
from pathlib import Path
import json
import sys
import numpy as np

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[2]))
from heritage_catalog import castles
from build_heritage_catalog import bake
from heritage_catalog.materials import validate_runtime_ids

validate_runtime_ids()
output=HERE/'mesh-validation.json'
records=json.loads(output.read_text())['models'] if len(sys.argv)>1 and output.exists() else []
for entry in castles.ENTRIES:
    name=entry['name'];levels=[]
    if len(sys.argv)>1 and name not in sys.argv[1:]:continue
    for lod in range(3):
        mesh=castles.build(name,lod)
        points=np.asarray([p for p,c,mat in mesh.triangles])
        assert np.isfinite(points).all(),name
        assert (np.linalg.norm(np.cross(points[:,1]-points[:,0],points[:,2]-points[:,0]),axis=1)>1e-9).all(),name
        assert all(mat not in (11,12) for p,c,mat in mesh.triangles),(name,'unassigned generic material')
        levels.append({'lod':lod,'triangles':len(points),'bbox_min':points.min((0,1)).tolist(),'bbox_max':points.max((0,1)).tolist(),'materials':sorted(set(mat for p,c,mat in mesh.triangles))})
    assert levels[0]['triangles']>=levels[1]['triangles']>=levels[2]['triangles'],name
    payload,record=bake(castles,entry)
    records=[r for r in records if r['name']!=name]
    records.append({'name':name,'levels':levels,'encoded_lms_bytes':len(payload),'source_exclusion_pieces':len(entry['source_exclusion_polygons']),'terrain_cut_count':len(entry.get('terrain_cuts',[]))})
    print(name,[v['triangles'] for v in levels],flush=True)
records.sort(key=lambda r:next(i for i,e in enumerate(castles.ENTRIES) if e['name']==r['name']))
output.write_text(json.dumps({'validation':'Finite nondegenerate triangles; decreasing LOD counts; physical materials; production LMS encoding and convex ownership/cut bounds. Engine visual review is separate.','models':records},indent=2)+'\n')
