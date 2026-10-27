"""Metric fortress architecture with geographic plans and bounded relief detail.

Coordinates are east/up/north at heading zero. Site measurements and remaining
interpretive dimensions live in references/fortresses; no mesh is downloaded.
"""
import json
import math
from functools import lru_cache
from pathlib import Path

import numpy as np
from city_catalog.common import Mesh
from heritage_catalog.materials import (
    WEATHERED_LIMESTONE, VEINED_MARBLE, BRICK_WALL, STONE_WALL,
    TIMBER_WALL, TILE_ROOF, SCULPTURE_METAL, retag,
)

REF = Path(__file__).with_name('references') / 'fortresses'
STONE = (179, 162, 132)
LIME = (214, 204, 179)
SAND = (196, 159, 111)
RED = (151, 73, 49)
BRICK = (162, 94, 59)
DARK = (45, 43, 37)
TEAL = (40, 118, 133)
OCHRE = (220, 166, 55)
MARBLE = (230, 225, 207)
TILE = (65, 73, 74)


def face(m, points, color, normal=(0, 1, 0), material=11):
    p = np.asarray(points, float)
    # Scanline strips may collapse one end: orient each surviving triangle.
    for i in range(1, len(p)-1):
        a,b,c=p[0],p[i],p[i+1]
        if np.dot(np.cross(b-a,c-a),normal)<0:b,c=c,b
        m.triangle(a,b,c,color,material)


def turn(m, degrees, x=0, y=0, z=0):
    a = math.radians(degrees)
    c, s = math.cos(a), math.sin(a)
    return m.transform([[c,0,s,x],[0,1,0,y],[-s,0,c,z],[0,0,0,1]])


def lathe(m, x, z, profile, color, n=24, material=11):
    for k, ((y0,r0),(y1,r1)) in enumerate(zip(profile, profile[1:])):
        for j in range(n):
            a, b = j*math.tau/n, (j+1)*math.tau/n
            c = color(k,j) if callable(color) else color
            m.quad((x+r0*math.cos(a),y0,z+r0*math.sin(a)),
                   (x+r1*math.cos(a),y1,z+r1*math.sin(a)),
                   (x+r1*math.cos(b),y1,z+r1*math.sin(b)),
                   (x+r0*math.cos(b),y0,z+r0*math.sin(b)),c,material)
    for (y,r), sign in [(profile[0],-1),(profile[-1],1)]:
        c = color(0,0) if callable(color) else color
        for j in range(n):
            a,b=j*math.tau/n,(j+1)*math.tau/n
            face(m,[(x,y,z),(x+r*math.cos(a),y,z+r*math.sin(a)),
                    (x+r*math.cos(b),y,z+r*math.sin(b))],c,(0,sign,0),material)


def dome(m, x, y, z, radius, height, color, lod, onion=False, material=12):
    profile=[]
    for j in range((12,8,5)[lod]+1):
        t=j/(12,8,5)[lod]
        r=radius*math.cos(t*math.pi/2)
        if onion: r*=1+.24*math.sin(t*math.pi)
        profile.append((y+t*height,max(r,.02)))
    lathe(m,x,z,profile,color,(32,20,12)[lod],material)


def arch(m, x,y,z, width,height,color, lod, pointed=False, depth=.15):
    """Dark inset aperture with individually colored voussoirs; faces south."""
    n=(16,10,6)[lod]
    spring=height-width/2 if not pointed else height*.52
    curve=[]
    for j in range(n+1):
        t=math.pi-j*math.pi/n
        xx=width*.5*math.cos(t)
        yy=spring+width*.5*math.sin(t)
        if pointed: yy=spring+(height-spring)*math.sqrt(max(0,1-(abs(xx)/(width/2))**.8))
        curve.append((xx,yy))
    poly=[(-width/2,0)]+curve+[(width/2,0)]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        face(m,[(x,y+height*.4,z),(x+a[0],y+a[1],z),(x+b[0],y+b[1],z)],DARK,(0,0,-1),15)
    if lod<2:
        for i,(a,b) in enumerate(zip(curve,curve[1:])):
            aa,bb=np.array(a),np.array(b);v=bb-aa;v=np.array([-v[1],v[0]])/np.linalg.norm(v)*.16
            face(m,[(x+aa[0]-v[0],y+aa[1]-v[1],z-depth),
                    (x+bb[0]-v[0],y+bb[1]-v[1],z-depth),
                    (x+bb[0]+v[0],y+bb[1]+v[1],z-depth),
                    (x+aa[0]+v[0],y+aa[1]+v[1],z-depth)],
                 tuple(min(255,u+(i%3-1)*6) for u in color),(0,0,-1),17)
    for xx in [-width/2,width/2]:
        m.box((x+xx,y+spring/2,z-depth),(.23,max(.2,spring),.25),color,17)


def aperture_sides(m,x,y,z,w,d,h,lod,color,spacing=5):
    for angle,ww,zz in [(0,w,-d/2),(180,w,-d/2),(90,d,-w/2),(270,d,-w/2)]:
        panel=Mesh()
        for xx in np.arange(-ww/2+spacing*.65,ww/2-.5,spacing):
            for yy in np.arange(1.2,h-2,4.2):
                arch(panel,xx,yy,zz-.035,1.2,2.5,color,lod)
        m.merge(turn(panel,angle,x,y,z))


def block(m,x,y,z,w,h,d,color,lod=2,windows=False,material=11):
    m.box((x,y+h/2,z),(w,h,d),color,material)
    m.box((x,y+h-.15,z),(w+.4,.3,d+.4),tuple(min(255,v+12) for v in color),17)
    if windows: aperture_sides(m,x,y,z,w,d,h,lod,color)


def wall(m,a,b,width,height,color,lod,merlons=True,batter=0,footings=None):
    """Variable-bottom wall strip; each endpoint carries its terrain elevation."""
    a,b=np.asarray(a,float),np.asarray(b,float)
    delta=b-a;length=np.linalg.norm(delta[[0,2]])
    if length<.01:return
    right=np.array([delta[2],0,-delta[0]])/length
    for side in [-1,1]:
        p=a+right*side*(width/2+batter);q=b+right*side*(width/2+batter)
        pp=a+right*side*width/2+[0,height,0];qq=b+right*side*width/2+[0,height,0]
        if footings is None:
            face(m,[p,q,qq,pp],color,side*right)
        else:
            steps=max(1,math.ceil(length/6))
            for j in range(steps):
                t0=j/steps;t1=(j+1)/steps
                lo=p*(1-t0)+q*t0;hi=p*(1-t1)+q*t1
                lo[1]=min(lo[1],support_grade(footings,lo[0],lo[2])-.5)
                hi[1]=min(hi[1],support_grade(footings,hi[0],hi[2])-.5)
                face(m,[lo,hi,pp*(1-t1)+qq*t1,pp*(1-t0)+qq*t0],color,side*right)
    face(m,[a-right*width/2+[0,height,0],b-right*width/2+[0,height,0],
            b+right*width/2+[0,height,0],a+right*width/2+[0,height,0]],
            tuple(min(255,v+12) for v in color))
    for p,normal in [(a,-delta),(b,delta)]:
        left=p-right*(width/2+batter);right_bottom=p+right*(width/2+batter)
        if footings is not None:
            left[1]=min(left[1],support_grade(footings,left[0],left[2])-.5)
            right_bottom[1]=min(right_bottom[1],support_grade(footings,right_bottom[0],right_bottom[2])-.5)
        face(m,[left,right_bottom,p+right*width/2+[0,height,0],p-right*width/2+[0,height,0]],color,normal)
    # Course joints are relief bands, retained at medium LOD on large masonry.
    if lod<2:
        for yy in np.arange(2.5,height-1,3):
            for side in [-1,1]:
                if footings is None:
                    off=right*side*(width/2+batter*(1-yy/height)+.03)
                    face(m,[a+off+[0,yy-.055,0],b+off+[0,yy-.055,0],
                            b+off+[0,yy+.055,0],a+off+[0,yy+.055,0]],
                         tuple(max(0,v-19) for v in color),side*right,17)
                else:
                    steps=max(1,math.ceil(length/6))
                    def course_point(t,dy):
                        base=a*(1-t)+b*t
                        lo=base+right*side*(width/2+batter)
                        hi=base+right*side*width/2+[0,height,0]
                        lo[1]=min(lo[1],support_grade(footings,lo[0],lo[2])-.5)
                        return lo+(hi-lo)*(base[1]+yy+dy-lo[1])/(hi[1]-lo[1])+right*side*.03
                    for j in range(steps):
                        face(m,[course_point(j/steps,-.055),course_point((j+1)/steps,-.055),
                                course_point((j+1)/steps,.055),course_point(j/steps,.055)],
                             tuple(max(0,v-19) for v in color),side*right,17)
    if merlons:
        count=max(1,int(length/2.5))
        for j in range(count):
            t=(j+.5)/count;p=a*(1-t)+b*t
            piece=Mesh();piece.box((0,.55,0),(length/count*.56,1.1,min(width,1.2)),color)
            m.merge(turn(piece,math.degrees(math.atan2(-delta[2],delta[0])),p[0],p[1]+height,p[2]))


def circuit(m,points,width,height,color,lod,closed=True,merlons=True,batter=0,footings=None):
    pairs=list(zip(points,points[1:]))
    if closed:pairs.append((points[-1],points[0]))
    for a,b in pairs:wall(m,a,b,width,height,color,lod,merlons,batter,footings)


