"""Three additional US towers with mapped plans and native facade finishes."""
import json
from pathlib import Path
import numpy as np
from skyscraper_catalog.americas_common import facade, loft, roof, rect
from skyscraper_catalog.geometry import Mesh, face, plan_solid
from skyscraper_catalog.regional import prism_part, ccw
from heritage_catalog.geometry import lathe
from heritage_catalog.materials import PANEL_WALL, STONE_WALL, SCULPTURE_METAL, FLAT_ROOF

REF=Path(__file__).parent/'references/extra_us_east'
SPECS={
 'central-park-tower':('New York City',472.44,'Mapped stepped glass residential tower, east cantilever and seven-story Nordstrom podium.',['https://www.smithgill.com/work/central_park_tower/','https://centralparktower.com/tower']),
 'bank-of-america-plaza-atlanta':('Atlanta',311.8,'Rose-granite stepped shaft with receding crown and open gold pyramidal lattice.',['https://www.gensler.com/projects/bank-of-america-plaza-lobby-renovations','https://www.bankofamericaplaza.com/','https://www.skyscrapercentre.com/building/bank-of-america-plaza/429']),
 '200-clarendon-street':('Boston',240.8,'Minimal reflective rhomboid slab with mapped notches in both narrow ends.',['https://www.pcf-p.com/projects/john-hancock-tower/','https://200clarendon.com/the-building/','https://www.skyscrapercenter.com/building/200-clarendon/974'])}

def read(n,suffix):return json.loads((REF/(n+'-'+suffix+'.json')).read_text())
PLANS={n:read(n,'placement')for n in SPECS}
GROUND={n:read(n,'ground')for n in SPECS}

def entry(n):
 p=PLANS[n];g=GROUND[n];city,height,desc,urls=SPECS[n]
 polys=[t for q in p['owned']for t in q['triangles']];ground_polys=[t for q in p.get('ground_owned',p['owned'])for t in q['triangles']];points=np.array([v for q in polys for v in q]);lo,hi=points.min(0),points.max(0)
 return dict(name=n,city=city,lat=p['lat'],lon=p['lon'],heading=p['heading'],base=g['datum_m'],suppress_buildings=True,suppress_roads=False,suppress_trees=True,source_exclusion_polygons=polys,max_source_building_span_m=180,terrain_flatten=[],terrain_cuts=[dict(polygon_xz=t,plane=[0,0,-.15],feather_m=2.5)for t in ground_polys],activation_bounds_xz=[lo.tolist(),hi.tolist()],description=desc,references=urls,attribution='Original Low Poly Earth geometry; mapped plans © OpenStreetMap contributors, ODbL1.0 via Protomaps.',audit=dict(architectural_height_m=height,tip_height_m=height,ground_datum_m=g['datum_m'],foundation_bottom_m=g['foundation_bottom_m'],placement_file=str((REF/(n+'-placement.json')).relative_to(REF.parents[3])),ground_file=str((REF/(n+'-ground.json')).relative_to(REF.parents[3])),estimates='Native facade bay subdivision and secondary crown detail are modeled approximations; primary dimensions and production plan/ground controls retained in references.'),capture=dict(distance=height*1.8,azimuth=150,elevation=24,target_height=height*.48))
ENTRIES=[entry(n)for n in SPECS]
E={e['name']:e for e in ENTRIES}

def footing(m,n):
 for q in PLANS[n].get('ground_owned',PLANS[n]['owned']):prism_part(m,q,GROUND[n]['foundation_bottom_m'],.2,(160,159,147),STONE_WALL)


def central(lod):
 n='central-park-tower';m=Mesh();footing(m,n)
 # Unioned active source parts avoid coplanar walls where separate mapped
 # components overlap;89m minimum-height parts preserve the real cantilever.
 for layer in PLANS[n]['layers']:
  lo,hi=layer['bottom'],layer['top']
  for q in layer['polygons']:
   p=ccw(q['outer']);facade(m,p,p,max(.2,lo),hi,4.2,3,lod,metal=(161,172,175),native_material=104)
 return m


def atlanta(lod):
 n='bank-of-america-plaza-atlanta';m=Mesh();footing(m,n)
 p=ccw(PLANS[n]['owned'][0]['outer']);center=(p.min(0)+p.max(0))/2
 # The stepped perimeter and chamfered corners are retained from the source;
 # upper setbacks are proportion estimates constrained to the311.8m tip.
 for lo,hi,scale in [(0,232,1),(232,246,.88),(246,256,.72),(256,266,.55)]:
  ring=center+(p-center)*scale
  facade(m,ring,ring,max(.2,lo),hi,4.3,3.2,lod,stone=True,metal=(167,131,120),wall_material=PANEL_WALL,native_material=106)
 top=rect(24,24,*center);loft(m,top,top*.82+center*.18,266,279,(173,143,91),PANEL_WALL)
 # Gold-leaf pyramidal lattice remains open, with a small central mast.
 corners=rect(20,20,*center);apex=np.array([center[0],305.2,center[1]])
 for a,b in zip(corners,np.roll(corners,-1,axis=0)):
  aa=np.array([a[0],279,a[1]]);bb=np.array([b[0],279,b[1]])
  m.beam(aa,apex,.32,(186,157,81),SCULPTURE_METAL,(8,6,4)[lod])
  for t in np.linspace(0,.88,(9,6,4)[lod]):
   u=aa*(1-t)+apex*t;v=bb*(1-t)+apex*t;m.beam(u,v,.12,(175,143,70),SCULPTURE_METAL,4)
  if lod<2:
   for t in [.25,.5,.75]:m.beam(aa*(1-t)+bb*t,apex,.10,(180,150,77),SCULPTURE_METAL,4)
 lathe(m,tuple(center),[(279,.65),(305.2,.38),(311.8,.05)],(190,161,84),SCULPTURE_METAL,(12,8,6)[lod])
 return m


def clarendon(lod):
 n='200-clarendon-street';m=Mesh();footing(m,n)
 for q in PLANS[n]['parts'][0]['polygons']:
  p=ccw(q['outer']);facade(m,p,p,.2,240.8,3.85,4,lod,metal=(143,162,169),native_material=104,mullion=.10)
 return m

BUILDERS=dict(zip(SPECS,[central,atlanta,clarendon]))
DEPENDENCIES=[Path(__file__),Path(__file__).parent/'americas_common.py',Path(__file__).parent/'geometry.py',Path(__file__).parent/'appearance.py',Path(__file__).parent/'regional.py',Path(__file__).resolve().parents[2]/'crates/vearth-gen/src/highrise.rs',*sorted(REF.glob('*.json'))]

def build(name,lod):
 if lod not in (0,1,2):raise ValueError('expected LOD0,1,2')
 return BUILDERS[name](lod)
