"""Bled Island: mapped architecture and a shoreline-bounded relief model.

Horizontal dimensions follow OSM; unmeasured heights and relief are photographic
estimates. Only the island's missing rise is reconstructed, never lake terrain.
"""
import json
import math
from functools import lru_cache
from pathlib import Path

import numpy as np
from city_catalog.common import Mesh
from heritage_catalog import castles, materials as MAT
from heritage_catalog.castles import CastleMesh, rectangle_fit, roof, aperture
from heritage_catalog.geometry import (
    face, plan_triangles, plan_solid, turn, arch, lathe, tube, cross, ring,
)

ROOT = Path(__file__).resolve().parents[2]
REF = Path(__file__).parent/'references/islands'
DEPENDENCIES = [Path(castles.__file__), ROOT/'tools/build_island_catalog.py']
PLAN = json.loads((REF/'bled-plan.json').read_text())
GROUND = json.loads((REF/'bled-ground.json').read_text())
BASE = GROUND['base_m']
CREAM = (231, 225, 207)
WHITE = (240, 236, 221)
PINK = (188, 130, 121)
STONE = (166, 161, 146)
TRIM = (199, 198, 178)
TILE = (189, 82, 44)
COPPER = (69, 85, 79)
DARK = (45, 47, 39)
WOOD = (98, 67, 43)
GOLD = (193, 146, 54)


def way(ident):
    return next(w for w in PLAN['ways'] if w['id'] == ident)


def points(ident):
    p = np.asarray(way(ident)['xz'], float)
    return p[:-1] if np.array_equal(p[0], p[-1]) else p


def raw_height(xz):
    p = np.asarray(xz, float)
    xs, zs = np.asarray(GROUND['x_m']), np.asarray(GROUND['z_m'])
    a = np.asarray(GROUND['heights_m'])
    x, z = np.clip(p[..., 0], xs[0], xs[-1]), np.clip(p[..., 1], zs[0], zs[-1])
    i = np.clip(np.searchsorted(xs, x)-1, 0, len(xs)-2)
    j = np.clip(np.searchsorted(zs, z)-1, 0, len(zs)-2)
    u, v = (x-xs[i])/2, (z-zs[j])/2
    return (1-u)*(1-v)*a[j,i]+u*(1-v)*a[j,i+1]+(1-u)*v*a[j+1,i]+u*v*a[j+1,i+1]-BASE


def relief(xz):
    """Estimated island crest, constrained by shore and stair landing heights."""
    p = np.asarray(xz, float)
    shore = points(8149592)
    a, b = shore, np.roll(shore, -1, axis=0)
    v = b-a
    t = np.clip(np.sum((p[...,None,:]-a)*v, axis=-1)/np.sum(v*v,axis=-1),0,1)
    distance = np.linalg.norm(p[...,None,:]-a-t[...,None]*v,axis=-1).min(axis=-1)
    crest = np.interp(p[...,1], [-105,-99,-68,-45,14,45,71], [0,.8,16.6,18.5,18.5,1,0])
    return raw_height(p)+np.minimum(distance*.96, crest)


@lru_cache(None)
def grades():
    # Splitting the longest edge maintains the mapped outer boundary. All
    # neighbors interpolate identical endpoint heights along shared edges.
    triangles = []
    def split(t):
        lengths = [np.linalg.norm(t[(i+1)%3]-t[i]) for i in range(3)]
        i = int(np.argmax(lengths))
        if lengths[i] > 11:
            a,b,c=t[i],t[(i+1)%3],t[(i+2)%3]
            m=(a+b)/2
            split(np.asarray([a,m,c]));split(np.asarray([m,b,c]))
        else:
            h=relief(t)
            plane=np.linalg.solve(np.column_stack([t,np.ones(3)]),h)
            triangles.append({'polygon_xz': t.tolist(), 'plane': plane.tolist(), 'feather_m': .03})
    for t in plan_triangles(points(8149592)):
        split(t)
    return triangles


@lru_cache(None)
def grade_arrays():
    t=np.asarray([g['polygon_xz'] for g in grades()])
    return t,np.roll(t,-1,axis=1)-t,np.asarray([g['plane'] for g in grades()])