def tower(m,x,y,z,r,h,color,lod,crenels=True,batter=1):
    n=(32,20,12)[lod]
    lathe(m,x,z,[(y,r+batter),(y+h-1.2,r),(y+h,r+.25)],color,n)
    if crenels:
        count=max(8,int(r*2.6))
        for j in range(count):
            a=j*math.tau/count
            q=Mesh();q.box((0,.5,0),(max(.5,r*math.tau/count*.5),1,1),color)
            m.merge(turn(q,-math.degrees(a),x+r*math.sin(a),y+h,z+r*math.cos(a)))
    if lod<2:
        for yy in np.arange(3,h-2,5):
            for j in range(max(8,n//2)):
                a=j*math.tau/max(8,n//2)
                q=Mesh();q.box((0,0,-r-batter*(1-yy/h)-.03),(.25,1.0,.06),DARK,15)
                m.merge(turn(q,math.degrees(a),x,y+yy,z))


def chhatri(m,x,y,z,r,h,color,lod,roof=None,material=11,roof_material=12):
    n=(24,16,10)[lod];roof=roof or color
    lathe(m,x,z,[(y,r*1.1),(y+.35,r*1.1)],color,n,material)
    for j in range(8):
        a=j*math.tau/8
        m.beam((x+r*.72*math.cos(a),y+.35,z+r*.72*math.sin(a)),
               (x+r*.72*math.cos(a),y+h,z+r*.72*math.sin(a)),.17,color,material,6)
    lathe(m,x,z,[(y+h,r*1.12),(y+h+.3,r*1.12)],roof,n,roof_material)
    dome(m,x,y+h+.3,z,r, r*.95,roof,lod,True,roof_material)
    lathe(m,x,z,[(y+h+r*1.2,.25),(y+h+r*1.5,.06)],OCHRE,8,SCULPTURE_METAL)


def hip(m,x,y,z,w,d,rise,color,lod,korean=False):
    # Two shallow eave stages give East Asian roofs their swept corner profile.
    rings=[]
    for t,hh in ([(0,0),(.16,-.18),(.6,rise*.45),(1,rise)] if korean else [(0,0),(1,rise)]):
        ww=w*(1-.7*t);dd=d*(1-t)
        rings.append([(x-ww/2,y+hh,z-dd/2),(x+ww/2,y+hh,z-dd/2),
                      (x+ww/2,y+hh,z+dd/2),(x-ww/2,y+hh,z+dd/2)])
    for a,b in zip(rings,rings[1:]):
        for j in range(4):face(m,[a[j],a[(j+1)%4],b[(j+1)%4],b[j]],color,(0,1,0),TILE_ROOF)
    if lod<2:
        for side in [-1,1]:
            for xx in np.arange(-w/2+.5,w/2,.9 if lod==0 else 1.8):
                m.beam((x+xx,y+.06,z+side*d/2),(x+xx*.30,y+rise+.08,z),.055,
                       tuple(min(255,v+16) for v in color),TILE_ROOF,4)
    m.beam((x-w*.16,y+rise+.12,z),(x+w*.16,y+rise+.12,z),.17,color,TILE_ROOF,6)


def deck(m,poly,y,color,base=None):
    # All deck polygons supplied here are convex. Concave courts use separate bays.
    face(m,[(x,y,z) for x,z in poly],color,material=8)
    if base is not None:
        c=np.mean(poly,axis=0)
        for a,b in zip(poly,poly[1:]+poly[:1]):
            normal=((a[0]+b[0])/2-c[0],0,(a[1]+b[1])/2-c[1])
            face(m,[(a[0],base,a[1]),(b[0],base,b[1]),(b[0],y,b[1]),(a[0],y,a[1])],STONE,normal)


def cannon(m,x,y,z,heading,lod):
    q=Mesh();q.box((0,.3,0),(.8,.6,1.6),(81,60,40),TIMBER_WALL)
    q.beam((0,.85,.5),(0,1.05,-1.9),.22,(49,56,53),16,(10,8,6)[lod])
    m.merge(turn(q,heading,x,y,z))


def garita(m,x,y,z,lod,color=LIME):
    lathe(m,x,z,[(y-1.8,.15),(y,1.25),(y+.3,1.2),(y+3.1,1.2)],color,(16,12,8)[lod])
    for heading in (0,90,180,270):
        q=Mesh();arch(q,0,y+1,-1.22,.42,1.2,color,lod);m.merge(turn(q,heading,x,0,z))
    dome(m,x,y+3.1,z,1.38,1.3,color,lod)
    lathe(m,x,z,[(y+4.3,.25),(y+4.9,.05)],color,8)


_PLANS = {}
_GROUNDS = {}


def plan(site):
    if site not in _PLANS:
        _PLANS[site]=json.loads((REF/(site+'-plan.json')).read_text())
    return _PLANS[site]


def way(site, ident):
    return next(w for w in plan(site)['ways'] if w['id']==ident)


def ring(site,ident):
    p=way(site,ident)['xz']
    return p[:-1] if p[0]==p[-1] else p


def grade(site,x,z):
    if site not in _GROUNDS:
        j=json.loads((REF/(site+'-terrain.json')).read_text())
        p=np.array([[q['east_m'],q['north_m'],q['ground_m']] for q in j['ground_samples']])
        _GROUNDS[site]=(p,float(j['ground_m']))
    p,base=_GROUNDS[site]
    dist=np.sum((p[:,:2]-[x,z])**2,axis=1)
    near=np.argsort(dist)[:4]
    if dist[near[0]]<.01:return float(p[near[0],2]-base)
    weights=1/np.maximum(dist[near],.01)
    return float(np.sum(p[near,2]*weights)/weights.sum()-base)


def datum(site):
    grade(site,0,0)
    return _GROUNDS[site][1]


_SUPPORT_REQUESTS={}
_SUPPORT_SAMPLES={}


def support_grade(site,x,z):
    """Measured toe ground is independent of the retained architectural floor."""
    key=(round(float(x),5),round(float(z),5))
    _SUPPORT_REQUESTS.setdefault(site,set()).add(key)
    if site not in _SUPPORT_SAMPLES:
        path=REF/(site+'-footing-ground.json')
        data=json.loads(path.read_text()) if path.exists() else {'ground_samples':[]}
        _SUPPORT_SAMPLES[site]={(round(p['east_m'],5),round(p['north_m'],5)):p['ground_m'] for p in data['ground_samples']}
    absolute=_SUPPORT_SAMPLES[site].get(key)
    return grade(site,x,z) if absolute is None else float(absolute-datum(site))


def footing_prism(m,site,poly,top,color):
    """Continuous support skin under a mapped flat floor, with buried toes."""
    p=np.asarray(poly,float)
    if np.linalg.norm(p[0]-p[-1])<1e-5:p=p[:-1]
    area=sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(p,np.roll(p,-1,axis=0)))
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        d=b-a;steps=max(1,math.ceil(np.linalg.norm(d)/6))
        for j in range(steps):
            aa=a+d*j/steps;bb=a+d*(j+1)/steps
            ay=min(top-.3,support_grade(site,*aa)-.5);by=min(top-.3,support_grade(site,*bb)-.5)
            face(m,[(aa[0],ay,aa[1]),(bb[0],by,bb[1]),(bb[0],top,bb[1]),(aa[0],top,aa[1])],color,(d[1]*np.sign(area),0,-d[0]*np.sign(area)))


def box_footing(m,site,c,width,depth,heading,top,color):
    angle=math.radians(heading);axis=np.array([math.cos(angle),-math.sin(angle)])
    normal=np.array([-axis[1],axis[0]])
    polygon=[np.asarray(c)+axis*u+normal*v for u,v in [(-width/2,-depth/2),(width/2,-depth/2),(width/2,depth/2),(-width/2,depth/2)]]
    footing_prism(m,site,polygon,top,color)


def round_footing(m,site,x,z,r,top,color,lod):
    polygon=[(x+r*math.cos(a),z+r*math.sin(a)) for a in np.linspace(0,math.tau,(32,20,12)[lod],endpoint=False)]
    footing_prism(m,site,polygon,top,color)


def measured_wall(m,site,ident,width,height,color,lod,merlons=True,batter=0):
    p=way(site,ident)['xz']
    points=[]
    for a,b in zip(p,p[1:]):
        a,b=np.array(a),np.array(b)
        count=max(1,int(np.linalg.norm(b-a)/15))
        for k in range(count):
            x,z=a+(b-a)*k/count
            points.append((x,grade(site,x,z)-1.5,z))
    x,z=p[-1];points.append((x,grade(site,x,z)-1.5,z))
    circuit(m,points,width,height+1.5,color,lod,False,merlons,batter,footings=site if site not in ['qaitbay','krak','hwaseong'] else None)


def _triangulate(poly):
    """Ear clipping retains concave measured wall/deck plans without a dependency."""
    p=np.asarray(poly,float)
    if np.linalg.norm(p[0]-p[-1])<1e-6:p=p[:-1]
    area=sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(p,np.roll(p,-1,axis=0)))
    if area<0:p=p[::-1]
    indices=list(range(len(p)));out=[]
    def cross(a,b):return a[0]*b[1]-a[1]*b[0]
    while len(indices)>3:
        found=False
        for j,i in enumerate(indices):
            i0,i1=indices[j-1],indices[(j+1)%len(indices)]
            a,b,c=p[i0],p[i],p[i1]
            if cross(b-a,c-b)<1e-7:continue
            def inside(q):return min(cross(b-a,q-a),cross(c-b,q-b),cross(a-c,q-c))>=-1e-7
            if any(inside(p[k]) for k in indices if k not in (i0,i,i1)):continue
            out.append((a,b,c));indices.pop(j);found=True;break
        if not found:
            # Collinear points are removed only when they add no occupied area.
            candidates=[(abs(cross(p[indices[j]]-p[indices[j-1]],p[indices[(j+1)%len(indices)]]-p[indices[j]])),j) for j in range(len(indices))]
            v,j=min(candidates)
            if v>1e-4:raise ValueError('non-simple authored polygon')
            indices.pop(j)
    if len(indices)==3:out.append(tuple(p[k] for k in indices))
    return out


def prism(m,poly,base,top,color,roof=None,batter=0):
    p=np.asarray(poly,float)
    if np.linalg.norm(p[0]-p[-1])<1e-6:p=p[:-1]
    c=p.mean(axis=0)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        orientation=1 if sum(u[0]*v[1]-v[0]*u[1] for u,v in zip(p,np.roll(p,-1,axis=0)))>0 else -1
        normal=np.array([b[1]-a[1],0,a[0]-b[0]])*orientation
        face(m,[(a[0],base,a[1]),(b[0],base,b[1]),(b[0],top,b[1]),(a[0],top,a[1])],color,normal)
    for tri in _triangulate(p):face(m,[(x,top,z) for x,z in tri],roof or color,material=12 if roof else 8)


def oriented(site,ident):
    p=np.asarray(ring(site,ident))
    # Longest footprint edge fixes the local building frame; no guessed compass turn.
    d=np.roll(p,-1,axis=0)-p;axis=d[np.argmax(np.sum(d*d,axis=1))];axis/=np.linalg.norm(axis)
    a=math.degrees(math.atan2(-axis[1],axis[0]));angle=math.radians(a)
    local=p@np.array([[math.cos(angle),math.sin(angle)],[-math.sin(angle),math.cos(angle)]])
    lo,hi=local.min(axis=0),local.max(axis=0);mid=(lo+hi)/2
    c=np.array([mid[0]*math.cos(angle)+mid[1]*math.sin(angle),-mid[0]*math.sin(angle)+mid[1]*math.cos(angle)])
    return c,float(hi[0]-lo[0]),float(hi[1]-lo[1]),a


def mapped_block(m,site,ident,height,color,lod,windows=True,roof=False):
    c,w,d,a=oriented(site,ident);y=grade(site,*c)
    q=Mesh();block(q,0,0,0,w,height,d,color,lod,windows)
    if roof:hip(q,0,height,0,w+1,d+1,min(6,d*.25),(124,76,53),lod)
    placed=turn(q,a,c[0],y,c[1]);m.merge(placed)
    if site not in ['qaitbay','hwaseong','krak']:
        box_footing(m,site,c,w,d,a,y,color)
    return c,w,d,a,y


def _derawar(lod):
    m=Mesh();site='derawar';color=(166,104,68)
    # Forty bastions occupy a roughly 200m square, as measured from OSM170220501.
    # Preserve their mapped scalloped perimeter instead of extrapolating a square
    # from the contradictory published 1.5km total wall-length figure.
    p=ring(site,170220501)
    y=float(np.median([grade(site,*q) for q in p]));height=30
    circuit(m,[(x,y-2,z) for x,z in p],3.8,height+2,color,lod,True,False,1.3,footings=site)
    # Brick courses and lozenge bands follow the actual curved bastion faces.
    if lod<2:
        center=np.mean(p,axis=0)
        for a,b in zip(p,p[1:]+p[:1]):
            a,b=np.array(a),np.array(b);v=b-a;length=np.linalg.norm(v)
            if length<.3:continue
            normal=np.array([v[1],-v[0]])/length
            if np.dot(normal,(a+b)/2-center)<0:normal=-normal
            for yy in [5,9.0,12.7,17,21.5,25.8,28.7]:
                aa=a+normal*(2.02+1.3*(1-yy/32));bb=b+normal*(2.02+1.3*(1-yy/32))
                m.beam((aa[0],y+yy,aa[1]),(bb[0],y+yy,bb[1]),.10,(196,137,91),17,4)
            if lod==0:
                mid=(a+b)/2+normal*2.15
                for yy in [10.8,19.2,24]:
                    # Alternating recessed brickwork diamonds are geometry, not a texture.
                    dx,dz=v/length*.60
                    face(m,[(mid[0]-dx,y+yy,mid[1]-dz),(mid[0],y+yy+.8,mid[1]),
                            (mid[0]+dx,y+yy,mid[1]+dz),(mid[0],y+yy-.8,mid[1])],
                         (120,73,46),(normal[0],0,normal[1]),17)
    for ident,h in [(404546455,7),(404546456,8),(404546458,6),(404546460,8),(404546461,5),(404546463,5),(404546464,4),(404546466,4),(1529612395,7)]:
        c,w,d,a,_=mapped_block(m,site,ident,h,(177,130,87),lod,True)
        if ident in (404546455,1529612395):
            for xx in [-w*.36,w*.36]:
                q=Mesh();chhatri(q,xx,h,0,1.3,2.2,(186,147,106),lod);m.merge(turn(q,a,c[0],y,c[1]))
    # Northeast entrance projects from the wall, with a genuinely open upper court.
    for ident in [863160753,863160754,404546468]:
        measured_wall(m,site,ident,1.8,5,color,lod)
    return m


def _qaitbay_floor():
    return json.loads((REF/'qaitbay-courtyard.json').read_text())['court_absolute_m']-datum('qaitbay')


def _qaitbay_apron():
    a,b=np.array([-110.,-101.]),np.array([-63.7,-78.3])
    side=np.array([-(b-a)[1],(b-a)[0]])/np.linalg.norm(b-a)*2.1
    return [a-side,b-side,b+side,a+side]


def _qaitbay(lod):
    m=Mesh();site='qaitbay';color=(210,195,160)
    floor=_qaitbay_floor();roof=floor+7.4
    outline=ring(site,28826432)
    bottom=min(grade(site,*p) for p in outline)-2
    # The coastal DSM contains below-sea-level court samples and tower returns.
    # A supported paved court ties the keep to the independently calibrated approach.
    prism(m,outline,bottom,floor,color)
    circuit(m,[(x,bottom,z) for x,z in outline],2.0,roof-bottom,color,lod,True,True,.7)
    for garden in json.loads((REF/'qaitbay-courtyard.json').read_text())['garden_ways']:
        for tri in _triangulate(garden['xz']):
            face(m,[(x,floor+.025,z)for x,z in tri],(106,124,69),material=3)
    apron=_qaitbay_apron();start=json.loads((REF/'qaitbay-courtyard.json').read_text())['approach_start_absolute_m']-datum(site)
    for tri in [(0,1,2),(0,2,3)]:
        face(m,[(apron[i][0],start if i in (0,3) else floor,apron[i][1])for i in tri],color,material=8)
    for i,j in [(0,1),(1,2),(2,3),(3,0)]:
        a,b=apron[i],apron[j];ya=start if i in (0,3)else floor;yb=start if j in (0,3)else floor
        normal=(a[0]+b[0]+173.7,0,a[1]+b[1]+179.3)
        face(m,[(a[0],bottom,a[1]),(b[0],bottom,b[1]),(b[0],yb,b[1]),(a[0],ya,a[1])],color,normal)
    # Barracks follow the measured inner court outline, not a guessed square.
    for ident in [602123353,1231905928,1231905929]:
        p=ring(site,ident)
        circuit(m,[(x,floor-.4,z) for x,z in p],3.4,7.8,color,lod,True,False)
        if ident==602123353:
            orientation=1 if sum(u[0]*v[1]-v[0]*u[1]for u,v in zip(p,p[1:]+p[:1]))>0 else -1
            for aa,bb in zip(p,p[1:]+p[:1]):
                aa,bb=np.array(aa),np.array(bb);v=bb-aa;length=np.linalg.norm(v)
                if length<10:continue
                normal=np.array([-v[1],v[0]])/length*orientation
                front=Mesh()
                for xx in np.arange(-length/2+2.2,length/2-1.8,4.2):
                    for yy in [.25,4.1]:arch(front,xx,yy,-.04,1.6,2.6,color,lod)
                center=(aa+bb)/2+normal*1.73
                m.merge(turn(front,math.degrees(math.atan2(-normal[0],-normal[1])),center[0],floor,center[1]))
    outer=ring(site,602123350)
    circuit(m,[(x,floor-.4,z) for x,z in outer],1.2,7.8,color,lod,True,False)
    roof_ranges(m,outer,[ring(site,i) for i in [602123353,1231905928,1231905929]],roof,color)
    c,w,d,a=oriented(site,602123355);p=ring(site,602123355)
    y=floor;q=Mesh();keep=Mesh()
    # The measured scalloped keep is ~37m outside the corner towers and ~30m core.
    block(q,0,0,0,29.2,20.8,29.2,color,lod,True)
    for xx in [-14.3,14.3]:
        for zz in [-14.3,14.3]:
            tower(q,xx,0,zz,2.65,20.0,color,lod,False,.4)
            lathe(q,xx,zz,[(18.6,2.7),(19.9,3.6),(21.2,3.6)],LIME,(24,16,12)[lod])
            for k in range(12):
                t=k*math.tau/12
                q.box((xx+3.2*math.cos(t),21.8,zz+3.2*math.sin(t)),(.65,1.2,.65),color)
    for angle in [0,90,180,270]:
        front=Mesh();arch(front,0,.8,-14.71,4.4,8.2,LIME,lod,True)
        for yy in [8,13.8]:
            for xx in [-9.5,-4.7,4.7,9.5]:arch(front,xx,yy,-14.73,1.1,2.5,LIME,lod)
        for xx in np.arange(-13,14,2):front.box((xx,21.45,-14.4),(.9,1.3,.9),color)
        for xx in [-5.7,5.7]:
            block(front,xx,12.4,-15.2,2.4,6,2.8,color)
            lathe(front,xx,-15.4,[(11.5,.35),(13,1.55)],color,8)
        keep.merge(turn(front,angle))
    keep.merge(q)
    # Published17m is the body above its floor; corner crowns project above it.
    keep=keep.transform([[1,0,0,0],[0,17/20.8,0,0],[0,0,1,0],[0,0,0,1]])
    m.merge(turn(keep,a,c[0],y,c[1]))
    steps=Mesh()
    for j in range(4):
        h=(j+1)*(.8*17/20.8)/4
        steps.box((0,h/2,-16.5+j*.45),(4.6,h,1.8-j*.35),color,8)
    m.merge(turn(steps,a+90,c[0],floor,c[1]))
    for ident in [1232623655,1232623656,1233994796]:
        cc,w,d,aa=oriented(site,ident);q=Mesh();block(q,0,0,0,w,4.2,d,color,lod,True)
        foundation=min(floor-.5,min(grade(site,*p) for p in ring(site,ident))-1)
        q.box((0,(foundation-floor)/2,0),(w,floor-foundation,d),color)
        m.merge(turn(q,aa,cc[0],floor,cc[1]))
    return m


def _krak(lod):
    m=Mesh();site='krak';color=(180,168,143)
    # Retain the measured outer-wall crowns while extending each actual
    # battered toe into its surveyed slope, rather than using centreline grade.
    outline=way(site,226877188)['xz'];outer=[]
    for a,b in zip(outline,outline[1:]):
        a,b=np.asarray(a),np.asarray(b)
        count=max(1,int(np.linalg.norm(b-a)/15))
        for k in range(count):
            x,z=a+(b-a)*k/count;outer.append((x,grade(site,x,z)-1.5,z))
    x,z=outline[-1];outer.append((x,grade(site,x,z)-1.5,z))
    circuit(m,outer,4.8,12.3,color,lod,closed=False,merlons=True,batter=2.1,footings=site)
    p=ring(site,375152260);inner=ring(site,375151598)
    y=float(np.median([grade(site,*q) for q in inner]))-3
    # Inner galleries preserve the mapped court as a real void. The great talus
    # descends to the lower ward, rather than flattening the summit underneath it.
    circuit(m,[(x,y-4,z) for x,z in p],6.0,19,color,lod,closed=True,merlons=True,batter=4.8,footings=site)
    circuit(m,[(x,y+5,z) for x,z in inner],3.0,8.5,LIME,lod,closed=True,merlons=False,footings=site)
    for ident,h in [(375156451,16),(375156452,14),(375156453,20),(375156454,13),
                    (375156455,15),(375156456,22),(375156457,24),(375156458,18)]:
        poly=ring(site,ident)
        footing_prism(m,site,poly,y-2,color)
        prism(m,poly,y-2,y+h,color)
        circuit(m,[(x,y+h,z) for x,z in poly],.9,.6,color,lod,True,True)
    gallery=ring(site,375151599)
    circuit(m,[(x,y+4,z) for x,z in gallery],1.8,9.5,color,lod,closed=True,merlons=False,footings=site)
    roof_ranges(m,gallery,[inner],y+13.5,color)
    # Gothic loggia on the western side of the inner court.
    walkway=[(23.5,26.4),(26.5,26.4),(26.5,66.0),(23.5,66.0)]
    footing_prism(m,site,walkway,y+5-.08,LIME)
    prism(m,walkway,y+5-.18,y+5,LIME)
    for z in np.arange(28,65,4.4):
        q=Mesh();arch(q,z-46,0,-.1,3.1,5.8,LIME,lod,True);m.merge(turn(q,90,25,y+5,46))
    # Southern towers emerge from the talus; their different diameters survive LOD.
    for x,z,r,h in [(12,-30,7.8,24),(31,-34,8.2,27),(64,-20,8.7,26)]:
        foot=[(x+(r+5)*math.cos(t),z+(r+5)*math.sin(t)) for t in np.arange((32,20,12)[lod])*math.tau/(32,20,12)[lod]]
        footing_prism(m,site,foot,y-6,color)
        tower(m,x,y-6,z,r,h,color,lod,True,5)
    return m


def arcade(m,w,d,height,color,lod,bays=9):
    """Open columned audience hall, with roof and pierced front/side aisles."""
    m.box((0,.35,0),(w+1,.7,d+1),color,8)
    for x in np.linspace(-w/2+1,w/2-1,bays+1):
        for z in np.linspace(-d/2+1,d/2-1,3):
            m.box((x,height/2,z),(.75,height,.75),color)
            m.box((x,height-.8,z),(1.2,.5,1.2),LIME,17)
    m.box((0,height,0),(w+2,.55,d+2),color,12)
    for x in np.linspace(-w/2+1,w/2-1,bays+1):
        for side in [-1,1]:
            # Curved spandrel relief spans each bay but leaves the opening empty.
            if x>=w/2-1:continue
            span=(w-2)/bays
            for t in np.linspace(0,1,(9,6,4)[lod]):
                yy=height-.4-math.sin(t*math.pi)*.9
                m.box((x+span*t,yy,side*(d/2-.9)),(span/(9,6,4)[lod]*1.1,.24,.45),color)


def mughal_gate(m,x,y,z,w,h,d,lod,color=SAND,heading=0,footings=None):
    q=Mesh();opening=w*.27
    for xx in [-(w+opening)/4,(w+opening)/4]:block(q,xx,0,0,(w-opening)/2,h,d,color,lod,True)
    block(q,0,h*.66,0,opening,h*.34,d,color)
    for zz in [-d/2-.05,d/2+.05]:
        p=Mesh();arch(p,0,0,-abs(zz),opening,h*.68,MARBLE,lod,True)
        q.merge(turn(p,0 if zz<0 else 180))
    for xx in [-w*.36,w*.36]:chhatri(q,xx,h,0,2.2,3,MARBLE,lod,material=VEINED_MARBLE,roof_material=VEINED_MARBLE)
    if lod<2:
        for xx in np.arange(-w/2+1,w/2,2.2):
            q.box((xx,h-2.5,-d/2-.1),(1.4,1.1,.2),TEAL,TILE_ROOF)
            q.box((xx,h-2.5,-d/2-.24),(.4,.6,.12),OCHRE,TILE_ROOF)
    m.merge(turn(q,heading,x,y,z))
    if footings is not None:
        box_footing(m,footings,(x,z),w,d,heading,y,color)
        floor=Mesh();floor.box((0,-.08,0),(w,.16,d),color,8);m.merge(turn(floor,heading,x,y,z))


def roof_ranges(m,outer,holes,height,color):
    """Even/odd scanline tessellation preserves measured inner courtyard voids."""
    rings=[np.asarray(r,float) for r in [outer,*holes]]
    breaks=sorted(set(float(v[1]) for r in rings for v in r))
    edges=[(a,b) for r in rings for a,b in zip(r,np.roll(r,-1,axis=0)) if abs(a[1]-b[1])>1e-8]
    for z0,z1 in zip(breaks,breaks[1:]):
        if z1-z0<1e-6:continue
        mid=(z0+z1)/2
        active=[(a,b) for a,b in edges if min(a[1],b[1])<mid<max(a[1],b[1])]
        def intersection(edge,z):
            a,b=edge;return a[0]+(b[0]-a[0])*(z-a[1])/(b[1]-a[1])
        active.sort(key=lambda e:intersection(e,mid))
        if len(active)%2:raise ValueError('invalid courtyard scanline')
        for left,right in zip(active[::2],active[1::2]):
            a,b=intersection(left,z0),intersection(right,z0)
            c,d=intersection(right,z1),intersection(left,z1)
            y=height((a+b+c+d)/4,mid) if callable(height) else height
            face(m,[(a,y,z0),(b,y,z0),(c,y,z1),(d,y,z1)],color,material=12)


@lru_cache(None)
def _amber_terraces():
    return {ident:float(np.median([grade('amber',*q) for q in ring('amber',ident)]))
            for ident in [192093289,192093288,192093290,192093291]}


def _amber_roof_height(x,z):
    terraces=_amber_terraces()
    ident=min(terraces,key=lambda i:np.linalg.norm(np.mean(ring('amber',i),axis=0)-[x,z]))
    return terraces[ident]+7.5


def _amber(lod):
    m=Mesh();site='amber';color=(205,164,110)
    outer=ring(site,192093292)
    # The four measured court polygons have independent terraces; connect their
    # retaining walls on the surveyed slope instead of one floating plateau.
    measured_wall(m,site,192093292,5.6,12,color,lod,True,1.2)
    court_levels=_amber_terraces()
    for ident,y in court_levels.items():
        p=ring(site,ident)
        for tri in _triangulate(p):deck(m,[tuple(q) for q in tri],y+.2,(198,178,143))
        circuit(m,[(x,y,z) for x,z in p],6.4,7.5,color,lod,True,False,footings=site)
        footing_prism(m,site,p,y+.19,color)
        for a,b in zip(p,p[1:]+p[:1]):
            a,b=np.array(a),np.array(b);v=b-a;length=np.linalg.norm(v)
            if length<8:continue
            q=Mesh()
            for x in np.arange(-length/2+2,length/2-1,4.6):arch(q,x,1,-3.3,2.7,4.7,LIME,lod,True)
            mid=(a+b)/2
            m.merge(turn(q,math.degrees(math.atan2(-v[1],v[0])),mid[0],y,mid[1]))
    courts=[ring(site,i) for i in court_levels]
    roof_y=_amber_roof_height
    roof_ranges(m,outer,courts,roof_y,(207,176,132))
    # Small rooftop service chambers follow the rear range, leaving the courts open.
    for x,z,w,d in [(-97,-36,15,12),(-65,13,16,10),(-33,65,14,10),(0,132,13,9)]:
        yy=roof_y(x,z);block(m,x,yy,z,w,3.8,d,color,lod,True)
        for xx in [-w*.35,w*.35]:chhatri(m,x+xx,yy+3.8,z,1.2,1.5,LIME,lod)
    # Diwan-e-Aam is an open colonnaded hall at the upper side of the public court.
    c,w,d,a=oriented(site,192093288);y=court_levels[192093288]
    q=Mesh();arcade(q,43,13,7.5,(221,193,149),lod,8)
    m.merge(turn(q,a,c[0]-10,y+1,c[1]+14))
    box_footing(m,site,(c[0]-10,c[1]+14),44,14,a,y+1,(221,193,149))
    # Ganesh Pol has colored relief panels, marble arch trim and painted floral bays.
    mughal_gate(m,-3,court_levels[192093290]+1,45,22,14,5,lod,color,heading=34,footings=site)
    c,w,d,a=oriented(site,192093290);y=court_levels[192093290]
    q=Mesh()
    for xx in [-12,12]:
        for zz in [-12,12]:q.box((xx,.3,zz),(18,.5,17),(71,106,62),8)
    for xx in [-22,0,22]:q.box((xx,.62,0),(.7,.15,43),(210,194,160),8)
    for zz in [-22,0,22]:q.box((0,.62,zz),(43,.15,.7),(210,194,160),8)
    # Reflective mirror-hall panels remain small facets within stone bay surrounds.
    for side in [-1,1]:
        for xx in np.arange(-17,18,4.1):
            arch(q,xx,1,side*25,2.5,5.2,MARBLE,lod,True)
            if lod==0:
                for yy in [2,3,4,5]:
                    q.box((xx,yy,side*25-.2),(.32,.34,.16),(180,192,184),17)
    m.merge(turn(q,a,c[0],y+.25,c[1]))
    c,w,d,a=oriented(site,192093291);y=court_levels[192093291]
    pavilion=Mesh();arcade(pavilion,10,10,4.6,LIME,lod,3)
    for xx in [-4,4]:
        for zz in [-4,4]:chhatri(pavilion,xx,4.9,zz,1.4,1.8,LIME,lod)
    m.merge(turn(pavilion,a,c[0],y,c[1]))
    for x,z,r,h in [(-130,-96,7,18),(128,107,5,13),(54,52,3.5,16),(-84,30,3,15)]:
        yy=grade(site,x,z);tower(m,x,yy-2,z,r,h,color,lod,False,1.4);chhatri(m,x,yy+h-2,z,r*.72,3,LIME,lod)
        round_footing(m,site,x,z,r+1.4,yy-2,color,lod)
    # Foreground lake retaining walls remain distinct from the palace terraces.
    for ident in [191861480,191861483,192119232,192119235,192280059]:
        measured_wall(m,site,ident,2.4,5.6,(176,145,100),lod,True,.8)
    for ident in [191861468,191861469,191861470,192116836,370166571]:mapped_block(m,site,ident,7.5,color,lod,True)
    return m


def _gwalior(lod):
    m=Mesh();site='gwalior';stone=(174,145,102)
    for w in plan(site)['ways']:
        if w['tags'].get('barrier')=='city_wall':measured_wall(m,site,w['id'],3.6,10.7,stone,lod,True,1.5)
    # The mapped F-shaped range includes the gate extension; keep its two open
    # courtyard notches. NMA documents a 91.5m decorated facade and six towers.
    poly=ring(site,1176311658);y=float(np.median([grade(site,*v) for v in poly]))
    prism(m,poly,y-6,y+14,stone)
    footing_prism(m,site,poly,y-6,stone)
    # Way1176311659 duplicates three faces of the palace footprint.
    for ident in [1176311660]:
        p=way(site,ident)['xz'];circuit(m,[(x,y-2,z) for x,z in p],3.2,16,stone,lod,False,True,footings=site)
    start=np.array([153.88,41.159]);end=np.array([179.89,185.144])
    axis=(end-start)/np.linalg.norm(end-start);out=np.array([axis[1],-axis[0]])
    for distance in np.linspace(4,95.5,6):
        x,z=start+axis*distance
        tower(m,x,y-6,z,3.6,22,stone,lod,False,.5)
        round_footing(m,site,x,z,4.1,y-6,stone,lod)
        for yy in [1.5,7,12.7]:lathe(m,x,z,[(y+yy,3.72),(y+yy+.72,3.72)],TEAL,(24,16,10)[lod],TILE_ROOF)
        chhatri(m,x,y+16,z,3.2,3.2,SAND,lod,roof=TEAL,roof_material=TILE_ROOF)
    facade=Mesh();length=146.3
    for yy in [2,7.5,12]:
        facade.box((length/2,yy,-.15),(length,.72,.15),TEAL,TILE_ROOF)
        facade.box((length/2,yy+.52,-.27),(length,.18,.45),(204,171,116),17)
        for xx in np.arange(3,length-2,3.5):
            # Enamel frieze units: blue ground, yellow bird body, neck and beak.
            face(facade,[(xx-.60,yy-.14,-.28),(xx+.20,yy-.14,-.28),
                         (xx+.50,yy+.10,-.28),(xx+.46,yy+.28,-.28),
                         (xx+.20,yy+.33,-.28),(xx+.02,yy+.06,-.28),
                         (xx-.48,yy+.16,-.28)],OCHRE,(0,0,-1),17)
    for xx in np.arange(6,length-3,7.1):
        for yy in [3.0,8.5]:
            arch(facade,xx,yy,-.28,2.2,2.6,LIME,lod,True)
            if lod<2:
                for off in [-.65,0,.65]:facade.box((xx+off,yy+1,-.48),(.09,1.7,.16),SAND,17)
                for h in [.5,1,1.5]:facade.box((xx,yy+h,-.48),(1.7,.08,.17),SAND,17)
            facade.box((xx,yy+2.75,-.48),(3.4,.25,1.3),LIME,17)
            for off in [-1.1,1.1]:facade.box((xx+off,yy+2.4,-.48),(.3,.6,.75),SAND,17)
    angle=math.degrees(math.atan2(-axis[1],axis[0]))
    m.merge(turn(facade,angle,start[0],y,start[1]))
    # Narrow rear terraces and internal carved bay screens bound both courts.
    for z0,x0,ww in [(95,116,31),(151,126,25)]:
        wall(m,(x0,y-6,z0),(x0+ww,y-6,z0),1.4,20,stone,lod,False,footings=site)
        for xx in np.arange(x0,x0+ww,4.4):
            arch(m,xx,y+3,z0,2.6,4.5,LIME,lod,True)
            arch(m,xx,y+9,z0,2.2,3.3,LIME,lod,True)
    # Jahangir/Karan palace is an irregular connected range with a large open court.
    for ident in [1176920965,1500163693]:
        p=ring(site,ident);yy=float(np.median([grade(site,*v) for v in p]))
        circuit(m,[(x,yy,z) for x,z in p],4.5,13,stone,lod,True,True,footings=site)
    for ident,h in [(1176311654,8),(1176311655,7),(1176311657,10),(1176920966,8),
                    (1231803603,12),(1500161573,8),(1500161574,8)]:mapped_block(m,site,ident,h,stone,lod,True)
    c,w,d,a=oriented(site,1500163692);chhatri(m,c[0],grade(site,*c),c[1],3.6,5,LIME,lod)
    return m


def _kumbhalgarh(lod):
    m=Mesh();site='kumbhalgarh';stone=(165,149,114)
    # The OSM circuit is 7.2km, while tourism's 36km covers a larger historical
    # attribution. Render only evidenced surviving alignments; don't invent walls.
    for w in plan(site)['ways']:
        t=w['tags'];p=np.asarray(w['xz'])
        if t.get('barrier')=='city_wall' or (t.get('barrier')=='wall' and np.linalg.norm(p.mean(axis=0))<300):
            measured_wall(m,site,w['id'],5.0 if w['id']==254041054 else 2.4,
                          9.0 if w['id']==254041054 else 6.5,stone,lod,True,2.5 if w['id']==254041054 else .7)
    # Badal Mahal sits above Kumbha Mahal. Two measured courtyard holes remain
    # open through the roof; shared summit datum prevents overlapping roof slabs.
    outer=ring(site,210591763);courts=[ring(site,i) for i in [210591766,785895226]]
    y=18.65;roof_y=y+13
    circuit(m,[(x,y-2,z) for x,z in outer],2.8,15,(215,205,173),lod,True,False)
    for court in courts:
        circuit(m,[(x,y+1,z) for x,z in court],2.2,12,(224,216,185),lod,True,False)
        for tri in _triangulate(court):deck(m,[tuple(v) for v in tri],y+1,(191,182,148))
    roof_ranges(m,outer,courts,roof_y,(220,211,179))
    # Recessed upper windows and jaali screens follow each measured exterior face.
    exterior=list(outer)
    if sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(exterior,exterior[1:]+exterior[:1]))<0:exterior.reverse()
    for a,b in zip(exterior,exterior[1:]+exterior[:1]):
        a,b=np.array(a),np.array(b);v=b-a;length=np.linalg.norm(v)
        if length<5:continue
        q=Mesh()
        for xx in np.arange(-length/2+1.8,length/2-1,3.5):
            arch(q,xx,5,-1.45,1.2,2.1,LIME,lod,True)
            arch(q,xx,9,-1.45,1.2,2.1,LIME,lod,True)
            if lod<2:
                for off in [-.3,.3]:q.box((xx+off,9.9,-1.65),(.08,1.6,.12),(174,192,175),17)
        mid=(a+b)/2;m.merge(turn(q,math.degrees(math.atan2(-v[1],v[0])),mid[0],y,mid[1]))
    # The older palace below is a stepped irregular masonry range, not its AABB.
    for ident,top in [(785895228,18),(785895229,23)]:
        p=ring(site,ident);bottom=min(grade(site,*q) for q in p)-1
        prism(m,p,bottom,top,(187,172,133))
        circuit(m,[(x,top,z) for x,z in p],.65,.8,(200,185,146),lod,True,True)
    for x,z in [(0,45),(34,45),(5,90),(40,82)]:chhatri(m,x,roof_y,z,2.2,3.1,(224,215,185),lod)
    # The entry sequence follows the measured zigzag below the summit.
    for ident in [391934992,391934993,391934994,785981053]:mapped_block(m,site,ident,6.5,stone,lod,True)
    for x,z,heading in [(91,-27,10),(75,-63,32),(161,-128,90)]:mughal_gate(m,x,grade(site,x,z),z,12,11,6,lod,stone,heading,footings=site)
    for ident in [252292354,252292377,252292383,1171303572,1171303576,1171303583]:
        c,w,d,a=oriented(site,ident);y=grade(site,*c);q=Mesh()
        block(q,0,0,0,w,5,d,stone,lod,True)
        # Distinct lower pillared mandapa and tapering sanctuary shikhara.
        for h,r in [(5,min(w,d)*.34),(7,min(w,d)*.27),(9,min(w,d)*.22),(11,min(w,d)*.16)]:
            lathe(q,0,d*.2,[(h,r),(h+2,r*.8)],LIME,(16,12,8)[lod])
        lathe(q,0,d*.2,[(13,1.1),(14,.6),(15,.1)],LIME,12)
        m.merge(turn(q,a,c[0],y,c[1]))
        box_footing(m,site,c,w,d,a,y,stone)
    return m


