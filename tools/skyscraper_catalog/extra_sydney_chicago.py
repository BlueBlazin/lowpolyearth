"""Mapped Chicago setbacks/frusta and Crown Sydney's three-petal envelope."""
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.pacific_geometry import (
    Mesh, face, clean, part, site, support, envelope, roof, slab, make_entry,
    PANEL_WALL, SCULPTURE_METAL,
)

GROUP = 'extra_sydney_chicago'
NAMES = ('crown-sydney', 'st-regis-chicago', 'trump-international-hotel-tower-chicago')
SITES = {name: site(GROUP, name) for name in NAMES}
BASES = dict(zip(NAMES, (3.7, 177.0, 177.5)))
PARENTS = dict(zip(NAMES, (581013658, 464802811, 64594680)))
REFDIR = Path(__file__).parent/'references'/GROUP
DEPENDENCIES = [Path(__file__), Path(__file__).with_name('pacific_geometry.py'),
                *sorted(REFDIR.glob('*.json'))]
from skyscraper_catalog import crown_sydney
DEPENDENCIES += crown_sydney.DEPENDENCIES


def _skin(m, rings, lod, glass, metal, material=103):
    """Loft disjoint glass/pier strips; no hidden near-coplanar backing wall."""
    for low, high in zip(rings, rings[1:]):
        for i in range(len(low)):
            j = (i+1) % len(low)
            a,b,c,d = low[i],low[j],high[j],high[i]
            edge = b-a
            length = np.linalg.norm(edge[[0,2]])
            if length < .05:
                continue
            normal = (edge[2], 0, -edge[0])
            count = max(1, min((12,6,1)[lod], int(length/(4,8,10000)[lod])))
            tint = tuple(round(.62*g+.38*w) for g,w in zip(glass,metal))
            for k in range(count):
                start,end = k/count,(k+1)/count
                pier = min(.16/length, (end-start)/8)
                bands = [(start,start+pier,metal,PANEL_WALL),
                         (start+pier,end-pier,tint,material),
                         (end-pier,end,metal,PANEL_WALL)] if lod < 2 and length > 4 else [(start,end,tint,material)]
                for u,v,color,mat in bands:
                    face(m,[a+(b-a)*u,a+(b-a)*v,d+(c-d)*v,d+(c-d)*u],color,mat,normal)
    top=rings[-1]
    center=top.mean(axis=0)
    for a,b in zip(top,np.roll(top,-1,axis=0)):
        face(m,[center,a,b],(133,145,146),91,(0,1,0))


def _crown(lod):
    from skyscraper_catalog.crown_sydney import build
    return build(lod,SITES['crown-sydney'])


def _frustum(m,p,lo,height,lod):
    center=p.mean(axis=0)
    # OSM plan is the wide floor plate; alternating twelve-storey frusta
    # retain their real sloping faces at all three distances.
    levels=list(np.arange(lo,height,43.8))+[height]
    for i,(bottom,top) in enumerate(zip(levels,levels[1:])):
        scales=(.79,1) if i%2==0 else (1,.79)
        q0=center+(p-center)*scales[0];q1=center+(p-center)*scales[1]
        a=np.array([(x,bottom,z) for x,z in q0]);b=np.array([(x,top,z) for x,z in q1])
        tone=i%6
        _skin(m,[a,b],lod,(98+tone*3,139+tone*2,153+tone),(154,173,176),103)


def _st_regis(lod):
    data=SITES[NAMES[1]];m=Mesh();p=part(data,PARENTS[NAMES[1]])
    support(m,data,p,BASES[NAMES[1]])
    west=part(data,465242989);middle=part(data,465242991);east=part(data,465242992)
    _frustum(m,west,0,362.9,lod)
    _frustum(m,middle,12,259,lod)
    _frustum(m,east,0,179,lod)
    # The center volume is lifted above a public passage. Its underside is
    # authored explicitly, while the ground-level plaza remains unobstructed.
    q=middle.mean(axis=0)+(middle-middle.mean(axis=0))*.79
    slab(m,q,12,.8,(147,164,165))
    low=part(data,465242990)
    envelope(m,low,0,59,lod,(102,143,157),(156,174,177),material=103)
    return m