@lru_cache(None)
def cached_height(x,z):
    t,v,planes=grade_arrays();d=np.array([x,z])-t
    c=v[:,:,0]*d[:,:,1]-v[:,:,1]*d[:,:,0]
    hits=np.flatnonzero((c>=-1e-7).all(axis=1)|(c<=1e-7).all(axis=1))
    return float(planes[hits[0]]@[x,z,1]) if len(hits) else float(raw_height([x,z]))


def height(xz):
    """Use exactly the baked terrain triangles for architectural grounding."""
    return cached_height(float(xz[0]),float(xz[1]))


def foundation(m, polygon, top, color=STONE):
    # Separate retaining courses expose stone at downhill walls; there is no
    # enclosing island slab. Every wall toe extends into its terrain triangle.
    plan_solid(m,polygon,top-.22,top,color,MAT.STONE_WALL)
    for a,b in zip(polygon,np.roll(polygon,-1,axis=0)):
        count=max(1,math.ceil(np.linalg.norm(b-a)/1.5))
        for i in range(count):
            p=a+(b-a)*i/count;q=a+(b-a)*(i+1)/count
            yp,yq=min(height(p)-.5,top-.3),min(height(q)-.5,top-.3)
            edge=q-p
            winding=np.sign(sum(u[0]*v[1]-u[1]*v[0]for u,v in zip(polygon,np.roll(polygon,-1,axis=0))))
            face(m,[(p[0],yp,p[1]),(q[0],yq,q[1]),(q[0],top,q[1]),(p[0],top,p[1])],color,MAT.STONE_WALL,(winding*edge[1],0,-winding*edge[0]))


def building_base(ident):
    p=points(ident)
    samples=[a+(b-a)*t for a,b in zip(p,np.roll(p,-1,axis=0)) for t in np.linspace(0,1,10)]
    return max(height(p) for p in samples)+.18


def band(m,w,d,y,thick=.16,color=WHITE):
    m.box((0,y,0),(w+.18,thick,d+.18),color,MAT.PLASTER_WALL)


def clock_face(m,x,y,z,lod):
    m.box((x,y,z),(3.55,3.55,.12),GOLD,MAT.PLASTER_WALL)
    m.box((x,y,z-.09),(3.22,3.22,.10),WHITE,MAT.PLASTER_WALL)
    n=(48,32,20)[lod]
    center=(x,y,z-.17)
    for i in range(n):
        a,b=i*math.tau/n,(i+1)*math.tau/n
        face(m,[center,(x+1.32*math.sin(a),y+1.32*math.cos(a),z-.17),(x+1.32*math.sin(b),y+1.32*math.cos(b),z-.17)],CREAM,MAT.PLASTER_WALL,(0,0,-1))
    ring(m,center,1.4,.035,DARK,n,16,'xy')
    for i in range(12):
        a=i*math.tau/12
        for dr in ([-.065,.065] if i%3 else [-.11,0,.11]):
            q=a+dr
            tube(m,(x+1.09*math.sin(q),y+1.09*math.cos(q),z-.21),(x+1.27*math.sin(q),y+1.27*math.cos(q),z-.21),.024,DARK,16,4)
    tube(m,(x,y,z-.24),(x-.64,y+.42,z-.24),.045,DARK,16,5)
    tube(m,(x,y,z-.24),(x+.34,y+.91,z-.24),.032,DARK,16,5)
    if lod<2:
        for sx in (-1,1):
            for sy in (-1,1):
                ring(m,(x+sx*1.42,y+sy*1.42,z-.2),.18,.035,GOLD,12,17,'xy')


