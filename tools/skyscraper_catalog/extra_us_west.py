"""Three American towers with production footprints and native facade finishes."""
import json
import math
from pathlib import Path
import numpy as np
from heritage_catalog.geometry import Mesh, face, plan_solid, plan_triangles, lathe
from heritage_catalog.materials import STONE_WALL, PANEL_WALL, FLAT_ROOF, SCULPTURE_METAL
from skyscraper_catalog.geometry import curtain_wall
from skyscraper_catalog.east_asia_geometry import ccw, resample, loft, roof
from skyscraper_catalog import appearance

REF=Path(__file__).parent/'references/extra_us_west'
NAMES=['us-bank-tower','wilshire-grand-center','comcast-technology-center']
FIT={n:json.loads((REF/(n+'-placement.json')).read_text())for n in NAMES}
SOURCES={
    'us-bank-tower':['https://www.pcf-p.com/projects/us-bank-tower-formerly-library-tower/'],
    'wilshire-grand-center':['https://www.thorntontomasetti.com/project/wilshire-grand-center',
        'https://m.acmartin.com/sectors/commercialindustrial','https://wilshiregrandcenter.com/'],
    'comcast-technology-center':['https://www.fosterandpartners.com/projects/comcast-technology-center/',
        'https://www.skyscrapercenter.com/building/comcast-technology-center/16192',
        'https://www.thorntontomasetti.com/project/comcast-technology-center'],
}


def _part(name,seed):
    d=FIT[name]
    return ccw(np.asarray(next(b for b in d['buildings']if b['seed']==seed)['rings'][0])-d['center_en_m'])


def _prism(m,p,bottom,top,lod,material=104,metal=(150,157,156)):
    p=ccw(p)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        curtain_wall(m,a,b,bottom,top,4.2,5.0,(110,135,143),metal,lod,facade_material=material)
    roof(m,p,top)


def _foundation(m,name):
    d=FIT[name];p=ccw(d['owned_plan_xz'])
    plan_solid(m,p,d['foundation_bottom_m'],.12,(147,145,135),STONE_WALL)
    return p


def _bank(lod):
    name=NAMES[0];m=Mesh();p=_foundation(m,name)
    # Orthogonal notches and circular lobes are retained from the mapped base.
    p=resample(p,(64,40,24)[lod]); center=p.mean(0)
    for bottom,top,scale in [(.12,195,1),(195,226,.94),(226,251,.87),(251,272,.77)]:
        q=center+(p-center)*scale
        _prism(m,q,bottom,top,lod,106,(174,172,161))
        if lod<2:
            plan_solid(m,q,top-.65,top,(178,175,160),STONE_WALL)
    count=(64,40,24)[lod]
    def crown(y):
        r=float(np.interp(y,[272,282,288,295,302,307,310.2864],[20.8,20.8,18.6,18.6,16.8,16.8,16.2]))
        t=np.linspace(0,2*math.pi,count,endpoint=False)
        # The lobed crown blends the tower's circular and orthogonal geometry.
        rr=r*(1+.018*np.cos(8*t))
        return np.column_stack([rr*np.cos(t),rr*np.sin(t)])+center
    loft(m,crown,[272,282,288,295,302,307,310.2864],lod,metal=(184,181,170),glass_material=106)
    for y,r in [(282,20.8),(288,18.6),(302,16.8),(307,16.8)]:
        lathe(m,tuple(center),[(y-.55,r+.2),(y,r+.2)],(180,177,163),STONE_WALL,count)
    roof(m,crown(310.2864),310.2864,(152,155,148))
    return m


def _wilshire(lod):
    name=NAMES[1];m=Mesh();_foundation(m,name)
    podium=_part(name,'7090649355758135356')
    _prism(m,podium,.12,25,lod,107,(150,154,153))
    p=ccw(FIT[name]['tower_plan_xz']);center=p.mean(0)
    # A convex mapped envelope avoids separate overlapping native part skins.
    # Rounded corner samples preserve the sail tower's lenticular plan.
    q=[]
    for i,v in enumerate(p):
        prev=p[i-1];nxt=p[(i+1)%len(p)]
        a=v+(prev-v)*.12;b=v+(nxt-v)*.12
        for t in np.linspace(0,1,(5,3,2)[lod],endpoint=False):
            q.append((1-t)**2*a+2*t*(1-t)*v+t*t*b)
    q=ccw(q);_prism(m,q,.12,245,lod,103,(145,155,157))
    # Crown elevation rises toward the mapped north-west end; the architectural
    # sail is 172ft below the published 1100ft spire tip.
    axis=np.array([-.79,.61]);s=(q-center)@axis;s0,s1=s.min(),s.max()
    top=250+(335.28-172*.3048-250)*((s-s0)/(s1-s0))**.7
    tint=appearance.panel_tint(104,(156,162,160))
    for i,a in enumerate(q):
        j=(i+1)%len(q);b=q[j];normal=(b[1]-a[1],0,a[0]-b[0])
        face(m,[(a[0],245,a[1]),(b[0],245,b[1]),(b[0],top[j],b[1]),(a[0],top[i],a[1])],tint,104,normal)
        face(m,[(center[0],252,center[1]),(a[0],top[i],a[1]),(b[0],top[j],b[1])],(151,158,155),PANEL_WALL,(0,1,0))
    sp=q[np.argmax(s)]*.92+center*.08
    lathe(m,tuple(sp),[(274,1.25),(285,1.0),(310,.65),(328,.35),(335.28,.08)],(173,178,174),SCULPTURE_METAL,(12,8,6)[lod])
    return m


