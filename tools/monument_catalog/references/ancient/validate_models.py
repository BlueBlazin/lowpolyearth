"""Validate ancient monument extents, support footprints and retained silhouettes.

Run: PYTHONPATH=tools:tools/city_catalog python3 tools/monument_catalog/references/ancient/validate_models.py
"""
from collections import Counter
import json
from pathlib import Path

import numpy as np

from monument_catalog.ancient import (
    ENTRIES, MOAI_HEIGHTS, STONEHENGE_UPRIGHTS, STONEHENGE_LINTELS,
    BRODGAR_STANDING_SLOTS, build, build_components,
)


def main():
    assert len(ENTRIES)==10
    assert len(MOAI_HEIGHTS)==15 and len(set(MOAI_HEIGHTS))>=12
    assert len(STONEHENGE_UPRIGHTS)==17 and len(STONEHENGE_LINTELS)==6
    assert len(BRODGAR_STANDING_SLOTS)==27
    for entry in ENTRIES:
        name=entry['name'];last=None
        evidence=json.loads(Path(entry['audit']['survey_file']).read_text())
        assert evidence['name']==name and len(evidence['sources'])>=1
        assert not entry['suppress_roads']
        assert not any(entry.get(k) for k in ('terrain_flatten','terrain_cuts','terrain_grades','water_cuts'))
        assert entry['ground_samples_file']==f'tools/monument_catalog/references/ground/{name}.json'
        footprint=np.asarray(entry['suppress_bbox'])
        low=footprint.min(0);high=footprint.max(0)
        component_count=None
        for lod in range(3):
            mesh=build(name,lod)
            p=np.asarray([points for points,_,_ in mesh.triangles])
            colors=np.asarray([c for _,c,_ in mesh.triangles])
            assert np.isfinite(p).all() and p.min(axis=(0,1))[1]<0
            assert len(p)>0 and (last is None or len(p)<=last)
            assert len(np.unique(colors,axis=0))>=12
            assert np.min(np.linalg.norm(np.cross(p[:,1]-p[:,0],p[:,2]-p[:,0]),axis=1))>1e-9
            assert np.all(p[:,:,[0,2]]>=low-1e-6) and np.all(p[:,:,[0,2]]<=high+1e-6),(name,lod)
            parts=build_components(name,lod)
            assert component_count is None or len(parts)==component_count,(name,lod)
            component_count=len(parts)
            assert sum(len(part.triangles) for part in parts)==len(mesh.triangles),(name,lod)
            for i,part in enumerate(parts):
                assert any(points[:,1].min()<=.05 for points,_,_ in part.triangles),(name,lod,i)
            if lod==0:
                if name in ('stonehenge','callanish-stones','ring-of-brodgar'):
                    polygons=entry['source_exclusion_polygons']
                    assert len(polygons)==len(parts),(name,len(polygons),len(parts))
                    area=0
                    for polygon,part in zip(polygons,parts):
                        q=np.asarray(polygon);n=np.roll(q,-1,axis=0)
                        area+=abs(np.sum(q[:,0]*n[:,1]-q[:,1]*n[:,0]))*.5
                        feet=np.asarray([v[[0,2]] for tri,_,_ in part.triangles for v in tri if v[1]<=.05])
                        for a,b in zip(q,n):
                            d=b-a;rel=feet-a
                            assert np.all(d[0]*rel[:,1]-d[1]*rel[:,0]>=-1e-8),(name,a,b)
                    assert area<.15*np.prod(high-low),(name,area)
                key=lambda tri:(tri[0].tobytes(),tri[1],tri[2])
                assert Counter(map(key,mesh.triangles))==Counter(key(tri) for part in parts for tri in part.triangles),name
            height=p[:,:,1].max()
            assert abs(height-entry['audit']['architectural_dimensions_m'][1])<.025,(name,height)
            last=len(p)
        print(name, 'valid')


if __name__=='__main__':
    main()
