"""Validate the three Dohány LODs through the production encoder."""
import json
import sys
from pathlib import Path
import numpy as np

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[2]))
sys.path.insert(0,str(HERE.parents[2]/'city_catalog'))
from heritage_catalog import sacred_dohany as model
from build_heritage_catalog import bake

levels=[]
for lod in range(3):
    mesh=model.build(model.NAME,lod)
    points=np.asarray([p for p,c,material in mesh.triangles])
    assert np.isfinite(points).all()
    assert (np.linalg.norm(np.cross(points[:,1]-points[:,0],points[:,2]-points[:,0]),axis=1)>1e-9).all()
    materials=sorted(set(material for p,c,material in mesh.triangles))
    assert 11 not in materials and 12 not in materials
    levels.append(dict(lod=lod,triangles=len(points),bbox_min=points.min((0,1)).tolist(),bbox_max=points.max((0,1)).tolist(),materials=materials))
assert levels[0]['triangles']>=levels[1]['triangles']>=levels[2]['triangles']
assert abs(levels[0]['bbox_max'][1]-43.6)<1e-7
payload,record=bake(model,model.ENTRIES[0])
(HERE/'mesh-validation.json').write_text(json.dumps(dict(name=model.NAME,levels=levels,encoded_lms_bytes=len(payload),source_exclusion_pieces=len(model.OWNERSHIP)),indent=2)+'\n')
print(model.NAME,[level['triangles'] for level in levels])