def _coarse_ring(p):
    keep=[]
    for i,q in enumerate(p):
        u=q-p[i-1];v=p[(i+1)%len(p)]-q
        cosine=np.dot(u,v)/max(1e-9,np.linalg.norm(u)*np.linalg.norm(v))
        if i%2==0 or cosine<.94:
            keep.append(q)
    return clean(keep)


def _trump(lod):
    data=SITES[NAMES[2]];m=Mesh();p=part(data,PARENTS[NAMES[2]])
    support(m,data,p,BASES[NAMES[2]])
    tiers=((64594680,0,60),(188338549,60,120),(188338550,120,200),
           (188338548,200,345),(188338859,345,357))
    for identifier,lo,hi in tiers:
        q=part(data,identifier)
        if lod==2:q=_coarse_ring(q)
        envelope(m,q,lo,hi,0 if lod==0 else 2,(111,143,156),(164,178,182),material=104)
    # The mapped centered mast changes diameter at each upper stage.
    for identifier,lo,hi in ((188356529,357,380),(284773992,380,400),(284773991,400,423.2)):
        q=part(data,identifier)
        if lod==2:q=_coarse_ring(q)
        envelope(m,q,lo,hi,0 if lod==0 else 2,(164,179,181),(180,188,185),material=104)
    return m


BUILDERS=dict(zip(NAMES,(_crown,_st_regis,_trump)))

def build(name,lod):
    if lod not in (0,1,2):
        raise ValueError(lod)
    return BUILDERS[name](lod)


ENTRIES=[]
for name,city,height,description,az in (
    (NAMES[0],'Sydney',271.3,'Crown Sydney: architect-drawn hotel wing, rotating residential floor plates, unequal curved crown and recessed waterfront podium.',270),
    (NAMES[1],'Chicago',362.9,'St. Regis Chicago: three connected graduated towers of alternating twelve-storey frusta, raised central passage and low eastern wing.',25),
    (NAMES[2],'Chicago',423.2,'Trump International Hotel and Tower Chicago: mapped rounded setbacks and a slender stepped architectural spire.',140),
):
    data=SITES[name];p=part(data,PARENTS[name]);center=(p.min(axis=0)+p.max(axis=0))/2
    entry=make_entry(data,city,BASES[name],[p],height,description,
                     [str(REFDIR/'architectural-evidence.json'),str(REFDIR/(name+'-site.json'))],
                     {'ground_basis':'Estimated local entrance datum from clean production terrain samples; exact sampled support toes buried at least 0.5m. Upper heights remain relative to architectural entrance datum.',
                      'plan_basis':'Complete selected OSM parent and building-part rings; all source measurements retained with raw-response SHA256.',
                      'footing_sample_count':len(data.get('footing_ground_samples',[]))},
                     {'distance':height*2.15,'azimuth':az,'elevation':18,'target_height':height*.45,'target_xz':center.tolist()})
    if name == 'crown-sydney':
        measured=crown_sydney.reference()
        ground=[crown_sydney.world(q,measured) for q in measured['ground_support_uv_m']]
        entry=make_entry(data,city,BASES[name],ground,height,description,
            [str(crown_sydney.REFERENCE),str(REFDIR/(name+'-site.json'))],
            {'ground_basis':'Architect entrance RL3.70; production terrain sampled at every foundation toe, buried at least0.5m.',
             'plan_basis':'Registered WilkinsonEyre / Bates Smart floor plans; translation only against retained OSM site perimeter.',
             'footing_sample_count':len(data['footing_ground_samples'])},
            {'distance':550,'azimuth':225,'elevation':14,'target_height':132,
             'target_xz':measured['plan_translation_xz_m']})
        entry['attribution']=measured['attribution']
        from skyscraper_catalog.geometry import plan_triangles
        projection=crown_sydney.world(measured['overall_projection_uv_m'][0],measured)
        entry['terrain_cuts']=[{'polygon_xz':q.tolist(),'plane':[0,0,-.1],'feather_m':1.5}
                               for q in plan_triangles(clean(projection))]
    ENTRIES.append(entry)