def _red_fort(lod):
    m=Mesh();site='red-fort';color=(160,78,54)
    # Use mapped city-wall centre lines; a historic-site boundary includes lawns
    # and is not the architectural wall alignment.
    walls=[w for w in plan(site)['ways'] if w['tags'].get('barrier')=='city_wall' and w['id'] not in {973077710,973086044}]
    for w in walls:measured_wall(m,site,w['id'],5.5,17.5,color,lod,True,1.5)
    for ident in [320351009,320351010,320351011,320353005,320353009]:
        p=ring(site,ident);yy=float(np.median([grade(site,*q) for q in p]))
        circuit(m,[(x,yy-2,z) for x,z in p],2.0,14.5,color,lod,True,True,.6,footings=site)
    for ident in [80429907,120396374]:
        c,w,d,a=oriented(site,ident);yy=grade(site,*c)
        mughal_gate(m,c[0],yy-2,c[1],max(w,d)*.82,27,min(w,d),lod,color,a,footings=site)
    # West-to-east processional sequence, dimensioned from source building polygons.
    for ident,h in [(80429800,10),(80429900,16)]:mapped_block(m,site,ident,h,color,lod,True)
    for ident,h,bays,col in [(80429794,12,9,(181,118,84)),(80429830,9,5,MARBLE),
                            (80429887,8,5,MARBLE),(80429915,9,5,MARBLE),(80429815,9,6,MARBLE)]:
        c,w,d,a=oriented(site,ident);yy=grade(site,*c);q=Mesh()
        arcade(q,w,d,h,col,lod,bays)
        if ident!=80429794:
            for xx in [-w*.42,w*.42]:
                for zz in [-d*.38,d*.38]:chhatri(q,xx,h+.3,zz,1.8,2.2,MARBLE,lod)
        if ident!=80429794:retag(q,{11:VEINED_MARBLE,12:VEINED_MARBLE,17:VEINED_MARBLE})
        m.merge(turn(q,a,c[0],yy,c[1]))
        footing=Mesh();box_footing(footing,site,c,w+1,d+1,a,yy,col)
        if ident!=80429794:retag(footing,{11:VEINED_MARBLE})
        m.merge(footing)
    for ident,h in [(223903620,7),(320353006,5),(320353007,5),(320353008,6),(698892682,11)]:
        q=Mesh();mapped_block(q,site,ident,h,MARBLE if ident!=320353008 else color,lod,True)
        if ident!=320353008:retag(q,{11:VEINED_MARBLE,12:VEINED_MARBLE,17:VEINED_MARBLE})
        m.merge(q)
    c,w,d,a=oriented(site,223903623);yy=grade(site,*c);q=Mesh()
    circuit(q,[(-w/2,0,-d/2),(w/2,0,-d/2),(w/2,0,d/2),(-w/2,0,d/2)],1.6,6,MARBLE,lod,True,False)
    arcade(q,w*.90,d*.35,6.2,MARBLE,lod,3)
    for xx in [-w*.28,0,w*.28]:dome(q,xx,6.5,d*.20,w*.12,4.7,MARBLE,lod,True)
    retag(q,{11:VEINED_MARBLE,12:VEINED_MARBLE,17:VEINED_MARBLE})
    m.merge(turn(q,a,c[0],yy,c[1]))
    footing=Mesh();box_footing(footing,site,c,w+1.6,d+1.6,a,yy,MARBLE);retag(footing,{11:VEINED_MARBLE});m.merge(footing)
    # Later red barracks are extant architectural fabric and stay in their sites.
    for ident in [80429788,80429844,80429889,80429891]:mapped_block(m,site,ident,13,(168,94,67),lod,True,True)
    return m