def tower(lod):
    ident=261628609;x,z,w,d,h=rectangle_fit(points(ident));base=building_base(ident)
    m=CastleMesh(MAT.PLASTER_WALL,MAT.METAL_ROOF)
    m.box((0,15.8,0),(w,31.6,d),CREAM)
    for y in [1,25.8,31.6,32.0]:band(m,w,d,y,.26)
    for heading,width,depth in [(0,w,d),(180,w,d),(90,d,w),(270,d,w)]:
        f=Mesh()
        # Red painted quoins alternate long and short corner blocks.
        for y in np.arange(1.4,31.2,.85):
            k=int(y/.85)
            for side in (-1,1):
                widthq=.83 if k%2 else .52
                f.box((side*(width/2-widthq/2),y,-depth/2-.035),(widthq,.58,.065),PINK,83)
        for y in [5,12,19,23.1]:
            aperture(f,0,y,-depth/2-.045,.42,.92,TRIM,lod)
        for xx in [-1.18,1.18]:
            arch(f,(xx,27.2,-depth/2-.08),1.7,3.6,.17,TRIM,False,(20,14,10)[lod],DARK,82)
            for y in np.arange(27.35,29.85,.24 if lod==0 else .48):
                f.box((xx,y,-depth/2-.11),(1.50,.115,.10),WOOD,84)
        clock_face(f,0,24.2,-depth/2-.1,lod)
        if heading==0:arch(f,(0,0,-depth/2-.08),1.45,2.55,.22,TRIM,False,20,WOOD,82)
        m.merge(turn(f,heading))
    # Bell-shaped metal skirt, octagonal lantern, curved cap and slender spire.
    skirt=[(32.0,5.9),(32.5,5.6),(33.2,4.8),(34.2,3.5),(35.2,2.8),(35.7,2.75)]
    lathe(m,(0,0),skirt,COPPER,88,4,math.pi/4)
    for y,r in [(35.7,2.7),(36.1,2.7),(41.0,2.75),(41.5,2.8)]:
        lathe(m,(0,0),[(y,r),(y+.18,r)],TRIM if y==35.7 else COPPER,88,8,math.pi/8)
    lathe(m,(0,0),[(36.2,2.40),(41,2.40)],DARK,88,8,math.pi/8)
    for i in range(8):
        a=i*math.tau/8+math.pi/8
        tube(m,(2.46*math.cos(a),36,2.46*math.sin(a)),(2.46*math.cos(a),41.2,2.46*math.sin(a)),.13,COPPER,88,6)
    profile=[(41.4,3.0),(41.9,2.8),(42.5,2.0),(43.1,1.6),(43.8,1.45),(44.6,1.22),(48.2,.30),(49.0,.14)]
    lathe(m,(0,0),profile,COPPER,88,(16,12,8)[lod],math.pi/8)
    lathe(m,(0,0),[(48.9,.16),(49.2,.42),(49.55,.16)],GOLD,16,12)
    cross(m,(0,49.5,0),2.5,1.22,.105,GOLD)
    # Narrow seam ribs follow each metal roof panel, including concave skirt.
    if lod<2:
        for profile,n,angle in [(skirt,4,math.pi/4),(profile,8,math.pi/8)]:
            for i in range(n):
                a=i*math.tau/n+angle
                for (y,r),(yy,rr) in zip(profile,profile[1:]):
                    tube(m,(r*math.cos(a),y+.02,r*math.sin(a)),(rr*math.cos(a),yy+.02,rr*math.sin(a)),.045,(90,101,90),88,4)
    out=Mesh();foundation(out,points(ident),base)
    out.merge(turn(m,h,x,base,z));return out


def church(lod):
    ident=261628608;base=building_base(ident)
    # Nave axes from the mapped west facade and south wall; apse and chapels
    # retain the concave source footprint instead of a rectangular envelope.
    origin=np.array([-14.273,-3.008]);heading=-12.77
    c,s=math.cos(math.radians(heading)),math.sin(math.radians(heading))
    local=(points(ident)-origin)@np.array([[c,s],[-s,c]])
    m=CastleMesh(83,85);plan_solid(m,local,0,9.5,CREAM,83)
    roof(m,14.65,0,29.9,12.3,9.5,15.0,TILE,True,lod,CREAM)
    # Northern and southern side chapels have their own lower pitched roofs.
    for x,z,w,d in [(14.35,7.35,7.2,4.2),(14.35,-7.3,7.2,3.5),(25.3,4.0,8.6,14.0)]:
        roof(m,x,z,w,d,9.52,12.2,TILE,True,lod,CREAM)
    for side in (-1,1):
        f=Mesh()
        for x in [3.4,8.4,19.8,24.2]:
            arch(f,(x,4.7,-5.92),1.48,2.8,.14,TRIM,False,(20,14,10)[lod],DARK,82)
        for x in [.5,5.9,11.8,18.0,23.9]:
            f.box((x,4.7,-5.93),(.45,9.2,.17),WHITE,83)
        f.box((12,8.9,-5.99),(24,.23,.21),WHITE,83)
        if side==1:
            for p,_,_ in f.triangles:p[:,2]*=-1;p[:]=p[::-1]
        m.merge(f)
    west=Mesh();arch(west,(0,.1,-.08),2.25,3.6,.26,TRIM,False,24,WOOD,82)
    aperture(west,0,5.1,-.12,1.8,2.4,TRIM,lod)
    # West entrance faces west; the chapel's long axis runs east-west.
    m.merge(turn(west,90,0,0,0))
    for x,z in [(5,0),(25.5,3)]:cross(m,(x,14.8 if x==5 else 12.1,z),1.25,.65,.065,(96,85,58))
    # Small ridge bellcote belonging to the church, separate from the tower.
    m.box((16,15.2,0),(1.6,1.2,1.6),CREAM,83)
    lathe(m,(16,0),[(15.8,1.15),(16.2,1.1),(17.0,.3),(17.3,.04)],COPPER,88,4,math.pi/4)
    out=Mesh();foundation(out,points(ident),base)
    out.merge(turn(m,heading,*[float(v)for v in [origin[0],base,origin[1]]]))
    return out


