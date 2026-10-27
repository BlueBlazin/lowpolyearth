"""Individually measured American towers; all model coordinates are metres."""
import json
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.geometry import Mesh, face, plan_solid, plan_triangles, curtain_wall, exposed_grid_towers
from heritage_catalog.geometry import lathe
from heritage_catalog.materials import WINDOW, SCULPTURE_METAL, FLAT_ROOF, STONE_WALL, PANEL_WALL
from skyscraper_catalog import appearance

REF = Path(__file__).parent/'references/americas'
FIT = json.loads((REF/'willis-tower-placement.json').read_text())
SURVEY = json.loads((REF/'willis-tower-survey.json').read_text())
H = math.radians(FIT['heading_deg'])
ROT = np.array([[math.cos(H),-math.sin(H)],[math.sin(H),math.cos(H)]])
PARTS = {b['seed']:b for b in SURVEY['buildings']}


def _plan(seed):
    return (np.asarray(PARTS[seed]['rings'][0])-FIT['center_en_m'])@ROT.T


WILLIS_PODIUM = _plan('15686133503869317160')
WILLIS_TUBES = [(-1,1,200),(0,1,355),(1,1,260),
               (-1,0,442.1),(0,0,442.1),(1,0,355),
               (-1,-1,260),(0,-1,355),(1,-1,200)]


def _willis(lod):
    m=Mesh(); metal=(111,111,103); glass=(129,123,109)
    # The architectural tower uses the published 75-foot structural modules;
    # map-derived podium rings retain the irregular current street perimeter.
    def caps(p,bottom,top):
        for tri in plan_triangles(p):
            face(m,[(x,bottom,z)for x,z in tri],metal,PANEL_WALL,(0,-1,0))
            face(m,[(x,top,z)for x,z in tri],(160,156,143),FLAT_ROOF)
    caps(WILLIS_PODIUM,-3.2,9.6)
    for seed in ['15493498041928313460','12200713245375684914','373437713016860856']:
        p=_plan(seed);height=PARTS[seed]['height']
        caps(p,9.6,height)
        if sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            if np.linalg.norm(b-a)>1:
                curtain_wall(m,a,b,9.6,height,4.5,2.0,(98,127,131),(67,75,72),min(lod,1),.7,.16,103)
        center=p.mean(axis=0)
        garden=center+(p-center)*.60
        for tri in plan_triangles(garden):
            face(m,[(x,height+.06,z)for x,z in tri],(98,127,72),3)
    # Low street glazing and separated doors follow the actual podium boundary.
    p=WILLIS_PODIUM
    if sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        if np.linalg.norm(b-a)>2:
            curtain_wall(m,a,b,-3.2,9.6,4.5,2.2,(89,107,109),metal,min(lod,1),.8,.18,103)
    exposed_grid_towers(m,WILLIS_TUBES,22.86,4.0,glass,metal,lod,
                        bands=[(112,124),(252,260),(348,355),(412,432)])
    for x,z,height in WILLIS_TUBES:
        cx,cz=x*22.86,z*22.86
        if height<400:
            lathe(m,(cx,cz),[(height,4.2),(height+3.0,4.2),(height+5,.8)],(109,113,113),SCULPTURE_METAL,(16,12,8)[lod])
    for x,tip in [(-24.75,527),(2.12,520.3)]:
        lathe(m,(x,0),[(442.1,1.45),(446,1.45),(447,.78),(475,.62),(499,.46),(tip,.12)],(199,200,189),SCULPTURE_METAL,(16,12,8)[lod])
        for y in np.arange(450,tip-2,5):
            lathe(m,(x,0),[(y,.85 if y<475 else .6),(y+.35,.85 if y<475 else .6)],(158,159,149),SCULPTURE_METAL,8)
    # Four west-facing Skydeck boxes project 1.3m beyond the existing facade.
    if lod<2:
        for z in [-5.85,-1.95,1.95,5.85]:
            m.box((-34.95,413.6,z),(1.3,3.0,3.3),(128,158,165),WINDOW)
            for dz in [-1.65,1.65]:m.beam((-35.6,412.1,z+dz),(-35.6,415.1,z+dz),.045,(166,175,174),SCULPTURE_METAL,4)
    return m


ENTRY = {
    'name':'willis-tower','city':'Chicago','lat':FIT['lat'],'lon':FIT['lon'],
    'heading':FIT['heading_deg'],'base':181.4,
    'suppress_buildings':True,'suppress_roads':True,'suppress_trees':True,
    'source_exclusion_polygons':[tri.tolist()for tri in plan_triangles(WILLIS_PODIUM)],
    'road_exclusion_polygons':[tri.tolist()for tri in plan_triangles(WILLIS_PODIUM)],
    'max_source_building_span_m':160,
    'terrain_flatten':[],
    'terrain_cuts':[{'polygon_xz':tri.tolist(),'plane':[0,0,-.15],'feather_m':2.0}for tri in plan_triangles(WILLIS_PODIUM)],
    'description':'Nine bundled tubes with directional setbacks, muted office glazing, mechanical bands, two antenna masts and the current retail podium.',
    'attribution':'Original Low Poly Earth geometry; mapped podium and placement © OpenStreetMap contributors (ODbL), via Protomaps.',
    'references':['https://www.som.com/projects/willis-tower-formerly-sears-tower/',
        'https://www.aisc.org/media/hbbp3p4k/f115-21.pdf',
        'https://www.skyscrapercenter.com/building/willis-tower/169',
        'https://www.architecture.org/online-resources/buildings-of-chicago/willis-tower',
        'https://www.gensler.com/projects/willis-tower-repositioning'],
    'audit':{'architectural_height_m':442.1,'tip_height_m':527,'structural_module_m':22.86,
        'height_authority':'CTBUH architectural/tip height; AISC 75ft modules and directional tube layout.',
        'estimates':'200/260/355m setback levels and podium heights are mapped estimates, not a vertical survey. Secondary mast520.3m and mechanical-band levels are approximate.',
        'foundation_bottom_m':-3.2,'ground_datum_m':181.4,'survey_file':'tools/skyscraper_catalog/references/americas/willis-tower-survey.json',
        'placement_file':'tools/skyscraper_catalog/references/americas/willis-tower-placement.json'},
    'capture':{'distance':920,'azimuth':235,'elevation':24,'target_height':220},
}
from skyscraper_catalog import americas_modern, americas_classic

ENTRIES = [ENTRY,*americas_modern.ENTRIES,*americas_classic.ENTRIES]
DEPENDENCIES = [Path(__file__).parent/'geometry.py',Path(__file__).parent/'inventory.py',
                Path(appearance.__file__),*appearance.DEPENDENCIES,
                Path(americas_modern.__file__),Path(americas_classic.__file__),
                *americas_modern.DEPENDENCIES,*americas_classic.DEPENDENCIES,*REF.glob('*.json')]


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('expected LOD 0, 1 or 2')
    if name=='willis-tower':return _willis(lod)
    if name in americas_modern.BUILDERS:return americas_modern.build(name,lod)
    return americas_classic.build(name,lod)