def _fasil(lod):
    m=Mesh();site='fasil';color=(154,137,103)
    measured_wall(m,site,1536242568,1.5,4.8,color,lod,True,.4)
    c,w,d,a=oriented(site,267159704);y=grade(site,*c);q=Mesh()
    # Fasilidas' main keep: high rectangular body and four attached domed towers.
    w,d=min(w,33),min(d,29)
    block(q,0,0,0,w,18.5,d,color,lod,True)
    for xx in [-w*.44,w*.44]:
        for zz in [-d*.43,d*.43]:
            tower(q,xx,0,zz,3.8,23,color,lod,False,.5)
            dome(q,xx,23,zz,4.1,3.8,(180,166,128),lod)
            lathe(q,xx,zz,[(26.8,.2),(27.8,.05)],LIME,8,17)
    for heading,ww,dd in [(0,w,d),(90,d,w),(180,w,d),(270,d,w)]:
        panel=Mesh()
        for xx in np.arange(-ww*.4,ww*.41,2.3):panel.box((xx,19.3,-dd/2),(.95,1.6,.85),color)
        for yy in [6,12]:
            for xx in [-ww*.28,0,ww*.28]:
                arch(panel,xx,yy,-dd/2-.04,1.6,3.7,LIME,lod,False)
                panel.box((xx,yy-.25,-dd/2-.5),(2.2,.3,1.1),LIME,17)
        m.merge(turn(panel,a+heading,c[0],y,c[1]))
    box_footing(m,site,c,w,d,a,y,color)
    for xx in [-w*.44,w*.44]:
        for zz in [-d*.43,d*.43]:
            angle=math.radians(a);x=c[0]+xx*math.cos(angle)+zz*math.sin(angle);z=c[1]-xx*math.sin(angle)+zz*math.cos(angle)
            round_footing(m,site,x,z,4.3,y,color,lod)
    # Roof stair lookout gives the keep its asymmetric upper silhouette.
    block(q,-w*.25,18.5,d*.21,7.4,6.0,8.5,color,lod,True)
    m.merge(turn(q,a,c[0],y,c[1]))
    for ident,h in [(267159675,13),(182025517,11),(267159693,10),(182025103,8),
                    (267159678,10),(267159703,12),(267159677,10),(342847641,5)]:
        poly=ring(site,ident);c,w,d,a=oriented(site,ident);yy=grade(site,*c)
        if ident in [267159675,182025103]:
            # Ruined halls retain an open sky instead of a speculative new roof.
            circuit(m,[(x,yy,z) for x,z in poly],1.8,h,color,lod,True,True,footings=site)
        else:
            mapped_block(m,site,ident,h,color,lod,True,ident in [267159678,342847641])
        if ident in [267159675,267159703]:
            for xx in [-w/2,w/2]:
                t=Mesh();tower(t,xx,0,-d/2,2.1,h+2,color,lod,False,.2)
                if ident==267159703:dome(t,xx,h+2,-d/2,2.3,2.4,LIME,lod)
                m.merge(turn(t,a,c[0],yy,c[1]))
                angle=math.radians(a);x=c[0]+xx*math.cos(angle)-d/2*math.sin(angle);z=c[1]-xx*math.sin(angle)-d/2*math.cos(angle)
                round_footing(m,site,x,z,2.3,yy,color,lod)
    for ident in [267159673,267159674,267159676,267159706]:
        poly=ring(site,ident);yy=grade(site,*np.mean(poly,axis=0))
        circuit(m,[(x,yy,z) for x,z in poly],1.0,3.0,color,lod,True,False,footings=site)
    return m