def house(ident,lod,eave,peak,floors,hip=True):
    x,z,w,d,h=rectangle_fit(points(ident));base=building_base(ident)
    m=CastleMesh(83,85);m.box((0,eave/2,0),(w,eave,d),CREAM)
    roof(m,0,0,w+.6,d+.6,eave,peak,TILE,hip,lod,CREAM)
    for level in range(floors+1):band(m,w,d,level*eave/floors,.12)
    for angle,width,depth in [(0,w,d),(180,w,d),(90,d,w),(270,d,w)]:
        f=Mesh();cols=max(2,round(width/3.7))
        for row in range(floors):
            for xx in np.linspace(-width/2+1.7,width/2-1.7,cols):
                yy=.85+row*eave/floors
                # Warm pink recessed cassette surrounds distinguish Sexton's.
                if ident==261628607:
                    f.box((xx,yy+.9,-depth/2-.04),(1.95,2.28,.08),(197,168,153),83)
                aperture(f,xx,yy,-depth/2-.10,.92,1.55,TRIM,lod)
                if lod<2:f.box((xx,yy+.8,-depth/2-.12),(.055,1.5,.045),WHITE,17)
        for side in (-1,1):
            f.box((side*(width/2-.18),eave/2,-depth/2-.04),(.30,eave,.10),WHITE,83)
        if angle==0:arch(f,(0,0,-depth/2-.15),1.4,2.3,.15,TRIM,False,16,WOOD,82)
        m.merge(turn(f,angle))
    # Masonry chimney stacks and terra-cotta caps.
    if eave>6:
        m.box((w*.24,peak-.3,d*.22),(.7,2.0,.75),CREAM,83)
        m.box((w*.24,peak+.73,d*.22),(.88,.17,.9),TILE,85)
    out=Mesh();foundation(out,points(ident),base)
    out.merge(turn(m,h,x,base,z));return out


def chapel(ident,lod):
    x,z,w,d,h=rectangle_fit(points(ident));base=building_base(ident)
    m=CastleMesh(82 if ident==977653514 else 83,85)
    eave,peak=(2.8,4.1) if ident==977653514 else (3.7,7.3)
    color=STONE if ident==977653514 else CREAM
    m.box((0,eave/2,0),(w,eave,d),color)
    roof(m,0,0,w+.4,d+.4,eave,peak,TILE,ident==424606788,lod,color)
    f=Mesh();arch(f,(0,.25,-d/2-.12),min(w*.55,1.5),2.25,.18,TRIM,False,(20,14,10)[lod],DARK,82)
    # Small Marian figure in the recessed niche, with a blue mantle.
    lathe(f,(0,-d/2-.17),[(.48,.3),(.95,.21),(1.45,.13)],(97,125,147),17,(12,8,6)[lod])
    lathe(f,(0,-d/2-.17),[(1.45,.08),(1.62,.15),(1.8,.04)],CREAM,17,(12,8,6)[lod])
    m.merge(f);cross(m,(0,peak,0),.65,.35,.05,(90,78,58))
    out=Mesh();foundation(out,points(ident),base)
    out.merge(turn(m,h,x,base,z));return out