def _comcast(lod):
    name=NAMES[2];m=Mesh();p=_foundation(m,name)
    _prism(m,p,.12,14,lod,103,(161,168,164))
    body=_part(name,'16898846936607203610')
    _prism(m,body,14,277,lod,104,(154,164,166))
    for seed in ['16414775183476930553','5014060363124197221']:
        wing=_part(name,seed);_prism(m,wing,14,131,lod,104,(154,164,166))
    blade=_part(name,'4862792050857699906');bc=blade.mean(0)
    _prism(m,blade,14,339.1,lod,104,(168,173,169))
    # Upper mechanical enclosure and hotel roof lead into the 38m glass blade.
    hotel=body.mean(0)+(body-body.mean(0))*.58
    _prism(m,hotel,277,301.1,lod,107,(157,165,164))
    lathe(m,tuple(bc),[(339.1,.3),(341.4,.12)],(159,166,162),SCULPTURE_METAL,8)
    # Thirteen triple-height garden modules and a few major structural chevrons
    # retain the facade's identity without adding a second window grid.
    angle=math.atan2(p[1,1]-p[0,1],p[1,0]-p[0,0]);u=np.array([math.cos(angle),math.sin(angle)])
    v=np.array([-u[1],u[0]])
    for k in range(13 if lod<2 else 7):
        y=24+k*(16 if lod<2 else 32)
        c=body.mean(0)+u*26
        garden=ccw([c-u*3-v*7,c+u*3-v*7,c+u*3+v*7,c-u*3+v*7])
        roof(m,garden,y,(105,121,85),3)
        if lod==0:
            for dz in [-6.5,6.5]:
                z=c+v*dz
                m.beam((z[0],y,z[1]),(z[0],y+10,z[1]),.24,(164,169,164),PANEL_WALL,4)
    return m


BUILDERS=dict(zip(NAMES,[_bank,_wilshire,_comcast]))


def build(name,lod):
    return BUILDERS[name](lod)


ENTRIES=[]
for name,height,tip,city in zip(NAMES,[310.2864,335.28,339.1],[310.2864,335.28,341.4],['Los Angeles','Los Angeles','Philadelphia']):
    d=FIT[name];owned=[t.tolist()for t in plan_triangles(d['owned_plan_xz'])]
    ENTRIES.append({'name':name,'city':city,'lat':d['lat'],'lon':d['lon'],'heading':0,'base':d['base_m'],
        'suppress_buildings':True,'suppress_roads':True,'suppress_trees':True,
        'source_exclusion_polygons':owned,'road_exclusion_polygons':owned,'max_source_building_span_m':180,
        'terrain_flatten':[],
        'terrain_cuts':[{'polygon_xz':p,'plane':[0,0,-.08],'feather_m':1.0}for p in owned],
        'description':{'us-bank-tower':'Circular and orthogonal stepped granite office tower with a lobed crown and helipad.',
            'wilshire-grand-center':'Mapped glazed hotel and office tower, curved sail crown, offset spire and city-block podium.',
            'comcast-technology-center':'Stepped office and hotel volumes, triple-height gardens and a slender glass service blade.'}[name],
        'references':SOURCES[name],
        'attribution':'Original Low Poly Earth geometry; mapped footprints © OpenStreetMap contributors (ODbL), via Protomaps.',
        'audit':{'architectural_height_m':height,'tip_height_m':tip,'ground_datum_m':d['base_m'],
            'foundation_bottom_m':d['foundation_bottom_m'],'minimum_sampled_toe_burial_m':.6,
            'survey_file':str(REF/(name+'-ground.json')),'placement_file':str(REF/(name+'-placement.json')),
            'estimates':'Street-controlled fixed datum; published full height. Intermediate setback, podium, crown and mechanical levels are visual estimates or mapped part heights. Foundations extend below the lowest sampled perimeter; only owned footprints receive lower-only terrain clearance.'},
        'capture':{'distance':760,'azimuth':225 if name!='comcast-technology-center'else 135,'elevation':24,'target_height':155}})

DEPENDENCIES=[Path(__file__).parent/'geometry.py',Path(__file__).parent/'east_asia_geometry.py',
    Path(appearance.__file__),*appearance.DEPENDENCIES,
    Path(__file__).parents[1]/'heritage_catalog/geometry.py',Path(__file__).parents[1]/'heritage_catalog/materials.py',
    *sorted(REF.glob('*.json'))]