def korean_pavilion(m,w,d,base,lod,stories=1,open_sides=True):
    red=(143,55,39);green=(52,104,87);blue=(59,99,124)
    for k in range(stories):
        y=base+k*4.6;ww=w*(1-.18*k);dd=d*(1-.16*k)
        if not open_sides:
            block(m,0,y,0,ww*.90,3.2,dd*.80,(140,68,45),material=TIMBER_WALL)
            for heading,span,depth in [(0,ww*.90,dd*.80),(180,ww*.90,dd*.80),(90,dd*.80,ww*.90),(270,dd*.80,ww*.90)]:
                lattice=Mesh()
                for xx in np.arange(-span/2+1,span/2-.6,2.1):
                    lattice.box((xx,y+1.7,-depth/2-.20),(1.5,1.8,.32),(40,67,53),TIMBER_WALL)
                    for off in np.linspace(-.7,.7,5 if lod<2 else 3):
                        lattice.box((xx+off,y+1.7,-depth/2-.42),(.08,1.8,.14),green,TIMBER_WALL)
                    for yy in [1,1.45,1.9,2.35]:
                        lattice.box((xx,y+yy,-depth/2-.43),(1.5,.08,.15),green,TIMBER_WALL)
                m.merge(turn(lattice,heading))
        for xx in np.linspace(-ww*.43,ww*.43,max(3,int(ww/4)+1)):
            for zz in [-dd*.38,dd*.38]:
                m.beam((xx,y,zz),(xx,y+3.4,zz),.20,red,TIMBER_WALL,8)
                m.box((xx,y+3.1,zz),(1.0,.35,.85),green,TIMBER_WALL)
                if lod<2:
                    for off in [-.36,.36]:m.box((xx+off,y+3.5,zz),(.22,.6,1.2),blue,TIMBER_WALL)
        for zz in [-dd*.4,dd*.4]:
            m.box((0,y+2.9,zz),(ww,.2,.25),green,TIMBER_WALL)
            m.box((0,y+3.25,zz),(ww,.16,.3),(185,136,72),TIMBER_WALL)
        hip(m,0,y+3.8,0,ww+2.2,dd+2.2,3.1,TILE,lod,True)
        if k==0 and stories==2:m.box((0,y+4.3,0),(w*.76,.4,d*.74),green,TIMBER_WALL)