def pavilion(lod):
    ident=977031768;base=building_base(ident);p=points(ident)
    m=Mesh();foundation(m,p,base)
    plan_solid(m,p,base,base+2.9,CREAM,83)
    plan_solid(m,p,base+2.9,base+3.1,(104,138,131),88)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        length=np.linalg.norm(b-a)
        if length<3:continue
        n=max(1,int(length/1.7))
        for i in range(n):
            u=a+(b-a)*(i+.08)/n;v=a+(b-a)*(i+.92)/n;edge=b-a
            # Both sides make the curtain-glass infill robust to plan winding.
            for normal in [(edge[1],0,-edge[0]),(-edge[1],0,edge[0])]:
                off=np.asarray(normal)[[0,2]]/length*.025
                face(m,[(u[0]+off[0],base+.45,u[1]+off[1]),(v[0]+off[0],base+.45,v[1]+off[1]),(v[0]+off[0],base+2.65,v[1]+off[1]),(u[0]+off[0],base+2.65,u[1]+off[1])],(75,98,92),15,normal)
    return m


def retaining_wall(m,a,b,top_a,top_b,width=.45):
    a,b=np.asarray(a,float),np.asarray(b,float);v=b-a;length=np.linalg.norm(v)
    n=np.array([v[1],-v[0]])/length*width/2
    count=max(1,math.ceil(length/2))
    for i in range(count):
        t,u=i/count,(i+1)/count;p,q=a+v*t,a+v*u
        ya=max(top_a+(top_b-top_a)*t, height(p)+.65)
        yb=max(top_a+(top_b-top_a)*u, height(q)+.65)
        for side in (-1,1):
            pp,qq=p+side*n,q+side*n
            face(m,[(pp[0],height(pp)-.4,pp[1]),(qq[0],height(qq)-.4,qq[1]),(qq[0],yb,qq[1]),(pp[0],ya,pp[1])],STONE,82,(side*n[0],0,side*n[1]))
        face(m,[(p[0]-n[0],ya,p[1]-n[1]),(p[0]+n[0],ya,p[1]+n[1]),(q[0]+n[0],yb,q[1]+n[1]),(q[0]-n[0],yb,q[1]-n[1])],TRIM,82)


def stairway(ident,lod):
    path=points(ident)
    if ident==438264032:path=path[::-1]
    a,b=path[0],path[-1];length=np.linalg.norm(b-a);v=(b-a)/length;n=np.array([v[1],-v[0]])
    count=int(way(ident)['tags']['step_count']);width=5.2 if count==99 else 2.4
    # Treads track a smooth rising envelope above the terrain under both edges.
    samples=np.linspace(0,1,count+1)
    heights=np.array([max(height(a+(b-a)*t+side*n*width/2)for side in [-1,0,1])+.24 for t in samples])
    heights=np.maximum.accumulate(heights)
    m=Mesh()
    for i in range(count):
        p,q=a+(b-a)*samples[i],a+(b-a)*samples[i+1]
        # The bottom of the southern flight fans out onto its landing.
        wa=width/2+(max(0,1-i/17)*2.6 if count==99 else 0)
        wb=width/2+(max(0,1-(i+1)/17)*2.6 if count==99 else 0)
        poly=np.array([p-n*wa,p+n*wa,q+n*wb,q-n*wb])
        y=heights[i+1]
        plan_solid(m,poly,min(height(p) for p in poly)-.3,y,tuple(v+(i%4-2)*2 for v in STONE),82)
        if lod<2:
            face(m,[(p[0]-n[0]*wa,y+.006,p[1]-n[1]*wa),(p[0]+n[0]*wa,y+.006,p[1]+n[1]*wa),(p[0]+n[0]*wa+v[0]*.055,y+.006,p[1]+n[1]*wa+v[1]*.055),(p[0]-n[0]*wa+v[0]*.055,y+.006,p[1]-n[1]*wa+v[1]*.055)],TRIM,82)
        for side in (-1,1):retaining_wall(m,p+side*n*(wa+.18),q+side*n*(wb+.18),y+.65,y+.65,.36)
    # Stone quay only where the real landing meets the staircase.
    poly=np.array([a-n*(width/2+3)-v*3,a+n*(width/2+3)-v*3,a+n*(width/2+3),a-n*(width/2+3)])
    foundation(m,poly,float(heights[0]))
    return m


