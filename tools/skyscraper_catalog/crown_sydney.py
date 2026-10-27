"""Crown Sydney from the architect's registered floor sections and elevations."""
import json
from pathlib import Path
import numpy as np
from skyscraper_catalog.pacific_geometry import (
    Mesh, face, clean, roof, slab, support, envelope, PANEL_WALL,
)
from skyscraper_catalog.appearance import panel_tint

REFERENCE = Path(__file__).parent/'references/extra_sydney_chicago/crown-sydney-sections.json'
DEPENDENCIES = [Path(__file__), REFERENCE]
BASE = 3.7


def reference():
    return json.loads(REFERENCE.read_text())


def world(points, data):
    return np.asarray(points, float) @ np.asarray(data['plan_to_world_matrix']).T + data['plan_translation_xz_m']


def _triangle_panel(mesh, triangle, edges, glass, frame, width):
    """Partition one triangle in its own plane: frames cannot intersect the glass."""
    original = np.asarray(triangle, float)
    normal = np.cross(original[1]-original[0], original[2]-original[0])
    area = np.linalg.norm(normal)
    if area < 1e-9:
        return
    normal /= area
    remaining = list(original)
    for i,j in edges:
        a,b = original[i],original[j]
        tangent = b-a
        inward = np.cross(normal,tangent)/np.linalg.norm(tangent)
        kept,removed = [],[]
        for c,d in zip(remaining,remaining[1:]+remaining[:1]):
            dc,dd = np.dot(c-a,inward)-width, np.dot(d-a,inward)-width
            (kept if dc >= 0 else removed).append(c)
            if (dc < 0) != (dd < 0):
                q = c+(d-c)*dc/(dc-dd)
                kept.append(q); removed.append(q)
        if len(removed) >= 3:
            face(mesh,removed,frame,PANEL_WALL,normal)
        remaining = kept
        if len(remaining) < 3:
            return
    face(mesh,remaining,glass,104,normal)


def _wall(mesh, low, high, lod, diagonal=True):
    glass = panel_tint(104,(149,163,168))
    frame = (145,158,163)
    for i,a in enumerate(low):
        j=(i+1)%len(low)
        b,c,d=low[j],high[j],high[i]
        # Rings are counterclockwise in the engine's east/north plane.
        normal=np.array([b[2]-a[2],0,a[0]-b[0]])
        if lod == 2:
            face(mesh,[a,b,c,d],glass,104,normal)
            continue
        diag = diagonal and lod == 0 and normal[0]+normal[2] < 0
        for tri,edges in (([a,d,b],[(0,1)]),([b,d,c],[(1,2)])):
            if diag: edges=[(0,1),(1,2),(2,0)]
            _triangle_panel(mesh,tri,edges,glass,frame,.055 if lod == 0 else .075)


def _ring(section, data, lod, key='ring_uv_m'):
    # Nested samples preserve the same measured section in every LOD.
    p=world(section[key][::(2,3,4)[lod]],data)
    heights=np.asarray(section['vertex_rl_m'][::(2,3,4)[lod]])-BASE if 'vertex_rl_m' in section else np.full(len(p),section['rl_m']-BASE)
    return np.column_stack([p[:,0],heights,p[:,1]])


def build(lod, site_data):
    data=reference(); mesh=Mesh(); tags=[]
    def mark(name):
        tags.extend([name]*(len(mesh.triangles)-len(tags)))
    for p in data['ground_support_uv_m']:
        support(mesh,site_data,world(p,data),BASE,color=(151,153,145))
    mark('Grounded foundations')
    profiles=data['podium']
    for i,profile in enumerate(profiles):
        lo=profile['rl_m']-BASE
        hi=profiles[i+1]['rl_m']-BASE if i+1<len(profiles) else 40-BASE
        for raw in profile['body_uv_m']:
            p=world(raw,data)
            envelope(mesh,p,lo,hi,lod,(118,141,151),(153,163,164),material=104)
        for raw in profile['terraces_uv_m']:
            slab(mesh,world(raw,data),lo,.35,(165,168,158))
        for raw in profile['pools_uv_m']:
            roof(mesh,world(raw,data),lo+.10,(67,131,141))
    mark('Measured podium floors and terraces')
    for group in data['tower_groups']:
        rings=[_ring(section,data,lod) for section in group['sections']]
        for low,high in zip(rings,rings[1:]):
            _wall(mesh,low,high,lod)
        if group.get('cap'):
            roof(mesh,rings[-1][:,[0,2]],rings[-1][0,1],(131,147,148))
        mark(group['name'])
    for terrace in data['tower_terraces']:
        p=world(terrace['uv_m'],data)
        slab(mesh,p,terrace['rl_m']-BASE,.18,(137,151,144))
    mark('Recessed apartment and hotel terraces')
    top=data['roof']
    low=_ring(top,data,lod)
    q=world(top['rim_uv_m'][::(2,3,4)[lod]],data)
    high=np.column_stack([q[:,0],np.asarray(top['rim_rl_m'][::(2,3,4)[lod]])-BASE,q[:,1]])
    _wall(mesh,low,high,lod)
    # A recessed maintenance roof leaves the actual unequal curved glass rims visible.
    roof(mesh,high[:,[0,2]],267.2-BASE,(103,121,130))
    for a,b in zip(high,np.roll(high,-1,axis=0)):
        normal=(b[2]-a[2],0,a[0]-b[0])
        face(mesh,[a,b,(b[0],263.5,b[2]),(a[0],263.5,a[2])],(125,142,150),PANEL_WALL,tuple(-v for v in normal))
    mark('Unequal curved crown rims')
    mesh.crown_face_tags=tags
    return mesh