def _hwaseong(lod):
    m=Mesh();site='hwaseong';stone=(159,158,144)
    for w in plan(site)['ways']:
        if w['tags'].get('barrier')=='city_wall':
            measured_wall(m,site,w['id'],3.4,5.6,stone,lod,True,.85)
    main={477614267:2,1024560873:2,1024546130:1,1026075984:1}
    excluded={766314741,766318354,1026075986,1154890589,1154890590,1154890591,1201063336,1446708925}
    for w in plan(site)['ways']:
        ident=w['id'];tags=w['tags']
        if not tags.get('building') or ident in excluded:continue
        c,ww,dd,a=oriented(site,ident);y=grade(site,*c);q=Mesh()
        if ident in main:
            w0=max(ww,dd);d0=min(ww,dd);opening=5.2 if main[ident]==2 else 3.6
            for xx in [-(w0+opening)/4,(w0+opening)/4]:block(q,xx,-1,0,(w0-opening)/2,7,d0,stone)
            block(q,0,5.0,0,opening,1.0,d0,stone)
            # The semicircular stone vault is an actual opening through the gate.
            radius=opening/2;spring=4.8-radius
            curve=[(radius*math.cos(t),spring+radius*math.sin(t)) for t in np.linspace(0,math.pi,(20,14,8)[lod]+1)]
            for (x0,y0),(x1,y1) in zip(curve,curve[1:]):
                for zz,normal in [(-d0/2,(0,0,-1)),(d0/2,(0,0,1))]:
                    face(q,[(x0,y0,zz),(x1,y1,zz),(x1,5,zz),(x0,5,zz)],stone,normal)
                face(q,[(x0,y0,-d0/2),(x0,y0,d0/2),(x1,y1,d0/2),(x1,y1,-d0/2)],stone,(-(x0+x1)/2,spring-(y0+y1)/2,0))
            korean_pavilion(q,w0*.92,d0*.92,6,lod,main[ident],False)
            m.merge(turn(q,a,c[0],y,c[1]))
        elif ident==1024546127:
            # Seven open water arches keep the Suwoncheon crossing permeable.
            for j in range(8):q.box(((j-3.5)*3.2,1.2,0),(.7,3.6,4.7),stone)
            q.box((0,3.35,0),(25.2,1.2,4.7),stone)
            korean_pavilion(q,14.4,7.0,4,lod,1,True)
            m.merge(turn(q,a,c[0],y-3,c[1]))
        elif ident in [1026075987,1026076007]:
            if ident==1026076007:
                lathe(q,0,0,[(0,6.1),(8.2,5.8)],(141,116,91),(28,20,12)[lod])
                for h in [2,4,6]:
                    for j in range(12):
                        b=Mesh();b.box((0,h,-6),(.3,.6,.10),DARK,15);q.merge(turn(b,j*30))
                korean_pavilion(q,7.5,6.3,8.2,lod,1,False)
            else:
                block(q,0,0,0,ww,9,dd,(133,125,108),lod,True)
                korean_pavilion(q,ww*.9,dd*.9,9,lod,1,False)
            m.merge(turn(q,a,c[0],y,c[1]))
        elif ident==1137849746:
            block(q,0,0,0,ww,3.5,dd,(139,103,78))
            for xx in np.linspace(-ww*.37,ww*.37,5):
                lathe(q,xx,0,[(3.5,1.2),(6.5,1.0),(7.0,1.25),(7.6,.8)],BRICK,(16,12,8)[lod],BRICK_WALL)
            m.merge(turn(q,a,c[0],y,c[1]))
        elif ident==1184619460:
            lathe(q,0,0,[(0,4.2),(3,3.4)],stone,8)
            m.merge(turn(q,a,c[0],y,c[1]))
        else:
            artillery='Artillery' in tags.get('name:en','')
            if artillery:block(q,0,-1,0,ww,6,dd,stone,lod,True)
            korean_pavilion(q,ww,dd,5 if artillery else .5,lod,1,not artillery)
            m.merge(turn(q,a,c[0],y,c[1]))
    # Original brick half-moon gate shields use their mapped open outlines.
    for ident in [1026075986,1154890589,1154890590,1154890591]:
        q=Mesh();measured_wall(q,site,ident,.8,4.5,(144,116,92),lod,True,.4)
        m.merge(retag(q,{11:BRICK_WALL,17:BRICK_WALL}))
    return m


def battery(m,site,poly,altitude,lod,cannon_count=0):
    top=altitude-datum(site);c=np.mean(poly,axis=0)
    for a,b in zip(poly,poly[1:]+poly[:1]):
        a,b=np.array(a),np.array(b);v=b-a;length=np.linalg.norm(v)
        if length<.2:continue
        normal=np.array([v[1],-v[0]])/length
        if np.dot(normal,(a+b)/2-c)<0:normal=-normal
        ay=min(grade(site,*a)-2,top-2);by=min(grade(site,*b)-2,top-2)
        aa=a+normal*2.8;bb=b+normal*2.8
        ay=min(ay,support_grade(site,*aa)-.5);by=min(by,support_grade(site,*bb)-.5)
        steps=max(1,math.ceil(length/6))
        for j in range(steps):
            t0=j/steps;t1=(j+1)/steps;lo=aa*(1-t0)+bb*t0;hi=aa*(1-t1)+bb*t1
            ly=min(ay*(1-t0)+by*t0,support_grade(site,*lo)-.5);hy=min(ay*(1-t1)+by*t1,support_grade(site,*hi)-.5)
            u=a*(1-t0)+b*t0;v=a*(1-t1)+b*t1
            face(m,[(lo[0],ly,lo[1]),(hi[0],hy,hi[1]),(v[0],top,v[1]),(u[0],top,u[1])],
                 (181,163,128),(normal[0],0,normal[1]))
        wall(m,(a[0],top,a[1]),(b[0],top,b[1]),2.0,1.4,(187,173,139),lod,False)
    for tri in _triangulate(poly):deck(m,[tuple(p) for p in tri],top,(198,184,153))
    if cannon_count:
        # Guns are placed along the longest firing face and point outward.
        pairs=list(zip(poly,poly[1:]+poly[:1]));a,b=max(pairs,key=lambda ab:np.linalg.norm(np.array(ab[1])-ab[0]))
        a,b=np.array(a),np.array(b);v=b-a;normal=np.array([v[1],-v[0]])/np.linalg.norm(v)
        if np.dot(normal,(a+b)/2-c)<0:normal=-normal
        for t in np.linspace(.08,.92,cannon_count):
            p=a+(b-a)*t-normal*3.0;cannon(m,p[0],top,p[1],math.degrees(math.atan2(-normal[0],-normal[1])),lod)