def terraces(lod):
    m=Mesh()
    # Narrow paved routes and the irregular fenced church platform; grass
    # between the church and bell tower stays exposed.
    routes=[([[-17,-9],[-15,-13],[-9,-14],[-5,-26],[-3,-60],[-3.35,-67.9]],2.1),
            ([[-16,-8],[-18,1],[-14,12],[-12.9,14.2]],2.0),
            ([[14,9],[18,2],[14,-9],[9,-16],[-5,-26]],1.8)]
    for path,width in routes:
        for a,b in zip(path,path[1:]):
            a,b=np.asarray(a,float),np.asarray(b,float);delta=b-a;n=np.array([delta[1],-delta[0]])/np.linalg.norm(delta)*width/2
            for t in np.arange(0,1,.1):
                p,q=a+delta*t,a+delta*min(1,t+.1)
                poly=[p-n,p+n,q+n,q-n]
                face(m,[(x,height([x,z])+.16,z)for x,z in poly],(192,188,173),82)
    for path in [[[-19,-8],[-20,1],[-16,13],[1,17],[16,14],[20,4],[17,-11],[11,-20]],
                 [[-18,-74],[-18,-80],[-15,-84],[-7,-84]],
                 [[-20,-80],[-20,-89],[-8,-91]]]:
        for a,b in zip(path,path[1:]):
            retaining_wall(m,a,b,height(a)+.8,height(b)+.8)
    return m


def build(name,lod):
    assert name=='bled-island' and lod in range(3)
    m=Mesh();m.merge(church(lod));m.merge(tower(lod))
    for spec in [(261628606,10.2,14.5,3,True),(261628607,8.8,13,3,False),
                 (261628610,6.1,9.9,2,True),(261628611,3.5,5.5,1,False),
                 (977554913,2.8,4.0,1,True)]:
        ident,eave,peak,floors,hip=spec;m.merge(house(ident,lod,eave,peak,floors,hip))
    for ident in [424606788,977653514]:m.merge(chapel(ident,lod))
    m.merge(pavilion(lod))
    for ident in [234052723,438264032]:m.merge(stairway(ident,lod))
    m.merge(terraces(lod))
    return m


def architectural_clearings():
    # The source wood polygon covers the whole island, including the actual
    # fenced summit court and stone approaches. Keep the surrounding banks.
    summit=[[-19,-8],[-20,1],[-16,13],[1,17],[16,14],[20,4],[17,-11],[13,-27],[-4,-28]]
    polygons=[t.tolist() for t in plan_triangles(summit)]
    for ident in [234052723,438264032]:
        p=points(ident);a,b=p[0],p[-1];v=b-a
        n=np.array([v[1],-v[0]])/np.linalg.norm(v)*(3.2 if ident==234052723 else 1.7)
        polygons.append(np.asarray([a-n,a+n,b+n,b-n]).tolist())
    polygons.append([[-5.4,-69],[-.9,-69],[1,-25],[-4,-25]])
    return polygons


BUILDINGS=[w['id']for w in PLAN['ways']if 'building'in w['tags']]
ENTRIES=[{'name':'bled-island','lat':46.36245,'lon':14.09015,'heading':0.,'base':BASE,
    'suppress_buildings':True,'suppress_roads':False,'suppress_trees':True,
    'suppress_bbox':[[-52,-108],[37,72]],'activation_bounds_xz':[[-53,-109],[38,73]],
    'source_exclusion_polygons':[t.tolist()for ident in BUILDINGS for t in plan_triangles(points(ident))]+architectural_clearings(),
    'max_source_building_span_m':45.,'terrain_grades':grades(),
    'references':['https://www.blejskiotok.si/en/attractions/',
                  'https://www.blejskiotok.si/en/attractions/cultural-heritage/',
                  PLAN['source_url'] if 'source_url' in PLAN else 'https://api.openstreetmap.org/api/0.6/map?bbox=14.0878,46.3608,14.0923,46.3641'],
    'notes':'Mapped footprints and shoreline from OSM (ODbL). Tower 52 m and southern stair count 99 documented; north 78 steps mapped. Remaining vertical dimensions, roof profiles, details and island rise (18.5 m maximum above raw DEM) are photographic estimates, not an elevation survey. Relief is bounded by the mapped island; no lake corrections, rectangular flatten patches or island-wide vegetation suppression.',
    'building_way_ids':BUILDINGS,'stairs':{'234052723':99,'438264032':78}}]