def _el_morro(lod):
    m=Mesh();site='el-morro'
    def toe_height(x,z):
        ground=support_grade(site,x,z)
        return min(ground-.5,-datum(site)-.5) if ground+datum(site)<=1.001 else ground-.5
    def offset_ring(poly,distance):
        p=np.asarray(poly,float);area=sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))
        edges=np.roll(p,-1,axis=0)-p
        normals=np.column_stack([edges[:,1],-edges[:,0]])/np.linalg.norm(edges,axis=1)[:,None]*np.sign(area)
        out=[]
        for i,v in enumerate(p):
            n=normals[i-1]+normals[i];denom=1+np.dot(normals[i-1],normals[i])
            shift=n*distance/max(.1,denom)
            if np.linalg.norm(shift)>abs(distance)*4:shift*=abs(distance)*4/np.linalg.norm(shift)
            out.append(v+shift)
        return np.asarray(out),normals
    def closed_battery(poly,altitude,guns=0):
        p=np.asarray(poly,float);top=altitude-datum(site);toe,normals=offset_ring(poly,2.8)
        crown,_=offset_ring(poly,1.0)
        for i in range(len(p)):
            j=(i+1)%len(p);length=np.linalg.norm(p[j]-p[i]);count=max(1,math.ceil(length/2))
            for k in range(count):
                t,u=k/count,(k+1)/count
                a=toe[i]*(1-t)+toe[j]*t;b=toe[i]*(1-u)+toe[j]*u
                c=crown[i]*(1-t)+crown[j]*t;d=crown[i]*(1-u)+crown[j]*u
                ay=min(top-2,toe_height(*a));by=min(top-2,toe_height(*b))
                face(m,[(a[0],ay,a[1]),(b[0],by,b[1]),(d[0],top,d[1]),(c[0],top,c[1])],(181,163,128),(normals[i,0],0,normals[i,1]))
        outer,_=offset_ring(poly,1.0);inner,_=offset_ring(poly,-1.0)
        for i in range(len(p)):
            j=(i+1)%len(p);normal=(normals[i,0],0,normals[i,1])
            for ring_,sign in [(outer,1),(inner,-1)]:
                aa,bb=ring_[i],ring_[j]
                face(m,[(aa[0],top-.04,aa[1]),(bb[0],top-.04,bb[1]),(bb[0],top+1.4,bb[1]),(aa[0],top+1.4,aa[1])],(187,173,139),tuple(sign*v for v in normal))
            face(m,[(q[0],top+1.4,q[1])for q in [outer[i],outer[j],inner[j],inner[i]]],(199,185,151))
        for tri in _triangulate(poly):deck(m,[tuple(q)for q in tri],top,(198,184,153))
        if guns:
            i=max(range(len(p)),key=lambda i:np.linalg.norm(p[(i+1)%len(p)]-p[i]));j=(i+1)%len(p)
            for t in np.linspace(.08,.92,guns):
                q=p[i]*(1-t)+p[j]*t-normals[i]*3
                cannon(m,q[0],top,q[1],math.degrees(math.atan2(-normals[i,0],-normals[i,1])),lod)
    # Six terraces use the NPS 16ft/140ft lower/upper survey constraint; geography
    # comes from the measured promontory. A sea-level datum avoids summit flattening.
    terraces=[
      ([(-130,132),(-121,154),(-108,147),(-106,116),(-116,110)],5.0,3),
      ([(-117,108),(-104,83),(-108,60),(-89,53),(-77,118),(-107,145)],11.0,5),
      ([(-105,42),(-92,24),(-82,0),(-43,17),(-20,43),(-48,71),(-89,75)],17.5,6),
      ([(-103,93),(-78,67),(-49,74),(-31,113),(-59,123),(-107,151)],24.0,8),
      ([(-80,-26),(-61,-21),(-36,-35),(-23,4),(-43,21),(-68,9)],31.0,4),
      ([(-27,116),(-34,129),(-5,121),(19,120),(35,120),(28,109),(28,89),(53,66),(17,48),(-4,65),(-21,46),(-40,64)],39.0,5),
    ]
    for poly,h,guns in terraces:closed_battery(poly,h,guns)
    # Landward court is a lower open rectangle behind the high hornwork.
    court=[(-60,30),(-20,42),(-17,94),(-44,109),(-67,79)]
    closed_battery(court,27.0)
    q=Mesh();block(q,0,0,0,9,9.2,8,(201,171,137),lod,True)
    lathe(q,0,0,[(9.2,2.1),(12.5,2.1),(13,2.7)],LIME,8)
    for j in range(8):
        a=j*math.tau/8
        q.beam((2.2*math.cos(a),12.6,2.2*math.sin(a)),(2.2*math.cos(a),15,2.2*math.sin(a)),.10,(62,70,66),17,4)
    dome(q,0,15,0,2.7,1.6,(78,90,87),lod)
    m.merge(turn(q,15,10,39-datum(site),96))
    for x,z,h in [(-118,153,5),(-106,57,11),(-78,-28,31),(35,121,39),(53,66,39),(-35,129,39)]:
        poly=next(poly for poly,alt,_ in terraces if alt==h)
        points=np.asarray(poly,float);k=int(np.argmin(np.linalg.norm(points-[x,z],axis=1)))
        edge,_=offset_ring(poly,.45);q=edge[k]
        garita(m,float(q[0]),h-datum(site),float(q[1]),lod)
    # Switchback ramps are sloped stone decks with continuous retaining edges.
    for a,b,w in [((-53,27-datum(site),39),(-79,17.5-datum(site),63),5),
                  ((-25,39-datum(site),99),(-42,27-datum(site),60),5),
                  ((-83,17.5-datum(site),57),(-111,11-datum(site),85),4)]:
        a,b=np.asarray(a),np.asarray(b);direction=b-a;length=np.linalg.norm(direction[[0,2]])
        normal=np.array([direction[2],0,-direction[0]])/length;count=max(1,math.ceil(length/2))
        for j in range(count):
            aa=a+(b-a)*j/count;bb=a+(b-a)*(j+1)/count
            corners=[aa-normal*w/2,bb-normal*w/2,bb+normal*w/2,aa+normal*w/2]
            crown=[q+[0,.35,0]for q in corners]
            bottom=[np.array([q[0],min(q[1]-.5,toe_height(q[0],q[2])),q[2]])for q in corners]
            face(m,crown,(203,188,155))
            for k,l,n in [(0,1,-normal),(2,3,normal),(3,0,-direction),(1,2,direction)]:
                if (k==3 and j!=0)or(k==1 and j!=count-1):continue
                face(m,[bottom[k],bottom[l],crown[l],crown[k]],(181,163,128),n)
    return m


def _barajas(lod):
    m=Mesh();site='barajas'
    # Separate batteries wrap the original castillejo; no rectangular tower replaces
    # the interconnected sloping scarps of the San Lázaro hill.
    batteries=[
      ('San Carlos',[(15,109),(54,151),(93,165),(132,159),(133,119),(113,126),(93,59),(40,69)],13.0,13),
      ('Hornabeque',[(0,75),(18,107),(48,93),(57,40),(28,29),(-3,42)],22.0,6),
      ('La Cruz',[(1,39),(29,30),(44,-5),(15,-32),(-2,-18)],25.0,8),
      ('Redencion',[(28,30),(58,48),(85,40),(75,5),(87,-17),(55,-33),(40,-8)],28.0,11),
      ('Santa Barbara',[(91,0),(112,-5),(143,-20),(163,-16),(175,6),(174,19),(157,16),(141,-9)],24.0,7),
      ('San Lazaro',[(34,-90),(24,-105),(38,-116),(36,-136),(56,-144),(79,-135),(95,-107),(105,-105),(116,-82),(80,-68)],20.0,5),
      ('Castillejo',[(46,-65),(58,-36),(81,-31),(110,-44),(107,-68),(80,-82)],38.2,6),
    ]
    for _,poly,h,count in batteries:battery(m,site,poly,h,lod,count)
    for x,z,h in [(19,103,22),(132,158,13),(176,16,24),(58,-143,20),(107,-68,38.2)]:garita(m,x,h-datum(site),z,lod,(184,166,128))
    # Original summit chapel/guardhouse and real sloping approach ramps.
    block(m,69,38.2-datum(site),-57,12,5.2,7,(195,179,141),lod,True)
    hip(m,69,43.4-datum(site),-57,13,8,2.6,(134,95,66),lod)
    for a,b in [((-25,3,-46),(13,22,-20)),((13,22,-20),(48,28,-3)),((48,28,-3),(63,38.2,-37))]:
        aa=(a[0],a[1]-datum(site),a[2]);bb=(b[0],b[1]-datum(site),b[2]);wall(m,aa,bb,4,.3,LIME,lod,False,footings=site)
    for x,z,head,h in [(15,-29,0,22),(81,-81,180,20),(50,68,0,13)]:
        q=Mesh();arch(q,0,0,-.1,2.3,3.7,LIME,lod);m.merge(turn(q,head,x,h-datum(site),z))
    return m


def _citadelle(lod):
    m=Mesh();site='citadelle';color=(150,145,122)
    outer=ring(site,220820148)
    # The footprint folds around an open court and culminates in the northeast
    # pointed battery. Continuous upper datum; foundations key into the hill.
    top=29.0
    for a,b in zip(outer,outer[1:]+outer[:1]):
        a,b=np.asarray(a),np.asarray(b);v=b-a;length=np.linalg.norm(v)
        if length<.4:continue
        normal=np.array([v[1],-v[0]])/length
        ay=grade(site,*a)-4;by=grade(site,*b)-4
        face(m,[(a[0],ay,a[1]),(b[0],by,b[1]),(b[0],top,b[1]),(a[0],top,a[1])],color,(normal[0],0,normal[1]))
        wall(m,(a[0],top,a[1]),(b[0],top,b[1]),2,1.5,(170,161,133),lod,False)
        # Three tiers of recessed casemate cannon ports are individually modeled.
        q=Mesh()
        for xx in np.arange(-length/2+2.8,length/2-2,5.2):
            for yy in [8,15,22]:
                arch(q,xx,yy,-.03,1.1,1.6,(174,163,135),lod)
                if lod==0:q.box((xx,yy-.15,-.15),(1.5,.20,.22),(117,115,97),17)
        mid=(a+b)/2;m.merge(turn(q,math.degrees(math.atan2(-v[1],v[0])),mid[0],0,mid[1]))
    for tri in _triangulate(outer):deck(m,[tuple(p) for p in tri],top,(175,161,133))
    for ident,h in [(220820145,19),(220820146,13),(220820147,15)]:
        c,w,d,a=oriented(site,ident);q=Mesh()
        block(q,0,0,0,w,h,d,(174,157,125),lod,True)
        hip(q,0,h,0,w+1,d+1,min(6,w*.22),(141,87,64),lod)
        m.merge(turn(q,a,c[0],3,c[1]))
        box_footing(m,site,c,w,d,a,3,(174,157,125))
    for ident in [183741402,183741406]:
        p=ring(site,ident);yy=grade(site,*np.mean(p,axis=0))
        circuit(m,[(x,yy,z) for x,z in p],1.7,8.0,color,lod,True,False,footings=site)
    # Stored shot and short guns identify the broad terrepleins at close range.
    for x,z in [(-22,76),(-15,80),(54,101),(63,112)]:
        for level in range(3):
            count=4-level
            for i in range(count):
                for j in range(count):
                    m.ellipsoid((x+(i-(count-1)/2)*.52,top+.27+level*.42,z+(j-(count-1)/2)*.52),(.27,.27,.27),(63,64,57),16,(10,8,6)[lod],(6,4,3)[lod])
    for x,z,h in [(64,111,45),(72,116,45),(-27,84,270),(-22,73,270)]:cannon(m,x,top,z,h,lod)
    return m


BUILDERS = {
    'hwaseong-fortress': ('hwaseong',_hwaseong),
    'amber-fort': ('amber',_amber),
    'gwalior-fort': ('gwalior',_gwalior),
    'kumbhalgarh-fort': ('kumbhalgarh',_kumbhalgarh),
    'red-fort': ('red-fort',_red_fort),
    'derawar-fort': ('derawar',_derawar),
    'krak-des-chevaliers': ('krak',_krak),
    'qaitbay-citadel': ('qaitbay',_qaitbay),
    'fasil-ghebbi': ('fasil',_fasil),
    'castillo-san-felipe-del-morro': ('el-morro',_el_morro),
    'castillo-san-felipe-de-barajas': ('barajas',_barajas),
    'citadelle-laferriere': ('citadelle',_citadelle),
}

_OWNED = {
 'derawar':[170220501,404546455,404546456,404546458,404546460,404546461,404546463,404546464,404546466,1529612395,863160753,863160754,404546468],
 'qaitbay':[602123350,28826432,602123353,1231905928,1231905929,602123355,1232623655,1232623656,1233994796],
 'krak':[375151599,226877188,375152260,375151598,375156451,375156452,375156453,375156454,375156455,375156456,375156457,375156458],
 'amber':[192093292,192093289,192093288,192093290,192093291,191861480,191861483,192119232,192119235,192280059,191861468,191861469,191861470,192116836,370166571],
 'gwalior':[1176311658,1176311659,1176311660,1176920965,1500163693,1176311654,1176311655,1176311657,1176920966,1231803603,1500161573,1500161574,1500163692],
 'kumbhalgarh':[210591763,210591766,785895226,785895228,785895229,391934992,391934993,391934994,785981053,252292354,252292377,252292383,1171303572,1171303576,1171303583],
 'red-fort':[320351009,320351010,320351011,320353005,320353009,80429907,120396374,80429800,80429900,80429794,80429830,80429887,80429915,80429815,223903620,320353006,320353007,320353008,698892682,223903623,80429788,80429844,80429889,80429891],
 'fasil':[1536242568,267159704,267159675,182025517,267159693,182025103,267159678,267159703,267159677,342847641,267159673,267159674,267159676,267159706],
 'el-morro':[561634537],
 'barajas':[49551791],
 'citadelle':[220820148,220820145,220820146,220820147,183741402,183741406],
}


def _owned(site):
    ids=set(_OWNED.get(site,[]))
    for w in plan(site)['ways']:
        t=w['tags'];c=np.mean(w['xz'],axis=0)
        if site=='hwaseong':
            excluded={766314741,766318354,1201063336,1446708925}
            if w['id'] not in excluded and (t.get('barrier')=='city_wall' or t.get('building')):ids.add(w['id'])
        if site=='gwalior' and t.get('barrier')=='city_wall':ids.add(w['id'])
        if site=='kumbhalgarh' and (t.get('barrier')=='city_wall' or (t.get('barrier')=='wall' and np.linalg.norm(c)<300)):ids.add(w['id'])
        if site=='red-fort' and t.get('barrier')=='city_wall' and w['id'] not in {973077710,973086044}:ids.add(w['id'])
    return [w for w in plan(site)['ways'] if w['id'] in ids]


def _exclusions(site):
    result=[]
    if site=='qaitbay':
        for ident in [28826432,1232623655,1232623656,1233994796]:
            result.extend(np.asarray(tri).tolist()for tri in _triangulate(ring(site,ident)))
        return result+[np.asarray(_qaitbay_apron()).tolist()]
    solid={'derawar':{170220501},'qaitbay':{602123355},'el-morro':{561634537},'barajas':{49551791},'citadelle':{220820148,220820145,220820146,220820147}}
    for w in _owned(site):
        p=np.asarray(w['xz']);tags=w['tags']
        # Empty courts and inhabited districts inside fort circuits remain intact.
        if w['id'] in solid.get(site,set()) or (tags.get('building') and not tags.get('relation_id') and len(p)<100 and tags.get('building')!='wall'):
            for tri in _triangulate(p):result.append([list(map(float,v)) for v in tri])
        else:
            half=6 if site in ['kumbhalgarh','krak','amber'] else 4
            for a,b in zip(p,p[1:]):
                v=b-a;length=np.linalg.norm(v)
                if length<.1:continue
                side=np.array([v[1],-v[0]])/length*half
                result.append([list(map(float,v)) for v in [a-side,b-side,b+side,a+side]])
    roofs={
      'qaitbay':(602123350,[602123353,1231905928,1231905929]),
      'krak':(375151599,[375151598]),
      'amber':(192093292,[192093289,192093288,192093290,192093291]),
      'kumbhalgarh':(210591763,[210591766,785895226]),
    }
    if site in roofs:
        outer,holes=roofs[site];mesh=Mesh()
        roof_ranges(mesh,ring(site,outer),[ring(site,i) for i in holes],0.0,STONE)
        result.extend([[list(map(float,v[[0,2]])) for v in triangle] for triangle,_,_ in mesh.triangles])
    if site=='gwalior':
        result.extend([[[x,z-.7],[x+w,z-.7],[x+w,z+.7],[x,z+.7]] for z,x,w in [(95,116,31),(151,126,25)]])
    if site in ['amber','kumbhalgarh']:
        for ident in (_amber_terraces() if site=='amber' else [210591766,785895226]):
            result.extend(np.asarray(tri).tolist()for tri in _triangulate(ring(site,ident)))
    return result


def _terrain_cuts(site):
    cuts=[]
    if site=='qaitbay':
        for ident in [28826432,1232623655,1232623656,1233994796]:
            cuts.extend({'polygon_xz':np.asarray(tri).tolist(),'plane':[0,0,_qaitbay_floor()-.12],'feather_m':.5}
                        for tri in _triangulate(ring(site,ident)))
        p=_qaitbay_apron();start=json.loads((REF/'qaitbay-courtyard.json').read_text())['approach_start_absolute_m']-datum(site)
        for indices in [(0,1,2),(0,2,3)]:
            triangle=np.asarray([p[i]for i in indices])
            heights=[(start if i in (0,3)else _qaitbay_floor())-.12 for i in indices]
            plane=np.linalg.solve(np.column_stack([triangle,np.ones(3)]),heights)
            cuts.append({'polygon_xz':triangle.tolist(),'plane':plane.tolist(),'feather_m':.5})
        return cuts
    if site=='kumbhalgarh':
        for ident,y in [(210591766,19.5),(785895226,19.5),(785895228,17.8),(785895229,22.8)]:
            cuts.extend({'polygon_xz':np.asarray(tri).tolist(),'plane':[0,0,y],'feather_m':.5}
                        for tri in _triangulate(ring(site,ident)))
        roof=Mesh();roof_ranges(roof,ring(site,210591763),[ring(site,i) for i in [210591766,785895226]],31.65,STONE)
        cuts.extend({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,31.45],'feather_m':.5} for p,_,_ in roof.triangles)
        return cuts
    if site!='amber':return []
    for ident,y in _amber_terraces().items():
        cuts.extend({'polygon_xz':np.asarray(tri).tolist(),'plane':[0,0,y+.05],'feather_m':.5}
                    for tri in _triangulate(ring(site,ident)))
    roof=Mesh()
    roof_ranges(roof,ring(site,192093292),[ring(site,i)for i in _amber_terraces()],_amber_roof_height,STONE)
    cuts.extend({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,float(p[0,1])-.2],'feather_m':.5}
                for p,_,_ in roof.triangles)
    return cuts


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('LOD must be 0, 1, or 2')
    mesh=BUILDERS[name][1](lod)
    wall_material = {
        'derawar-fort':BRICK_WALL,
        'qaitbay-citadel':WEATHERED_LIMESTONE,
        'krak-des-chevaliers':WEATHERED_LIMESTONE,
        'castillo-san-felipe-del-morro':WEATHERED_LIMESTONE,
        'castillo-san-felipe-de-barajas':WEATHERED_LIMESTONE,
    }.get(name,STONE_WALL)
    return retag(mesh,{11:wall_material,12:wall_material,16:SCULPTURE_METAL})


_RESEARCH=json.loads((REF/'research.json').read_text())['sites']
_CITIES=['Suwon','Amer','Gwalior','Kumbhalgarh','Delhi','Cholistan','Al-Husn','Alexandria','Gondar','San Juan','Cartagena','Milot']
ENTRIES=[]
for (name,(site,_)),city in zip(BUILDERS.items(),_CITIES):
    anchor=plan(site)['anchor'];exclusions=_exclusions(site)
    ownership=np.concatenate(exclusions)
    ENTRIES.append({
      'name':name,'city':city,'lat':anchor[0],'lon':anchor[1],'heading':0.0,
      'base':datum(site),'suppress_buildings':True,'suppress_roads':site=='qaitbay','suppress_trees':True,
      'source_exclusion_polygons':exclusions,'terrain_flatten':[],'terrain_cuts':_terrain_cuts(site),
      'activation_bounds_xz':[ownership.min(axis=0).tolist(),ownership.max(axis=0).tolist()],
      'references':_RESEARCH[name]['references']+['https://www.openstreetmap.org/copyright'],
      'description':_RESEARCH[name]['layout'],
      'attribution':'Plans: © OpenStreetMap contributors, ODbL 1.0. Architectural geometry originally authored for Vearth.',
      'capture':{'distance':650 if site in ['gwalior','kumbhalgarh','hwaseong','red-fort'] else 350,
                 'azimuth':135,'elevation':30,'target_height':20,'time_utc':'2026-03-20T10:00:00Z'},
      'audit':{'plan_file':str((REF/(site+'-plan.json')).relative_to(REF.parents[3])),
               'terrain_file':str((REF/(site+'-terrain.json')).relative_to(REF.parents[3])),
               'dimensional_evidence':_RESEARCH[name].get('published',{}),
               'grounding':'Fixed production-provider anchor datum; geometry follows sampled local ground. No whole-site flattening.',
               'dimension_uncertainty':'Mapped horizontal plans are measured; unreferenced architectural heights, ornament spacing and small details are visual interpretations, not a conservation survey.',
               'orientation':'Geometry is authored directly in east/north coordinates, so runtime heading is zero.'}
    })
    if site=='qaitbay':
        ENTRIES[-1]['road_exclusion_polygons']=exclusions
        ENTRIES[-1]['audit']['grounding']='Shared court and keep floor at estimated1.329m absolute from open approach (~1m) plus paving. Deep continuous stone supports fill negative coastal DSM lows; bounded lower-only cuts remove interior DSM peaks. See qaitbay-courtyard.json.'
