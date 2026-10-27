"""Metric castle architecture; source measurements live in references/castles.

Coordinates are x east/right, y up, z north/back before the entry bearing.
Roofs, courtyards and individual ranges are authored per site. Repeated small
masonry members share primitives; no castle is generated from a generic plan.
"""
import math
import json
from functools import lru_cache
from pathlib import Path
import numpy as np
from city_catalog.common import Mesh
from heritage_catalog import materials as MAT

DEPENDENCIES = [Path(MAT.__file__)]

STONE = (174, 164, 145)
PALE = (222, 218, 201)
SLATE = (69, 79, 88)
TILE = (150, 73, 48)
DARK = (44, 49, 48)
BRICK = (164, 88, 65)


class CastleMesh(Mesh):
    """Assign construction-atlas materials to explicitly authored components."""
    def __init__(self,wall_material=82,roof_material=86):
        super().__init__()
        self.wall_material=wall_material
        self.roof_material=roof_material

    def triangle(self,a,b,c,color,material=11):
        material=self.wall_material if material==11 else self.roof_material if material==12 else material
        super().triangle(a,b,c,color,material)

    def merge(self,other):
        self.triangles.extend((p,c,self.wall_material if mat==11 else self.roof_material if mat==12 else mat) for p,c,mat in other.triangles)

REFERENCES = Path(__file__).parent/'references'/'castles'
FRAMES = {'edinburgh':((10,-36),0), 'alhambra':((0,0),0),
          'malbork':((-37,-38),35), 'predjama':((-49,-4),0)}


@lru_cache(None)
def plan_data(source):
    return json.loads((REFERENCES/f'{source}-plan.json').read_text())


def plan_ring(source, way_id):
    way=next(w for w in plan_data(source)['ways'] if w['id']==way_id)
    points=np.asarray(way['xz'],dtype=float)
    if np.array_equal(points[0],points[-1]):points=points[:-1]
    offset,heading=FRAMES.get(source,((0,0),0))
    c,s=math.cos(math.radians(heading)),math.sin(math.radians(heading))
    return (points-np.asarray(offset))@np.array([[c,s],[-s,c]])


def rectangle_fit(points):
    """Minimum-area enclosing rectangle; source shape remains in the audit."""
    best=None
    for a,b in zip(points,np.roll(points,-1,axis=0)):
        v=b-a
        if np.linalg.norm(v)<.01:continue
        angle=math.atan2(v[1],v[0]);c,s=math.cos(angle),math.sin(angle)
        rotation=np.array([[c,-s],[s,c]])
        p=points@rotation;lo,hi=p.min(0),p.max(0)
        area=np.prod(hi-lo)
        if best is None or area<best[0]:best=(area,(lo+hi)/2@rotation.T,hi-lo,-math.degrees(angle))
    _,center,size,heading=best
    return (*center,*size,heading)


@lru_cache(None)
def terrain_data(source):
    return json.loads((REFERENCES/f'{source}-terrain.json').read_text())


@lru_cache(None)
def terrain_grid(source):
    data=terrain_data(source)
    a=np.asarray([[p['east_m'],p['north_m'],p['ground_m']] for p in data['ground_samples']])
    e=np.unique(a[:,0]);n=np.unique(a[:,1])
    lookup={(p['east_m'],p['north_m']):p['ground_m'] for p in data['ground_samples']}
    return e,n,lookup


def terrain_height(source,x,z):
    """Bilinear production elevation at an east/north offset from survey anchor."""
    e,n,lookup=terrain_grid(source)
    i=max(0,min(len(e)-2,int(np.searchsorted(e,x)-1)))
    j=max(0,min(len(n)-2,int(np.searchsorted(n,z)-1)))
    u=np.clip((x-e[i])/(e[i+1]-e[i]),0,1);v=np.clip((z-n[j])/(n[j+1]-n[j]),0,1)
    return float((1-u)*(1-v)*lookup[e[i],n[j]]+u*(1-v)*lookup[e[i+1],n[j]]+(1-u)*v*lookup[e[i],n[j+1]]+u*v*lookup[e[i+1],n[j+1]])


def plan_block(m,source,way_id,height,peak,lod,color=STONE,base=0,roof_color=SLATE,windows=True,hip=False):
    x,z,w,d,heading=rectangle_fit(plan_ring(source,way_id))
    part=CastleMesh(getattr(m,'wall_material',82),getattr(m,'roof_material',86));block(part,0,0,w,d,height,lod,color,base=base,peak=peak,roof_color=roof_color,windows=windows,hip=hip)
    m.merge(turn(part,heading,x,0,z))
    return x,z,w,d,heading


def shift_component(m,start,x=0,y=0,z=0):
    for points,_,_ in m.triangles[start:]:points+=np.array([x,y,z])


def surveyed_wall(m,source,points,height,lod,color,thick=2.3,closed=True,datum=None):
    if datum is None:datum=terrain_data(source)['ground_m']
    heading=FRAMES.get(source,((0,0),0))[1]
    c,s=math.cos(math.radians(heading)),math.sin(math.radians(heading))
    ends=np.roll(points,-1,axis=0) if closed else points[1:]
    for a,b in zip(points,ends):
        length=np.linalg.norm(b-a)
        if length<.25:continue
        count=max(1,math.ceil(length/6))
        for i in range(count):
            p=a+(b-a)*i/count;q=a+(b-a)*(i+1)/count
            x,z=(p+q)/2
            wall(m,p,q,height,lod,color,thick,base=terrain_height(source,c*x+s*z,-s*x+c*z)-datum-1)


def face(m, points, color, normal=(0, 1, 0), material=11):
    p = np.asarray(points, dtype=float)
    # A collapsed first edge is valid at a hip apex or clipped rock course.
    # Orient from the first nondegenerate fan triangle, preserving the fan.
    for i in range(1,len(p)-1):
        cross=np.cross(p[i]-p[0],p[i+1]-p[0])
        if np.linalg.norm(cross)>1e-9:
            if np.dot(cross,normal)<0:p=p[::-1]
            break
    for i in range(1, len(p)-1):
        m.triangle(p[0], p[i], p[i+1], color, material)


def turn(m, degrees, x=0, y=0, z=0):
    c, s = math.cos(math.radians(degrees)), math.sin(math.radians(degrees))
    return m.transform([[c,0,s,x],[0,1,0,y],[-s,0,c,z],[0,0,0,1]])


def lathe(m, x, z, profile, color, n=24, cap=True, material=11):
    rings = [[(x+r*math.cos(i*math.tau/n), y, z+r*math.sin(i*math.tau/n))
              for i in range(n)] for y, r in profile]
    for k, (a,b) in enumerate(zip(rings,rings[1:])):
        for i in range(n):
            j=(i+1)%n
            col = tuple(max(0,min(255,v+(i%3-1)*3)) for v in color)
            m.quad(a[i],b[i],b[j],a[j],col,material)
    if cap:
        for ring,(y,r),normal in [(rings[0],profile[0],(0,-1,0)),(rings[-1],profile[-1],(0,1,0))]:
            if r:
                for i in range(n):face(m,[(x,y,z),ring[i],ring[(i+1)%n]],color,normal,material)


def roof_surface(m,points,color,lod):
    a,b,c,d=map(np.asarray,points)
    rows=max(1,math.ceil(np.linalg.norm(d-a)/(4 if lod==0 else 9)))
    columns=max(1,math.ceil(np.linalg.norm(b-a)/(3 if lod==0 else 8)))
    for i in range(rows):
        for j in range(columns):
            def point(u,v):return (1-v)*((1-u)*a+u*b)+v*((1-u)*d+u*c)
            value=((i*7+j*3)%7-3)*1.4
            tint=tuple(int(np.clip(v+value,0,255)) for v in color)
            face(m,[point(j/columns,i/rows),point((j+1)/columns,i/rows),point((j+1)/columns,(i+1)/rows),point(j/columns,(i+1)/rows)],tint,(0,1,0),12)


def roof(m,x,z,w,d,eave,peak,color=SLATE,hip=False,lod=2,wall_color=PALE):
    if w>d:
        part=Mesh();roof(part,0,0,d,w,eave,peak,color,hip,lod,wall_color)
        m.merge(turn(part,90,x,0,z))
        return
    a,b,c,e=(x-w/2,eave,z-d/2),(x+w/2,eave,z-d/2),(x+w/2,eave,z+d/2),(x-w/2,eave,z+d/2)
    if hip:
        lo,hi=(x,peak,z-d/2+min(d/2,w*.42)),(x,peak,z+d/2-min(d/2,w*.42))
        face(m,[a,b,lo],color,(0,1,-1),12);face(m,[e,hi,c],color,(0,1,1),12)
    else:
        lo,hi=(x,peak,z-d/2),(x,peak,z+d/2)
        face(m,[a,b,lo],wall_color,(0,0,-1));face(m,[e,hi,c],wall_color,(0,0,1))
    roof_surface(m,[a,lo,hi,e],color,lod)
    roof_surface(m,[b,c,hi,lo],tuple(v+8 for v in color),lod)
    m.beam(lo,hi,.15,tuple(min(255,v+13) for v in color),12,4) if d>w*.84 or not hip else None


def aperture(m,x,y,z,w,h,color=PALE,lod=0,arched=False,trim_material=82):
    # Thin surface relief preserves a dark recess at all viewing distances.
    face(m,[(x-w/2,y,z),(x+w/2,y,z),(x+w/2,y+h,z),(x-w/2,y+h,z)],DARK,(0,0,-1),15)
    if lod<2:
        for dx in [-w/2-.11,w/2+.11]:
            face(m,[(x+dx-.09,y-.12,z-.07),(x+dx+.09,y-.12,z-.07),(x+dx+.09,y+h+.12,z-.07),(x+dx-.09,y+h+.12,z-.07)],color,(0,0,-1),trim_material)
        for yy in [y-.12,y+h+.12]:
            face(m,[(x-w/2-.2,yy-.085,z-.07),(x+w/2+.2,yy-.085,z-.07),(x+w/2+.2,yy+.085,z-.07),(x-w/2-.2,yy+.085,z-.07)],color,(0,0,-1),trim_material)
        if w>1.2:
            face(m,[(x-.05,y,z-.1),(x+.05,y,z-.1),(x+.05,y+h,z-.1),(x-.05,y+h,z-.1)],color,(0,0,-1),trim_material)
    if arched:
        for i in range(8 if lod<2 else 4):
            a,b=i*math.pi/(8 if lod<2 else 4),(i+1)*math.pi/(8 if lod<2 else 4)
            m.beam((x+w/2*math.cos(a),y+h+w/2*math.sin(a),z-.05),(x+w/2*math.cos(b),y+h+w/2*math.sin(b),z-.05),.12,color,trim_material,4)


def masonry_variation(m,span,base,height,depth,color,lod,material):
    if lod==2:return
    # Discrete ashlar/brick units carry low-frequency variation after the atlas
    # mips average out. Fine mortar relief remains the construction atlas's job.
    course=.32 if material==MAT.BRICK_WALL else .7
    width=.85 if material==MAT.BRICK_WALL else 1.6
    stride=1 if lod==0 else 2
    for row,y in enumerate(np.arange(base+.15,base+height-.2,course*stride)):
        for col,x in enumerate(np.arange(-span/2,span/2,width*stride)):
            if (row*3+col)%5:continue
            left=max(-span/2+.02,x+(row%2)*width*.5)
            right=min(span/2-.02,left+width-.035)
            if right<=left:continue
            top=min(base+height-.03,y+course-.03)
            value=((row*11+col*7)%13)-6-(4 if y<base+2 else 0)
            tint=tuple(int(np.clip(v+value,0,255)) for v in color)
            face(m,[(left,y,-depth/2-.025),(right,y,-depth/2-.025),(right,top,-depth/2-.025),(left,top,-depth/2-.025)],tint,(0,0,-1),material)


def block(m,x,z,w,d,h,lod,color=STONE,base=0,peak=None,roof_color=SLATE,hip=False,windows=True,material=None,roof_material=None):
    if material is not None or roof_material is not None:
        part=CastleMesh(material or 11,roof_material or 12)
        block(part,x,z,w,d,h,lod,color,base,peak,roof_color,hip,windows)
        m.merge(part)
        return
    m.box((x,base+h/2,z),(w,h,d),color)
    if peak is not None:roof(m,x,z,w+.6,d+.6,base+h,base+peak,roof_color,hip,lod,color)
    if windows:
        for angle,span,depth in [(0,w,d),(180,w,d),(90,d,w),(270,d,w)]:
            f=Mesh()
            surface_material=getattr(m,'wall_material',MAT.STONE_WALL)
            if surface_material in (MAT.STONE_WALL,MAT.WEATHERED_LIMESTONE,MAT.BRICK_WALL):masonry_variation(f,span,base,h,depth,color,lod,surface_material)
            for yy in np.arange(base+3,base+h-1,4.6 if h<25 else 5.5):
                for xx in np.arange(-span/2+2.4,span/2-1.0,3.7 if lod<2 else 6):
                    aperture(f,float(xx),float(yy),-depth/2-.06,1.1,1.9,color,lod)
            if lod<2:
                for yy in np.arange(base+1,base+h,4.6):f.box((0,float(yy),-depth/2-.08),(span,.19,.17),tuple(min(255,v+8) for v in color))
            m.merge(turn(f,angle,x,0,z))


def wall(m,a,b,h,lod,color=STONE,thick=2.2,base=0,crenel=True):
    dx,dz=b[0]-a[0],b[1]-a[1];length=math.hypot(dx,dz)
    w=Mesh();w.box((0,base+h/2,0),(length,h,thick),color)
    if crenel:
        w.box((0,base+h-.18,0),(length,.45,thick+.45),tuple(min(255,v+9) for v in color))
        n=max(2,round(length/2.6))
        for i in range(n):w.box((-length/2+(i+.5)*length/n,base+h+.6,0),(length/n*.52,1.25,thick+.35),color)
    if lod<2:
        for yy in np.arange(base+2,base+h,3):
            w.box((0,float(yy),-thick/2-.025),(length,.075,.06),tuple(max(0,v-8) for v in color))
    m.merge(turn(w,-math.degrees(math.atan2(dz,dx)),(a[0]+b[0])/2,0,(a[1]+b[1])/2))


def round_tower(m,x,z,r,h,lod,color=STONE,base=0,cone=None,hollow=False,roof_color=SLATE,material=None,roof_material=None):
    if material is not None or roof_material is not None:
        part=CastleMesh(material or 11,roof_material or 12)
        round_tower(part,x,z,r,h,lod,color,base,cone,hollow,roof_color)
        m.merge(part)
        return
    n=(28,20,12)[lod]
    lathe(m,x,z,[(base-.5,r*1.08),(base+1,r),(base+h,r)],color,n,not hollow)
    if hollow:
        # Roofless ruins expose inner masonry and a dark lower floor.
        lathe(m,x,z,[(base+2,r-1.8),(base+h,r-1.8)],tuple(v-22 for v in color),n,False)
        for i in range(n):
            a,b=i*math.tau/n,(i+1)*math.tau/n
            face(m,[(x+rr*math.cos(t),base+h,z+rr*math.sin(t)) for rr,t in [(r,a),(r,b),(r-1.8,b),(r-1.8,a)]],color)
        lathe(m,x,z,[(base+1.9,r-1.8),(base+2,r-1.8)],(75,77,69),n)
    if cone is not None:
        lathe(m,x,z,[(base+h-.2,r*1.16),(base+h+.4,r*1.18),(base+cone,.03)],roof_color,n,material=12)
        lathe(m,x,z,[(base+cone-.8,.15),(base+cone,.03)],SLATE,8,material=12)
    else:
        count=max(8,round(r*2.4))
        for i in range(count):
            a=i*math.tau/count;b=Mesh();b.box((0,base+h+.55,0),(1.15,1.1,1.1),color)
            m.merge(turn(b,-math.degrees(a),x+(r-.45)*math.cos(a),0,z+(r-.45)*math.sin(a)))
    for yy in [base+h*.34,base+h*.68]:
        for i in range(6 if lod<2 else 4):
            a=i*360/(6 if lod<2 else 4);f=Mesh();aperture(f,0,yy,-r-.04,.38,1.45,color,lod)
            m.merge(turn(f,a,x,0,z))
    if lod<2:
        for yy in np.arange(base+4,base+h,5):lathe(m,x,z,[(float(yy),r+.08),(float(yy)+.16,r+.08)],tuple(v+7 for v in color),n)


def court(m,poly,y,color=(152,146,128),skirt=3):
    # Only a shallow architectural retaining skirt is authored, never a mountain.
    for piece in _footprint_pieces(poly):face(m,[(x,y,z) for x,z in piece],color)
    cx=sum(x for x,z in poly)/len(poly);cz=sum(z for x,z in poly)/len(poly)
    for a,b in zip(poly,poly[1:]+poly[:1]):face(m,[(a[0],y-skirt,a[1]),(b[0],y-skirt,b[1]),(b[0],y,b[1]),(a[0],y,a[1])],STONE,((a[0]+b[0])/2-cx,0,(a[1]+b[1])/2-cz),MAT.STONE_WALL)


def measured_supports(m,name,lod):
    """Join surveyed terrain to architectural feet without moving roof datums.

    Each stored toe was queried at its final EN coordinate. Rock contours are
    bounded by the occupied crest; masonry contours continue actual wall feet.
    An embedded toe tolerates the coarser rendered terrain triangulation.
    """
    path=REFERENCES/f'{name}-ground-supports.json'
    if not path.exists():return
    data=json.loads(path.read_text())
    for support in data['supports']:
        vertices=support['vertices'];top=support['top_y_m'];color=support['color']
        rock=support['kind']=='rock';material=support['material']
        # Side faces overlap the architectural foot by4cm; the interior cap
        # stays below existing courtyard paving to avoid coplanar surfaces.
        top_ring=[(v['top_xz'][0],top-.12,v['top_xz'][1]) for v in vertices]
        center=np.mean(np.asarray(top_ring),axis=0)
        for i,a in enumerate(top_ring):face(m,[center,a,top_ring[(i+1)%len(top_ring)]],color,material=material)
        levels=(5,4,3)[lod] if rock else (4,3,1)[lod]
        rings=[]
        for j in range(levels+1):
            t=j/levels;ring=[]
            for i,v in enumerate(vertices):
                a=np.asarray(v['top_xz']);b=np.asarray(v['toe_xz']);xz=a+(b-a)*t
                if rock and 0<j<levels:
                    # Shallow bedding ledges articulate the exposed cliff;
                    # the surveyed boundary vertices never move.
                    direction=b-a;length=np.linalg.norm(direction)
                    if length>.01:xz+=direction/length*(.24*math.sin(i*1.7+j*.9))*math.sin(t*math.pi)
                ring.append((float(xz[0]),top+(v['toe_y_m']-top)*t,float(xz[1])))
            rings.append(ring)
        for j,(a,b) in enumerate(zip(rings,rings[1:])):
            for i in range(len(a)):
                k=(i+1)%len(a);normal=np.asarray(a[i])+np.asarray(a[k])-2*center;normal[1]=0
                shade=((i%5)-2)*1.6+(j%2)*2.0
                tint=tuple(int(np.clip(v+shade,0,255)) for v in color)
                face(m,[a[i],a[k],b[k],b[i]],tint,normal,material)


def neuschwanstein(lod):
    m=CastleMesh(82,86);white=(224,222,205);red=(167,91,69)
    court(m,[(-77,-20),(-68,10),(64,17),(75,10),(75,-29),(39,-31),(9,-24),(-14,-20)],0)
    # Bavarian palace administration plan: Palas west, gateway east;
    # lower/upper court gap remains open, and no unbuilt central keep is added.
    block(m,-50,-1,24,29,34,lod,white,peak=47)
    block(m,-33,-1,22,24,32,lod,white,peak=45)
    block(m,-16,0,13,21,28,lod,white,peak=38)
    for x,z,r,h,peak in [(-44,16,3.85,49,65),(-47,-12,3.0,37,49),(-28,15,2.6,39,49),(-68,-13,2.8,30,40)]:
        round_tower(m,x,z,r,h,lod,white,cone=peak)
    block(m,1,15,23,11,22,lod,white,peak=30)
    block(m,3,-19,22,12,19,lod,white,peak=27)
    block(m,47,14,30,6,14,lod,white,peak=19)
    block(m,30,14,8,9,35,lod,white,windows=True)
    for dx,dz in [(-3,-3),(-3,3),(3,-3),(3,3)]:round_tower(m,30+dx,14+dz,1.1,38,lod,white,cone=44)
    block(m,70,-5,9,36,16,lod,red,peak=23,material=MAT.BRICK_WALL)
    # White upper courtyard face and red external gatehouse face.
    m.box((65.4,9,-5),(.25,14,34),white)
    for z in [-23,14]:round_tower(m,71,z,2.9,18,lod,red,cone=29,material=MAT.BRICK_WALL)
    for z in [-18,10]:
        wall(m,(9,z),(38,z if z==10 else -22),5,lod,white,1.4)
    for xx in np.arange(-59,-17,4.8):
        f=Mesh();aperture(f,float(xx),25,-15.13,2.1,3.8,white,lod,True);m.merge(f)
    if lod<2:
        for x in [-58,-49,-39,-30,-20]:
            block(m,x,0,1.0,1.6,3,lod,white,base=43,windows=False)
        for z in [-14,14]:
            for x in [-57,-51,-44,-37,-30]:
                dm=Mesh();block(dm,0,0,2.3,2.8,2.1,lod,white,base=36,peak=4.2);m.merge(turn(dm,90,x,0,z))
    return m


def conwy(lod):
    m=CastleMesh(MAT.WEATHERED_LIMESTONE,MAT.SLATE_ROOF);stone=(148,147,131)
    # Digitised from the Cadw 0–30m scale: pixel(450,475) is local origin.
    def p(px,py):return ((px-450)*30/160,(475-py)*30/160)
    outer=[p(*v) for v in [(220,390),(377,398),(523,401),(675,401),(685,547),(523,557),(372,600),(232,520)]]
    court(m,outer,0,(125,133,107),3)
    for a,b in zip(outer,outer[1:]+outer[:1]):wall(m,a,b,12,lod,stone,3)
    for i,(x,z) in enumerate(outer):
        round_tower(m,x,z,6.1 if i!=6 else 6.5,21,lod,stone,hollow=True)
        if i in [2,3,4,5]:round_tower(m,x+2.9,z+2.7,1.85,27,lod,stone)
    wall(m,p(526,414),p(528,545),11,lod,stone,3)
    # Standing partition walls and roofless royal apartments, not imaginary roofs.
    for a,b,h in [((236,493),(283,486),10),((283,486),(349,509),9),((349,509),(414,521),9),((414,521),(466,503),9),((466,503),(490,563),9),((283,486),(266,547),9),((323,500),(306,560),8),((414,521),(431,577),10),((548,499),(673,492),11),((620,413),(625,492),12),((600,495),(600,543),10)]:
        wall(m,p(*a),p(*b),h,lod,stone,1.4,crenel=False)
    # Great Hall's transverse stone arches survive after roof stripping.
    for px in [337,392]:
        x,z=p(px,552)
        for t in np.linspace(0,math.pi,12 if lod<2 else 7)[:-1]:
            dt=math.pi/(11 if lod<2 else 6)
            m.beam((x,5+4.8*math.sin(t),z+4.8*math.cos(t)),(x,5+4.8*math.sin(t+dt),z+4.8*math.cos(t+dt)),.37,stone,11,5)
    for poly in [[(190,391),(205,520),(230,520)],[(700,403),(769,449),(775,516),(706,535)]]:
        pts=[p(*v) for v in poly]
        for a,b in zip(pts,pts[1:]):wall(m,a,b,5,lod,stone,1.6)
    return m


def japanese_roof(m,x,z,w,d,y,rise,lod,color):
    # Hipped kawara eaves curve gently upward; individual tile ribs are retained
    # in near LOD, while ridge caps and gold/white fascia survive at distance.
    steps=(5,4,3)[lod]
    for k in range(steps):
        t,u=k/steps,(k+1)/steps
        def ring(q):
            ww=w*(1-q);dd=d*(1-.52*q)
            yy=y+rise*q+.45*(1-q)**4
            return [(x-ww/2,yy,z-dd/2),(x+ww/2,yy,z-dd/2),(x+ww/2,yy,z+dd/2),(x-ww/2,yy,z+dd/2)]
        a,b=ring(t),ring(u)
        for i in range(4):face(m,[a[i],a[(i+1)%4],b[(i+1)%4],b[i]],color,(0,1,0),12)
    for a,b in zip(ring(0),ring(0)[1:]+ring(0)[:1]):m.beam(a,b,.14,(201,202,180),17,4)
    if lod==0:
        for xx in np.arange(-w/2+.8,w/2,.9):
            for side in [-1,1]:m.beam((x+xx,y+.49,z+side*d/2),(x+xx*.56,y+rise+.04,z+side*d*.24),.035,tuple(min(255,v+15) for v in color),12,4)


def japanese_keep(m,x,z,w,d,levels,lod,base=0,black=False,gold=False):
    # levels contain independently measured eave and ridge heights, not floors.
    plaster=(223,224,206);timber=(43,48,47) if black else (209,211,190)
    tile=(72,81,79) if black else (69,112,104)
    lower=base
    for j,(eave,ridge,scale) in enumerate(levels):
        ww,dd=w*scale,d*scale
        m.box((x,(lower+eave)/2,z),(ww,eave-lower,dd),plaster,MAT.PLASTER_WALL)
        m.box((x,lower+(eave-lower)*.29,z),(ww+.03,(eave-lower)*.58,dd+.03),timber,MAT.TIMBER_WALL if black else MAT.PLASTER_WALL)
        japanese_roof(m,x,z,ww+3,dd+3,eave,ridge-eave,lod,tile)
        for angle,span,dep in [(0,ww,dd),(180,ww,dd),(90,dd,ww),(270,dd,ww)]:
            f=Mesh()
            for xx in np.arange(-span/2+1.2,span/2-.4,1.7 if lod<2 else 2.8):
                aperture(f,float(xx),lower+.8,-dep/2-.08,.67,min(1.25,(eave-lower)*.42),timber,lod,trim_material=MAT.TIMBER_WALL)
            if gold:
                for xx in np.arange(-span/2+1,span/2,3):
                    f.box((float(xx),eave-.45,-dep/2-.12),(.4,.4,.12),(204,173,77),MAT.SCULPTURE_METAL)
            m.merge(turn(f,angle,x,0,z))
        if j in [0,1,2] and j<len(levels)-1:
            for angle in [0,180]:
                g=Mesh();wwg=ww*.44;dep=dd/2+1
                face(g,[(-wwg/2,eave, -dep),(wwg/2,eave,-dep),(0,ridge+2,-dep)],plaster,(0,0,-1),MAT.PLASTER_WALL)
                for a,b in [((-wwg/2,eave,-dep),(0,ridge+2,-dep)),((0,ridge+2,-dep),(wwg/2,eave,-dep))]:g.beam(a,b,.25,tile,12,5)
                m.merge(turn(g,angle,x,0,z))
        lower=eave+.7
    if gold:
        top=levels[-1][1]
        for side in [-1,1]:
            z0=z+side*d*levels[-1][2]*.24
            for a,b,r in [((x,top,z0),(x,top+1.1,z0+side*.45),.22),((x,top+1.1,z0+side*.45),(x,top+2,z0+side*.22),.15)]:m.beam(a,b,r,(212,172,61),MAT.SCULPTURE_METAL,6)


def stone_batter(m,x,z,w,d,h,lod):
    # No random rocks: deterministic staggered joints keep the masonry scale
    # stable across LODs and the concave batter follows the stone foundation.
    n=(7,5,3)[lod]
    for k in range(n):
        y0,y1=h*k/n,h*(k+1)/n
        if k==0:y0=-.45
        s0,s1=1-.19*(k/n)**.65,1-.19*((k+1)/n)**.65
        corners=lambda s,y:[(x-w*s/2,y,z-d*s/2),(x+w*s/2,y,z-d*s/2),(x+w*s/2,y,z+d*s/2),(x-w*s/2,y,z+d*s/2)]
        a,b=corners(s0,y0),corners(s1,y1)
        for side in range(4):
            aa,ab,ba,bb=np.array(a[side]),np.array(a[(side+1)%4]),np.array(b[side]),np.array(b[(side+1)%4])
            count=max(3,int(np.linalg.norm(ab-aa)/(2 if lod==0 else 4)))
            for j in range(count):
                t,u=j/count,(j+1)/count
                col=(133+(j*7+k*11)%29,133+(j*7+k*11)%26,118+(j*7+k*11)%26)
                face(m,[aa+(ab-aa)*t,aa+(ab-aa)*u,ba+(bb-ba)*u,ba+(bb-ba)*t],col,tuple(((aa+ab)/2-np.array((x,0,z)))*[1,0,1]),MAT.STONE_WALL)


def matsumoto(lod):
    m=CastleMesh(83,85)
    stone_batter(m,0,0,22,25,5,lod)
    japanese_keep(m,0,0,17,19,[(9,11.7,1),(13.2,15.2,.90),(17,19,.67),(21,23.1,.49),(26,29.4,.39)],lod,5,True)
    # Northwest small keep, connecting gallery, southeast wing and east-facing
    # vermilion moon-viewing room are separate, asymmetric structures.
    stone_batter(m,-3.5,17,13,13,4,lod)
    japanese_keep(m,-3.5,17,9.3,9.8,[(7.5,9.7,1),(11.7,13.5,.83),(14.5,16.8,.64)],lod,4,True)
    stone_batter(m,-3,9,10,12,4,lod)
    japanese_keep(m,-3,9,8,10,[(8.5,12,1)],lod,4,True)
    stone_batter(m,9,-10,12,15,4,lod)
    japanese_keep(m,9,-10,9,11,[(8.4,10.4,1),(11.8,14.7,.9)],lod,4,True)
    stone_batter(m,18,-10,10,13,4,lod)
    m.box((18,5.1,-10),(9,2.1,11),(220,219,199))
    for xx in [14,22]:
        for zz in [-15,-10,-5]:m.box((xx,8,zz),(.24,4,.24),(153,54,41),MAT.TIMBER_WALL)
    for xx in np.arange(14,22.1,1):
        for zz in [-15,-5]:m.box((float(xx),7,zz),(.12,1.2,.13),(163,58,44),MAT.TIMBER_WALL)
    for zz in [-15,-5]:m.box((18,7.65,zz),(8.2,.15,.2),(163,58,44),MAT.TIMBER_WALL)
    japanese_roof(m,18,-10,11,13,9,2.1,lod,(76,83,81))
    return m


def osaka(lod):
    m=CastleMesh(83,88);stone_batter(m,0,0,44,40,13.5,lod)
    japanese_keep(m,0,0,34,29,[(20.0,23.8,1),(28.6,32,.90),(36.8,39.5,.69),(43,45.1,.48),(49.1,52.6,.40)],lod,13.5,False,True)
    # The top black-and-gold frieze and green roof are defining present-day
    # features; the building is the extant1931 reconstruction, not Toyotomi's.
    for angle,span,dep in [(0,13.6,11.6),(180,13.6,11.6),(90,11.6,13.6),(270,11.6,13.6)]:
        f=Mesh();f.box((0,46.0,-dep/2-.12),(span,1.5,.18),(42,49,44),MAT.TIMBER_WALL)
        for xx in np.arange(-span/2+.9,span/2,2.1):
            f.ellipsoid((float(xx),46,-dep/2-.25),(.6,.27,.1),(207,170,69),MAT.SCULPTURE_METAL,8,4)
            if lod<2:
                f.beam((float(xx)-.5,45.9,-dep/2-.3),(float(xx)+.5,46.2,-dep/2-.3),.1,(207,170,69),MAT.SCULPTURE_METAL,4)
        m.merge(turn(f,angle))
    # South entrance projects from the keep's stone podium.
    block(m,4,-25,10,12,5,lod,(214,211,190),base=4,windows=False)
    japanese_roof(m,4,-25,13,15,9,4,lod,(65,112,99))
    # The entrance is reached above the plaza on a stone landing and stairs;
    # the operator's accessibility plan also shows a separate external lift.
    m.box((4,1.81,-25),(10.6,4.50,12.6),(153,152,135),MAT.STONE_WALL)
    steps=(22,18,12)[lod]
    for i in range(steps):
        height=4*(i+1)/steps;front=-39+8*i/steps
        m.box((4,(height-.4)/2,front+4/steps),(5.2,height+.4,8/steps+.015),(176,176,159),MAT.STONE_WALL)
    for x in [1.2,6.8]:
        m.beam((x,.85,-38.8),(x,4.85,-31),.055,(97,101,92),17,6)
        for i in range(5):
            z=-38.5+i*1.8;y=(z+39)*.5
            m.beam((x,y,z),(x,y+.85,z),.05,(97,101,92),17,6)
    return m


def chillon(lod):
    m=CastleMesh(MAT.WEATHERED_LIMESTONE,MAT.TILE_ROOF);stone=(187,180,162);tile=(153,86,59)
    outline=[(-14,-51),(8,-51),(19,-40),(25,-17),(24,24),(16,44),(-2,51),(-18,39),(-25,7),(-24,-29)]
    court(m,outline,0,(169,161,139),3)
    # An oval rock produces unequal lakeside residential ranges and a defended
    # eastern edge. Three courts remain open around the square central keep.
    for x,z,w,d,h,pk in [(-16,-26,14,31,12,19),(-17,9,14,35,14,21),(-9,39,18,17,13,21),(3,-44,22,13,13,19),(17,-23,10,27,10,17),(18,20,10,30,12,18),(-1,20,21,7,11,16)]:
        block(m,x,z,w,d,h,lod,stone,peak=pk,roof_color=tile)
    block(m,1,0,12.5,12.5,21,lod,stone,peak=25,roof_color=tile,hip=True)
    for x,z,r,h,pk in [(22,-35,3.8,12,19),(25,-3,4,14,20),(21,32,3.4,13,20)]:
        round_tower(m,x,z,r,h,lod,stone,cone=pk,roof_color=tile)
        # Tile roofs override the standard slate turret palette.
        lathe(m,x,z,[(h-.1,r*1.18),(pk,.03)],tile,(28,20,12)[lod],material=MAT.TILE_ROOF)
    wall(m,(24,-28),(24,-8),10,lod,stone,1.2)
    wall(m,(25,2),(24,24),10,lod,stone,1.2)
    for x,z in [(-25,-24),(-24,-12),(-24,2),(-24,17),(-18,38)]:
        f=Mesh();aperture(f,0,5,-.1,2,3,stone,lod,True);m.merge(turn(f,90,x,0,z))
    # The narrow bridge meets the road without adding an artificial lake plane.
    m.box((33,-.2,-28),(20,.7,3),(127,105,76),8)
    for z in [-29.5,-26.5]:m.box((33,.8,z),(20,.85,.2),(94,76,55),17)
    return m


def edinburgh(lod):
    m=CastleMesh(82,86);sand=(173,159,132);roofcol=(108,111,108)
    # Named footprint fits retain the compact Crown Square and the separate
    # western barracks/hospital courts shown on Historic Environment Scotland's
    # orientation map. Relative terrace levels are architectural estimates.
    court(m,[(-25,-39),(42,-39),(42,29),(-25,29)],4,(151,146,133),6)
    for way,h,peak,y in [(290611982,18,24,4),(467503845,11,17,4),
            (511343921,12,18,4),(41294347,13,19,-7),
            (41299975,10,15,-6),(41299976,10,16,-6),
            (41299978,7,12,-5),(41299983,10,15,-4),
            (446654897,10,16,-2),(467503844,8,13,-3)]:
        plan_block(m,'edinburgh',way,h,peak,lod,sand,y,roofcol)
    hx,hz,hw,hd,ha=plan_block(m,'edinburgh',467503846,13,22,lod,sand,4,(86,101,94))
    # Rooflight, tall lancets and stepped ends distinguish the medieval hall.
    f=Mesh()
    for x in np.linspace(-hw/2+3,hw/2-3,4):aperture(f,float(x),8,-hd/2-.12,1.9,5.2,sand,lod,True)
    m.merge(turn(f,ha,hx,0,hz))
    plan_block(m,'edinburgh',1362947350,17,23,lod,sand,4,roofcol)
    # The chapel is the mapped9.8×5.7m structure north of Crown Square.
    plan_block(m,'edinburgh',41299985,6,8,lod,(197,185,158),0,roofcol,False)
    plan_block(m,'edinburgh',41299987,13,19,lod,sand,-6,roofcol)
    gx,gz,gw,gd,ga=plan_block(m,'edinburgh',41299992,10,14,lod,sand,-2,roofcol)
    for side in [-1,1]:round_tower(m,gx+side*5.8,gz-12,2.8,12,lod,sand,base=-2)
    # Half-Moon Battery is east of the Royal Palace; its face is curved while
    # the enclosing walls follow the mapped outline of Castle Rock.
    path=[(36+21*math.cos(a),9+24*math.sin(a)) for a in np.linspace(-math.pi/2,math.pi/2,18 if lod<2 else 10)]
    for a,b in zip(path,path[1:]):wall(m,a,b,8,lod,sand,3.5,base=-3)
    outline=plan_ring('edinburgh',4301292)
    for a,b in zip(outline,np.roll(outline,-1,axis=0)):
        if np.linalg.norm(b-a)<1:continue
        mid=(a+b)/2
        grade=terrain_height('edinburgh',*mid)-terrain_data('edinburgh')['ground_m']
        wall(m,a,b,5,lod,sand,2.0,base=grade-1)
    # Argyle Battery guns occupy the short northern parapet by the gate.
    for x,z in [(-3,77),(-11,76),(-19,75),(-27,74),(-35,73),(-43,72)]:
        m.box((x,-1,z),(1.5,1.0,1.6),(75,81,64),17)
        m.beam((x,-.3,z),(x,-.05,z+2.7),.17,(49,58,55),16,8)
    return m


def windsor(lod):
    m=CastleMesh(82,86);stone=(183,177,156)
    # Middle Ward motte and Round Tower separate the long Lower Ward from the
    # rectangular Upper Ward; preserving this asymmetry is the central invariant.
    lathe(m,0,0,[(-4,39),(0,35),(7,23),(9,20)],(122,136,98),(40,28,18)[lod],material=3)
    round_tower(m,0,0,16.2,31,lod,stone,base=9)
    for x,z in [(5,-15),(-7,14)]:round_tower(m,x,z,3.1,32.5,lod,stone,base=9)
    m.beam((0,40,0),(0,52,0),.11,(91,94,84),17,6)
    upper_start=len(m.triangles)
    court(m,[(39,-71),(181,-64),(195,52),(48,69)],0,(152,149,131),5)
    m.box((111,.12,-1),(107,.24,98),(104,130,83),8)
    # The north side contains the long St George's Hall. Its tall upper bays
    # differ from the two principal residential storeys of the other ranges.
    for x,z,w,d,h in [(110,60,136,22,23),(186,-5,23,131,24),(116,-62,138,21,22),(45,-1,15,91,17)]:
        block(m,x,z,w,d,h,lod,stone,windows=False)
        for angle,span,depth in [(0,w,d),(180,w,d),(90,d,w),(270,d,w)]:
            f=Mesh();masonry_variation(f,span,0,h,depth,stone,lod,MAT.STONE_WALL)
            for xx in np.arange(-span/2+3,span/2-2,5.4):
                if z==60 and angle==0 and -35<xx<28:
                    aperture(f,float(xx),10.5,-depth/2-.08,2.65,9.2,stone,lod,True)
                    aperture(f,float(xx),2.1,-depth/2-.08,1.15,2.2,stone,lod)
                else:
                    for yy,ww,hh in [(2.0,1.4,2.2),(7.2,1.8,3.3),(13.7,1.8,3.9)]:aperture(f,float(xx),yy,-depth/2-.08,ww,hh,stone,lod,yy>12)
            m.merge(turn(f,angle,x,0,z))
        for a,b in [((x-w/2,z-d/2),(x+w/2,z-d/2)),((x-w/2,z+d/2),(x+w/2,z+d/2))]:wall(m,a,b,1.2,lod,stone,1.0,base=h)
    # Lead roofs in the Upper Ward are documented in the Royal Household
    # fabric-repair reports. Separate pitched fields sit behind the parapets.
    for x,z,w,d,eave,peak,metal in [(68,60,40,19,23.05,26.0,True),(117,60,61,19,23.05,29.0,False),(162,60,27,19,23.05,26.5,True),
            (181,35,12,49,24.05,26.8,True),(191,35,8,49,24.05,26,True),(181,-28,12,68,24.05,27,True),(191,-28,8,68,24.05,26,True),
            (80,-62,60,8,22.05,24.5,True),(80,-54,60,7,22.05,24.5,True),(143,-65,60,11,22.05,25.3,True),(143,-55,60,8,22.05,24.8,True),
            (45,-1,12,85,17.05,20,True)]:
        part=CastleMesh(82,88 if metal else 86)
        roof(part,x,z,w,d,eave,peak,(105,113,116) if metal else SLATE,True,lod,stone);m.merge(part)
    # Chimney groups break the lead/slate ridges without replacing the
    # characteristic low crenellated parapet with a continuous tall gable.
    for x,z,y in [(60,68,26),(91,68,26),(148,68,27),(172,68,27),(177,14,27),(177,-43,27),(193,-52,27),(66,-68,25),(91,-68,25),(131,-68,26),(159,-68,26),(44,-28,21),(44,21,21)]:
        m.box((x,y+1.1,z),(2.3,2.8,1.35),stone)
        m.box((x,y+2.45,z),(2.7,.28,1.65),(199,191,171))
        for dx in [-.72,0,.72]:lathe(m,x+dx,z,[(y+2.55,.20),(y+3.45,.20)],(139,104,81),(8,6,5)[lod],material=MAT.BRICK_WALL)
    for x,z,r,h in [(49,58,6.5,29),(176,56,8,31),(191,29,6,29),(191,-35,7,32),(167,-68,8,31),(111,-71,10,34),(58,-64,7,28),(36,32,6,27)]:round_tower(m,x,z,r,h,lod,stone)
    shift_component(m,upper_start,x=-5,z=15)
    # Lower Ward St George's Chapel: long low nave, tall perpendicular windows,
    # flying buttresses/pinnacles. The Round Tower is not duplicated here.
    chapel=Mesh();block(chapel,0,0,18,72,18,lod,stone,peak=23,windows=False)
    block(chapel,0,-7,34,18,15,lod,stone,peak=19,windows=False)
    for side in [-1,1]:
        for z in np.arange(-32,35,6):
            chapel.box((side*10.5,8,float(z)),(1,16,1.4),stone)
            chapel.beam((side*10.5,15,z),(side*15,7,z),.55,stone,11,6)
            lathe(chapel,side*10.5,float(z),[(16,.65),(23,.12)],stone,6)
            f=Mesh();aperture(f,0,5,-.1,3.1,10,stone,lod,True);chapel.merge(turn(f,-side*90,side*9.1,0,float(z)+3))
    for z in [-36,36]:
        for x in [-8,8]:lathe(chapel,x,z,[(18,.8),(25,.1)],stone,6)
    m.merge(turn(chapel,69.093,-160.371,-8.7,-48.557))
    for ident,h,pk in [(131174659,10,15),(131174662,11,16),(131174663,7,12),
            (131174665,10,16),(131189024,16,21),(131175692,18,None),
            (131188501,13,None),(131188506,18,None),(131174667,10,None)]:
        plan_block(m,'windsor',ident,h,pk,lod,stone,-8.7)
    # Horseshoe Cloister's bent Tudor ranges preserve its open northeastern
    # side and the oblique street frontage instead of forming a rectangle.
    spine=[(-239,-55),(-247,-70),(-233,-88),(-223,-97),(-203,-86),(-188,-79)]
    for a,b in zip(spine,spine[1:]):
        length=math.dist(a,b);part=Mesh();block(part,0,0,length,7.6,10,lod,stone,base=-8.7,peak=16,roof_color=(94,97,98))
        m.merge(turn(part,-math.degrees(math.atan2(b[1]-a[1],b[0]-a[0])),(a[0]+b[0])/2,0,(a[1]+b[1])/2))
    # Curfew Tower remains a separate round medieval defensive tower.
    x,z,_,_,_=rectangle_fit(plan_ring('windsor',131228046))
    round_tower(m,x,z,7.2,18,lod,stone,base=-8.7)
    surveyed_wall(m,'windsor',plan_ring('windsor',23580556),5,lod,stone,1.8,datum=52.4)
    return m


@lru_cache(None)
def eltz_relief():
    data=json.loads((REFERENCES/'eltz-relief.json').read_text())
    return np.asarray(data['height_m']),data['x_m'][0],data['z_m'][0]


def eltz_measured_ground(x,z):
    a,x0,z0=eltz_relief()
    u,v=(x-x0)/2,(z-z0)/2
    j,i=int(np.clip(math.floor(u),0,a.shape[1]-2)),int(np.clip(math.floor(v),0,a.shape[0]-2))
    u,v=u-j,v-i
    return float((1-v)*((1-u)*a[i,j]+u*a[i,j+1])+v*((1-u)*a[i+1,j]+u*a[i+1,j+1]))


def eltz_ground(x,z):
    c,s=math.cos(math.radians(9)),math.sin(math.radians(9))
    blend=float(np.clip((192-max(abs(x),abs(z)))/64,0,1))
    blend=blend*blend*(3-2*blend)
    h=eltz_measured_ground(x,z)
    # DGM interpolation beneath roofs is not an interior floor survey. Keep the
    # occupied foundation pad at the measured courtyard datum, retaining relief
    # outside its compact36x65m mapped envelope.
    if -19<=x<=20 and -33<=z<=33:h=min(h,140.35)
    return blend*h+(1-blend)*terrain_height('eltz',c*x+s*z,-s*x+c*z)


def eltz_spur(m,lod):
    # Actual1m DHHN2016 relief supplies the narrow rock, not the operator's broad
    # 60m landscape description. The outer64m blends to production ground.
    step=(4,8,16)[lod]
    for x in range(-192,192,step):
        for z in range(-192,192,step):
            p=[(xx,eltz_ground(xx,zz)-140.4,zz) for xx,zz in [(x,z),(x+step,z),(x+step,z+step),(x,z+step)]]
            rise=max(v[1] for v in p)-min(v[1] for v in p)
            rock=rise>step*.65 and abs(x)<65 and abs(z)<85
            material=4 if rock else 3
            color=(113,108,94) if rock else (126,137,83)
            face(m,p,color,(0,1,0),material)


def eltz_outworks(m,lod):
    stone=(157,152,137)
    # Mapped ruins are the two razed western towers and their connecting walls,
    # not occupied roofed ranges. Rim heights are modest proportional estimates.
    for ident in [238981200,238981202,238981208,604621402]:
        p=plan_ring('eltz',ident)
        if sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
        center=p.mean(0)
        tower=ident in [238981200,238981202]
        inner=center+(p-center)*.62
        level=max(eltz_ground(*v) for v in p)+(2.4 if tower else 1.6)
        for i,(a,b) in enumerate(zip(p,np.roll(p,-1,axis=0))):
            ya,yb=eltz_ground(*a)-.6,eltz_ground(*b)-.6
            top=level
            normal=(b[1]-a[1],0,a[0]-b[0])
            face(m,[(a[0],ya-140.4,a[1]),(b[0],yb-140.4,b[1]),(b[0],top-140.4,b[1]),(a[0],top-140.4,a[1])],stone,normal,MAT.WEATHERED_LIMESTONE)
            if tower:
                aa,bb=inner[i],inner[(i+1)%len(p)]
                face(m,[(aa[0],ya-140.4,aa[1]),(aa[0],top-140.4,aa[1]),(bb[0],top-140.4,bb[1]),(bb[0],yb-140.4,bb[1])],tuple(v-17 for v in stone),(-normal[0],0,-normal[2]),MAT.WEATHERED_LIMESTONE)
                face(m,[(a[0],top-140.4,a[1]),(b[0],top-140.4,b[1]),(bb[0],top-140.4,bb[1]),(aa[0],top-140.4,aa[1])],stone,(0,1,0),MAT.WEATHERED_LIMESTONE)
        if not tower:
            face(m,[(v[0],level-140.4,v[1]) for v in p],stone,(0,1,0),MAT.WEATHERED_LIMESTONE)
    # Small historic outer houses require the measured uphill footing: native
    # coarse anchors are5–9m lower and bury these ranges in the authored spur.
    for ident,height,pitch in [(604621396,4.6,3.2),(604621397,3.2,1.8),(604621401,5.2,3.0),(238981207,4.8,3.1)]:
        p=plan_ring('eltz',ident)
        if sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
        floor=max(eltz_ground(*v) for v in p)+.15
        x,z,w,d,angle=rectangle_fit(p)
        part=CastleMesh(MAT.STONE_WALL,MAT.SLATE_ROOF)
        block(part,0,0,w,d,height,lod,(180,175,159),base=floor-140.4,peak=height+pitch,roof_color=(87,92,92))
        m.merge(turn(part,angle,x,0,z))
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            face(m,[(a[0],eltz_ground(*a)-140.9,a[1]),(b[0],eltz_ground(*b)-140.9,b[1]),(b[0],floor-140.4,b[1]),(a[0],floor-140.4,a[1])],stone,(b[1]-a[1],0,a[0]-b[0]),MAT.WEATHERED_LIMESTONE)


def eltz(lod):
    m=CastleMesh(82,86);stone=(178,168,148);wood=(91,58,46);cream=(222,209,181)
    court(m,[(-22,-19),(-23,14),(-10,26),(13,23),(25,4),(20,-22)],0,(145,135,116),5)
    # Eight closely packed residential towers belong to the three family houses;
    # no curtain-wall quadrangle or four-corner turret pattern is substituted.
    houses=[(-13,-10,11,15,25,35,stone),(0,-17,13,11,23,32,cream),(13,-12,10,15,22,31,stone),(18,3,11,12,24,34,cream),(10,17,14,10,24,33,stone),(-3,21,12,10,20,30,cream),(-15,13,12,13,21,31,stone),(-20,1,8,12,18,27,cream)]
    # The southern middle house projects beyond the courtyard; its stone foot
    # penetrates the measured rock crest rather than leaving a5cm shadow slit.
    m.box((0,-.20,-17),(13,.50,11),stone,MAT.WEATHERED_LIMESTONE)
    for i,(x,z,w,d,h,pk,color) in enumerate(houses):
        block(m,x,z,w,d,h,lod,color,peak=pk,roof_color=(94,94,94),hip=i in [2,3,6],material=MAT.PLASTER_WALL if color==cream else MAT.STONE_WALL)
        if color==cream:
            for angle,span,dep in [(0,w,d),(180,w,d),(90,d,w),(270,d,w)]:
                f=Mesh()
                for yy in np.arange(h-9,h,3):
                    f.box((0,float(yy),-dep/2-.12),(span,.22,.15),wood,MAT.TIMBER_WALL)
                    for xx in np.arange(-span/2+.4,span/2,2.5):
                        f.box((float(xx),yy+1.5,-dep/2-.12),(.17,3,.15),wood,MAT.TIMBER_WALL)
                        if lod<2:f.beam((xx,yy,-dep/2-.16),(xx+2,yy+3,-dep/2-.16),.08,wood,MAT.TIMBER_WALL,4)
                m.merge(turn(f,angle,x,0,z))
        if lod<2:
            for side in [-1,1]:
                bay=CastleMesh(MAT.PLASTER_WALL,MAT.SLATE_ROOF);block(bay,0,0,2.7,2,5,lod,cream,base=h-6,peak=9,hip=True);m.merge(turn(bay,side*90,x+side*w/2,0,z))
    for x,z,r,h,pk in [(-21,-15,1.65,25,33),(22,-8,1.5,25,33),(14,24,1.5,27,35),(-9,25,1.45,25,34)]:round_tower(m,x,z,r,h,lod,stone,cone=pk)
    # Northern outer bailey and bridge approach are small and offset.
    court(m,[(-15,27),(-13,48),(11,50),(15,29)],-5,(151,145,126),4)
    block(m,7,41,8,18,6,lod,cream,base=-5,peak=10,roof_color=(92,92,91),material=MAT.PLASTER_WALL)
    wall(m,(-15,28),(-13,49),4,lod,stone,1,base=-5)
    m.box((-7,-3,57),(5,.8,19),(136,132,114),8)
    return m


def bran(lod):
    m=CastleMesh(83,85);plaster=(217,208,181);red=(150,74,48);stone=(169,156,132)
    court(m,[(-21,-12),(-15,22),(12,26),(23,7),(14,-20),(-4,-23)],0,(154,144,120),6)
    # Irregular curtain residential wings surround a genuine small inner court.
    for x,z,w,d,h,pk in [(-15,0,9,29,17,23),(-2,19,24,9,16,23),(17,7,10,24,20,27),(7,-15,21,11,15,22)]:block(m,x,z,w,d,h,lod,plaster,peak=pk,roof_color=red)
    block(m,-15,19,12,13,23,lod,plaster,peak=32,roof_color=red,hip=True)
    block(m,17,-7,11,12,24,lod,plaster,peak=31,roof_color=red,hip=True)
    round_tower(m,-15,-14,4.7,23,lod,stone,cone=31,roof_color=red,material=MAT.STONE_WALL)
    lathe(m,-15,-14,[(23,5.5),(31,.04)],red,(28,20,12)[lod],material=MAT.TILE_ROOF)
    block(m,3,25,8,8,19,lod,plaster,peak=28,roof_color=red,hip=True)
    # Timber galleries face the court; visible dark posts and warm plaster differ
    # from the red ceramic roofs and masonry lower retaining walls.
    for z in [-8,-3,2,7,12]:
        m.box((-9.7,12,z),(.22,3,.22),(89,65,47),MAT.TIMBER_WALL)
        m.box((-9,10.6,z),(1.8,.18,4.9),(123,91,61),MAT.TIMBER_WALL)
    for y in [10.6,13.4]:m.box((-9.1,y,2),(1.6,.15,25),(89,65,47),MAT.TIMBER_WALL)
    if lod<2:
        for x,z,h in [(-18,0,26),(-10,20,27),(13,-16,25),(20,3,30)]:block(m,x,z,1,1.4,3,lod,plaster,base=h,windows=False)
    return m


@lru_cache(None)
def predjama_relief():
    return json.loads((REFERENCES/'predjama-relief.json').read_text())


@lru_cache(None)
def predjama_grid():
    return np.asarray(predjama_relief()['height_m'])


def predjama_ground(x,z):
    data=predjama_relief();a=predjama_grid()
    u=(x-data['x_m'][0])/2;v=(z-data['z_m'][0])/2
    i=max(0,min(a.shape[1]-2,int(u)));j=max(0,min(a.shape[0]-2,int(v)))
    u,v=np.clip(u-i,0,1),np.clip(v-j,0,1)
    return float(a[j,i]*(1-u)*(1-v)+a[j,i+1]*u*(1-v)+a[j+1,i]*(1-u)*v+a[j+1,i+1]*u*v)


def predjama_cliff(m,lod):
    # The ARSO1m DMR establishes the cliff's absolute foot and crest. OSM's
    # natural=cliff line supplies its curved plan; only the overhang is inferred
    # from the operator photograph because heightfields cannot encode a void.
    data=predjama_relief();datum=data['base_m']
    line=np.asarray(data['cliff_polyline_xz_m'])[1:-1]
    center=np.array([-7.,6.]);along=np.array([.48,.8772685]);outward=np.array([along[1],-along[0]])
    rock=(171,173,162)
    columns=[]
    for aa,bb in zip(line,line[1:]):
        segments=max(1,math.ceil(np.linalg.norm(bb-aa)/(2.5,4,7)[lod]))
        columns.extend(aa+(bb-aa)*j/segments for j in range(segments))
    columns.append(line[-1]);columns=np.asarray(columns)
    normals=[];lower=[];upper=[]
    for i,p in enumerate(columns):
        tangent=columns[min(len(columns)-1,i+1)]-columns[max(0,i-1)]
        tangent/=np.linalg.norm(tangent);normal=np.array([tangent[1],-tangent[0]])
        normals.append(normal)
        lower.append(min(predjama_ground(*(p+normal*r)) for r in [8,14,20])-datum-1)
        upper.append(max(predjama_ground(*(p-normal*r)) for r in [6,12,18,24])-datum)
    def vertex(i,y):
        y=np.clip(y,lower[i],upper[i]);p=columns[i]
        # Bedding ledges intersect vertical solution channels; coherent spatial
        # phases retain strata and fissures without per-triangle random noise.
        relief=1.1*math.sin(y*.52+p[0]*.03)+1.2*math.sin(p[0]*.36+p[1]*.27)+.8*math.sin(y*.14+p[0]*.32)
        q=p+normals[i]*relief
        return np.array([q[0],y,q[1]])
    # Shared column vertices and common absolute-height courses avoid cracks
    # where differently sloping crest profiles meet.
    heights=np.arange(math.floor(min(lower))-1,math.ceil(max(upper))+7,(2.5,4,6)[lod])
    for j in range(len(columns)-1):
        normal=normals[j]+normals[j+1];normal/=np.linalg.norm(normal)
        for lo,hi in zip(heights,heights[1:]):
            points=[vertex(j,lo),vertex(j+1,lo),vertex(j+1,hi),vertex(j,hi)]
            mid=sum(points)/4;along_cave=np.dot(mid[[0,2]]-center,along)
            vault=25+24*math.sqrt(max(0,1-(along_cave/18)**2))
            if abs(along_cave)<18 and -3<mid[1]<vault:continue
            shade=int(6*math.sin(mid[1]*.52+mid[0]*.03)+8*math.sin(mid[0]*.24+mid[2]*.11)+4*math.sin(mid[1]*.14+mid[0]*.32))
            face(m,points,tuple(v+shade for v in rock),(normal[0],0,normal[1]),4)
        top=[vertex(j,upper[j]),vertex(j+1,upper[j+1])]
        back=[columns[i]-normals[i]*24 for i in [j,j+1]]
        cap=[top[0],top[1],np.array([back[1][0],predjama_ground(*back[1])-datum,back[1][1]]),np.array([back[0][0],predjama_ground(*back[0])-datum,back[0][1]])]
        face(m,cap,(128,140,96),(0,1,0),3)
    # A recessed horseshoe-shaped chamber makes the rock shelter the upper
    # castle. Aperture36m wide/49m high and17m depth are photographic estimates.
    outline=[(-18,-3),(-18,25),(-15,37),(-10,45),(-4,49),(4,46),(13,36),(18,25),(18,-3)]
    def chamber(s,y,depth):
        q=center+along*s+outward*depth
        return (q[0],y,q[1])
    for (s0,y0),(s1,y1) in zip(outline,outline[1:]):
        a,b=chamber(s0,y0,3),chamber(s1,y1,3)
        c,d=chamber(s1*.70,y1+2,-15),chamber(s0*.70,y0+2,-15)
        color=(111,108,96) if (y0+y1)/2>32 else (137,133,119)
        face(m,[a,b,c,d],color,(outward[0],-1,outward[1]),4)
        # An irregular rock lip covers the coarse surface/aperture junction.
        oa=chamber(s0*1.34,25+(y0-25)*1.30,4.2)
        ob=chamber(s1*1.34,25+(y1-25)*1.30,4.2)
        face(m,[a,oa,ob,b],(160,163,151),(outward[0],0,outward[1]),4)
    face(m,[chamber(s*.7,y+2,-15) for s,y in outline],(59,58,50),(outward[0],0,outward[1]),4)
    ledge=[(-17,-15),(-4,-15),(-4,-1),(17,-1),(17,12),(-17,12)]
    for a,b in zip(ledge,ledge[1:]+ledge[:1]):
        floor_a=min(-8,predjama_ground(*a)-datum-2);floor_b=min(-8,predjama_ground(*b)-datum-2)
        face(m,[(a[0],floor_a,a[1]),(b[0],floor_b,b[1]),(b[0],-4,b[1]),(a[0],-4,a[1])],(158,159,146),(b[1]-a[1],0,a[0]-b[0]),4)


def predjama(lod):
    m=CastleMesh(83,85);white=(224,217,194);stone=(174,165,146);tile=(113,99,85)
    predjama_cliff(m,lod)
    # The Slovenian GURS footprint inherited by OSM is a32.5×25.2m L.
    # The low eastern approach enters the long facade; the western wing turns
    # toward the viewer, under the measured limestone crest.
    court(m,[(-16,-14),(-5,-14),(-5,0),(16,0),(16,11),(-16,11)],0,(142,132,112),7)
    block(m,0,5,32,10,21,lod,white,peak=27,roof_color=tile,hip=True)
    block(m,-11,-6,10,16,24,lod,white,peak=30,roof_color=tile,hip=True)
    block(m,-7,8,12,6,14,lod,white,base=15,peak=19,roof_color=tile,hip=True)
    # Front bays and shallow stone buttresses are measured relative to the
    # mapped facade; storey/roof heights are photographic estimates.
    for x in [-14,-7,1,8,15]:m.box((x,6,-.3),(1,12,1.1),stone)
    for y in [6,12,18]:m.box((2,y,-.15),(27,.3,.3),stone,17)
    for x,z,h in [(-13,-13.5,22),(14,0,20)]:block(m,x,z,2.8,2.6,4,lod,white,base=h-4,peak=6.3,roof_color=tile,hip=True)
    m.box((24,.35,2),(17,.7,2.6),(132,121,98),8)
    for z in [.7,3.3]:wall(m,(16,z),(32.5,z),1.1,lod,stone,.25,crenel=False)
    return m


def segovia(lod):
    m=CastleMesh(82,86);stone=(199,185,153);slate=(77,90,103)
    # The prow tapers westward. Local x follows the long palace axis, with
    # Juan II's broad rectangular tower at the eastern approach.
    outline=[(-72,0),(-49,15),(58,23),(63,-29),(-11,-22),(-53,-10)]
    court(m,outline,0,(151,142,122),7)
    for a,b in zip(outline,outline[1:]+outline[:1]):wall(m,a,b,12,lod,stone,2.3,crenel=False)
    for x,z,w,d,h,pk in [(-25,9,64,12,17,27),(-12,-15,52,12,16,25),(33,17,40,12,17,26),(25,-22,47,12,15,24),(-52,0,20,21,20,31)]:
        b=Mesh();block(b,0,0,d,w,h,lod,stone,peak=pk,roof_color=slate);m.merge(turn(b,90,x,0,z))
    # Two surviving courts remain open between the side ranges.
    for x,z,w,d in [(-26,-3,17,13),(25,-2,19,19)]:
        m.box((x,.16,z),(w,.3,d),(190,184,167),8)
        for side in [-1,1]:
            for xx in np.arange(-w/2+1,w/2,2.8):
                m.beam((x+xx,0,z+side*d/2),(x+xx,4,z+side*d/2),.19,stone,17,6)
            m.box((x,4.3,z+side*d/2),(w,.55,1),stone,17)
    block(m,55,-2,17,42,31,lod,stone,windows=False)
    for y in [8,17,26,30.6]:m.box((55,y,-2),(17.5,.45,42.5),(218,203,170),17)
    # Twelve corbelled bartizans crown Juan II, rather than conical roofs.
    for z in [-21,-13,-5,3,11,19]:
        for x in [46.4,63.6]:
            lathe(m,x,z,[(27,.65),(29,1.35),(34,1.35)],stone,(16,12,8)[lod])
            for a in range(4):
                aa=a*math.pi/2;m.box((x+math.cos(aa),34.5,z+math.sin(aa)),(.6,1,.6),stone)
    for x,z,r,h,pk in [(-68,0,3.5,21,35),(-49,13,3.3,24,37),(-46,-12,3.1,22,34),(-2,17,2.8,21,33),(22,-27,2.8,20,32),(43,23,2.5,23,35)]:
        round_tower(m,x,z,r,h,lod,stone,cone=pk)
    for z in np.arange(-18,19,5):
        f=Mesh();aperture(f,0,12,0,1.8,3.8,stone,lod,True);m.merge(turn(f,-90,63.6,0,float(z)))
    if lod<2:
        # Low-relief sgraffito on the great tower uses small repeated diamonds;
        # palette separation survives without turning stone into a noisy texture.
        for y in np.arange(3,28,2):
            for z in np.arange(-20,18,2.4):
                face(m,[(63.56,y,z),(63.56,y+.55,z+.55),(63.56,y+1.1,z),(63.56,y+.55,z-.55)],(181,164,134),(1,0,0),17)
    m.box((76,-.2,-1),(23,.7,5),(172,162,140),8)
    return m


def malbork(lod):
    m=CastleMesh(81,85);brick=(168,92,68);tile=(167,84,49);trim=(195,167,125)
    # High Castle's51×61m convent surrounds the measured26×32m court.
    # Its southern dansker bridge runs diagonally away from the southwest corner.
    court(m,[(-28,-33),(23,-33),(23,29),(-28,29)],0,(164,154,133),4)
    high=[(-22,-2,12,60,22,39),(18,-3,12,60,24,41),
          (-2,-26,29,13,22,38),(-2,21,40,16,24,42)]
    for x,z,w,d,h,pk in high:block(m,x,z,w,d,h,lod,brick,peak=pk,roof_color=tile)
    for side in [-1,1]:
        for z in np.arange(-17,13,4):m.beam((side*13,2,z),(side*13,9,z),.29,trim,17,6)
        for x in np.arange(-13,13,4):m.beam((x,2,side*16),(x,9,side*16),.29,trim,17,6)
        m.box((side*13,9.5,-2),(1,.7,32),brick)
    block(m,16,23,13,13,51,lod,brick,peak=64,roof_color=tile,hip=True)
    block(m,35,30,12,34,26,lod,brick,peak=43,roof_color=tile)
    block(m,-63,-58,14,13,18,lod,brick,peak=25,roof_color=tile,hip=True)
    a,b=np.array([-28,-32]),np.array([-58,-57]);length=np.linalg.norm(b-a)
    bridge=CastleMesh(MAT.BRICK_WALL,MAT.TILE_ROOF);block(bridge,0,0,length,5,5,lod,brick,base=9,peak=9,roof_color=tile,windows=False)
    m.merge(turn(bridge,-math.degrees(math.atan2(b[1]-a[1],b[0]-a[0])),(a[0]+b[0])/2,0,(a[1]+b[1])/2))
    for t in [.2,.5,.8]:
        x,z=a+(b-a)*t;m.box((x,4.5,z),(1.8,9,3),brick)
    # The three-sided Middle Castle opens south toward the High Castle. Its
    # river range and Grand Master's Palace follow separately mapped outlines.
    court(m,[(-49,60),(45,60),(48,151),(-43,160)],-2,(166,156,134),3)
    for x,z,w,d,h,pk,ang in [(-36,116,20,75,23,37,6.7),(36,106,15,87,19,33,1.5),(-2,147,81,18,19,35,6.7)]:
        part=CastleMesh(MAT.BRICK_WALL,MAT.TILE_ROOF);block(part,0,0,w,d,h,lod,brick,base=-2,peak=pk,roof_color=tile)
        m.merge(turn(part,ang,x,0,z))
    plan_block(m,'malbork',261512837,24,31,lod,brick,-2,tile,hip=True)
    for z in [61,68,75,82]:
        f=Mesh();aperture(f,0,9,0,2.2,7.4,trim,lod,True);m.merge(turn(f,90,-78,0,z))
    for x,z in [(-76,60),(-77,80)]:round_tower(m,x,z,2.1,25,lod,brick,base=-2,cone=32)
    for x in [4,13]:round_tower(m,x,160,3.6,20,lod,brick,base=-2,cone=29)
    # Karwan is the eastern armoury at local98,250; the long western service
    # ranges and St Lawrence chapel are separate extant mapped structures.
    for ident,h,pk in [(62144160,15,27),(62144157,24,31),(261513898,12,21),
            (101914542,10,19),(261513528,10,20),(101915237,10,18),
            (101915918,10,17),(62682251,8,15),(122122605,7,12),
            (261513127,8,15),(261512960,10,18),(985854910,10,18)]:
        x,z,_,_,_=rectangle_fit(plan_ring('malbork',ident))
        c,ss=math.cos(math.radians(35)),math.sin(math.radians(35))
        grade=terrain_height('malbork',c*x+ss*z,-ss*x+c*z)-terrain_data('malbork')['ground_m']
        plan_block(m,'malbork',ident,h,pk,lod,brick,grade,tile)
    for ident in [62144151,106152188,122122606,122123490,122123498,122123499,
            433802824,698950336,1173702785]:
        surveyed_wall(m,'malbork',plan_ring('malbork',ident),7,lod,brick,2,False)
    surveyed_wall(m,'malbork',plan_ring('malbork',122123495),4,lod,brick,2.5,False)
    for ident in [122122609,122123493,122122626,432786364]:
        x,z,_,_,_=rectangle_fit(plan_ring('malbork',ident))
        c,ss=math.cos(math.radians(35)),math.sin(math.radians(35))
        grade=terrain_height('malbork',c*x+ss*z,-ss*x+c*z)-terrain_data('malbork')['ground_m']
        plan_block(m,'malbork',ident,13,23,lod,brick,grade,tile,hip=True)
    if lod<2:
        for x,z,w,d,h,pk in high:
            for side in [-1,1]:
                for yy in range(6):m.box((x,h+yy*2.1,z+side*(d/2+.12)),(w*(1-yy/7),.35,.24),trim,17)
    return m


def alhambra(lod):
    m=CastleMesh(MAT.EARTHEN_PLASTER,MAT.TILE_ROOF);earth=(180,132,94);plaster=(219,199,164);tile=(155,90,57)
    # Coordinates follow the surveyed north/east frame. The Alcazaba lies west
    # of Charles V and the Nasrid palaces; the medina extends southeast.
    alc=[(-310,-10),(-301,37),(-206,55),(-193,-20),(-262,-31)]
    court(m,alc,0,(156,140,114),5)
    for a,b in zip(alc,alc[1:]+alc[:1]):wall(m,a,b,12,lod,earth,3.8)
    for x,z,w,d,h in [(-297,10,16,16,25),(-204,43,12.12,10.46,26),(-206,-8,16,18,21),(-266,48,10,10,16),(-308,-10,13,12,14)]:
        block(m,x,z,w,d,h,lod,earth,windows=False)
        for a,b in [((x-w/2,z-d/2),(x+w/2,z-d/2)),((x-w/2,z+d/2),(x+w/2,z+d/2))]:wall(m,a,b,1,lod,earth,1,base=h)
    for x in [-280,-262,-244,-226]:
        for z in [-7,11,27]:
            for a,b in [((x-6,z-5),(x+6,z-5)),((x-6,z-5),(x-6,z+5)),((x+6,z-5),(x+6,z+5))]:wall(m,a,b,1.5,lod,earth,.65,crenel=False)
    shift_component(m,0,y=terrain_height('alhambra',-250,10)-terrain_data('alhambra')['ground_m'])
    # Charles V:63m square with30m open circular court, two32-column galleries.
    charles_start=len(m.triangles)
    cx,cz=-83,-24
    n=(64,48,32)[lod]
    for i in range(n):
        a,b=i*math.tau/n,(i+1)*math.tau/n
        def outer(t):return (cx+31.5*math.cos(t)/max(abs(math.cos(t)),abs(math.sin(t))),17,cz+31.5*math.sin(t)/max(abs(math.cos(t)),abs(math.sin(t))))
        inner=lambda t:(cx+18*math.cos(t),17,cz+18*math.sin(t))
        face(m,[outer(a),outer(b),inner(b),inner(a)],(172,153,123))
        oa,ob=outer(a),outer(b)
        face(m,[(oa[0],0,oa[2]),(ob[0],0,ob[2]),ob,oa],plaster,(math.cos((a+b)/2),0,math.sin((a+b)/2)))
    for y in [0,8.1]:
        for i in range(32):
            a=i*math.tau/32;x,z=cx+15*math.cos(a),cz+15*math.sin(a)
            lathe(m,x,z,[(y,.5),(y+.5,.4),(y+6.9,.4),(y+7.4,.65)],plaster,(12,8,6)[lod])
        lathe(m,cx,cz,[(y+7.4,15.2),(y+8,15.2)],plaster,n,False)
    for i in range(charles_start,len(m.triangles)):
        p,c,material=m.triangles[i]
        if material==MAT.EARTHEN_PLASTER:m.triangles[i]=(p,c,MAT.STONE_WALL)
    # Myrtles: published36×23m court and34×7.1m pool; Comares'45m tower is
    # measured from the downhill external foot, extending below the palace pad.
    mx,mz=-60,29
    m.box((mx,.15,mz),(23,.3,36),PALE,8);m.box((mx,.33,mz),(7.1,.08,34),(71,116,111),3)
    for x in [mx-5.7,mx+5.7]:m.box((x,.75,mz),(2,1.1,34),(82,108,62),9)
    for x,z,w,d,h in [(mx-15,mz,7,43,8),(mx+15,mz,7,43,8),(mx,mz-23,29,8,11),(mx,mz+23,29,8,11)]:block(m,x,z,w,d,h,lod,plaster,peak=h+4,roof_color=tile,windows=False,material=MAT.PLASTER_WALL)
    block(m,mx,mz+33,17,17,45,lod,earth,base=-17,windows=True)
    # Lions court: white columns, warm carved stucco, blue/green dado mosaic,
    # two projecting pavilions and twelve individually modelled lion supports.
    lx,lz=-25,12
    m.box((lx,.12,lz),(28.5,.25,15.7),PALE,8)
    for x,z,w,d,h in [(lx-18,lz,7,23,9),(lx+18,lz,7,23,9),(lx,lz-13,33,9,11),(lx,lz+13,33,9,13)]:block(m,x,z,w,d,h,lod,plaster,peak=h+4,roof_color=tile,windows=False,material=MAT.PLASTER_WALL)
    column_positions=[]
    for z in [-7.85,7.85]:
        for x in np.linspace(-14,14,40):column_positions.append((lx+float(x),lz+z))
    for x in [-14.25,14.25]:
        for z in np.linspace(-7.5,7.5,18):column_positions.append((lx+x,lz+float(z)))
    for x,z in column_positions:
        lathe(m,x,z,[(0,.17),(3.2,.17),(3.35,.34),(3.85,.34)],PALE,(8,6,5)[lod],material=MAT.VEINED_MARBLE)
    for x,z,w,d in [(lx,lz-7.85,28.5,2),(lx,lz+7.85,28.5,2),(lx-14.25,lz,2,15.7),(lx+14.25,lz,2,15.7)]:
        m.box((x,4.2,z),(w,.8,d),plaster)
        roof(m,x,z,w,d,4.6,6.0,tile,hip=True)
    for side in [-1,1]:
        roof(m,lx+side*10.5,lz,7,7,5,8,tile,hip=True)
        for dx in [-2.5,2.5]:
            for dz in [-2.5,2.5]:lathe(m,lx+side*10.5+dx,lz+dz,[(0,.22),(5,.22)],PALE,8,material=MAT.VEINED_MARBLE)
    for i in range(12):
        a=i*math.tau/12;x,z=lx+1.55*math.cos(a),lz+1.55*math.sin(a)
        m.ellipsoid((x,.68,z),(.43,.42,.65),PALE,17,(10,8,6)[lod],4)
        m.ellipsoid((x,.92,z),(.32,.3,.3),PALE,17,8,4)
    lathe(m,lx,lz,[(1.05,1),(1.3,1.5),(1.65,1.7)],PALE,(32,24,16)[lod],material=MAT.VEINED_MARBLE)
    if lod<2:
        for x in np.arange(lx-14,lx+15,1):
            for z in [lz-8.75,lz+8.75]:m.box((float(x),.7,z),(.8,1.1,.08),[(57,116,120),(218,209,174),(55,82,71)][int(x)%3],17)
    # The Partal stands on a lower garden terrace, with its small Ladies Tower
    # at the mapped northern end and an open portico facing the pool.
    start=len(m.triangles)
    block(m,42,50,6.7,6,12,lod,earth,peak=16,roof_color=tile)
    block(m,39,43,14,9,7,lod,earth,peak=11,roof_color=tile,windows=False)
    m.box((39,.12,20),(22,.2,30),(76,121,111),3)
    for x in [31,35,39,43,47]:lathe(m,x,37,[(0,.25),(5,.25)],plaster,8)
    roof(m,39,37,22,6,6,9,tile)
    shift_component(m,start,y=terrain_height('alhambra',42,35)-terrain_data('alhambra')['ground_m'])
    # Retained201-vertex OSM historic boundary provides the actual southeast
    # circuit, including the Water Tower terminus. Its grade follows the DEM.
    surveyed_wall(m,'alhambra',plan_ring('alhambra',92528618),8,lod,earth)
    for ident,h in [(92528624,12),(92528634,18),(92528662,16),(92528680,13),
            (92528682,12),(92528687,17),(92528702,15),(92528709,22),
            (92528739,12),(92528760,12),(92528764,16),(92528800,17),
            (196485670,12),(196485671,12),(196485672,12),(283661651,10)]:
        x,z,_,_,_=rectangle_fit(plan_ring('alhambra',ident))
        grade=terrain_height('alhambra',x,z)-terrain_data('alhambra')['ground_m']
        plan_block(m,'alhambra',ident,h,None,lod,earth,grade-1,windows=False)
    # Santa Maria is mapped south of the Nasrid complex. Hotel/service ranges
    # farther east are retained from source data instead of invented infill.
    x,z,_,_,_=rectangle_fit(plan_ring('alhambra',117369758))
    grade=terrain_height('alhambra',x,z)-terrain_data('alhambra')['ground_m']
    plan_block(m,'alhambra',117369758,14,21,lod,earth,grade,tile)
    block(m,-20,-50,8,8,26,lod,earth,base=grade,peak=31,roof_color=tile,hip=True)
    return m


BUILDERS = {
    'neuschwanstein-castle': neuschwanstein, 'edinburgh-castle': edinburgh,
    'windsor-castle': windsor, 'conwy-castle': conwy, 'malbork-castle': malbork,
    'alcazar-of-segovia': segovia, 'alhambra': alhambra, 'eltz-castle': eltz,
    'chillon-castle': chillon, 'bran-castle': bran, 'predjama-castle': predjama,
    'matsumoto-castle': matsumoto, 'osaka-castle': osaka,
}


def build(name, lod):
    if lod not in (0,1,2):
        raise ValueError('expected LOD 0, 1, or 2')
    m = BUILDERS[name](lod)
    # Photographic proportions are reconciled with the production footprint
    # envelope; these are architectural xz calibrations, never unit conversion.
    scales = {'eltz-castle':(.76,1.20), 'bran-castle':(.88,.80)}
    if name in scales:
        sx,sz=scales[name]
        m.transform([[sx,0,0,0],[0,1,0,0],[0,0,sz,0],[0,0,0,1]])
    if name=='eltz-castle':
        eltz_spur(m,lod)
        eltz_outworks(m,lod)
    measured_supports(m,name,lod)
    return m


def _entry(name, city, source, offset, heading, description, dimensions, references, base='terrain'):
    import json
    from pathlib import Path
    source_file=Path(__file__).parent/'references'/'castles'/f'{source}-source.json'
    data=json.loads(source_file.read_text())
    FRAMES[source]=(offset,heading)
    east,north=offset
    lat=data['lat']+north/111195
    lon=data['lon']+east/(111195*math.cos(math.radians(data['lat'])))
    return dict(name=name,city=city,lat=lat,lon=lon,heading=heading,base=base,
        suppress_buildings=True,suppress_roads=False,suppress_trees=True,
        description=description,dimensions=dimensions,references=references,
        terrain_flatten=[],audit={'source_file':str(source_file.relative_to(Path(__file__).parents[2])),
            'source_anchor_latlon':[data['lat'],data['lon']],
            'source_offset_east_north_m':list(offset),
            'ground_center_m':data['ground_m'],
            'grid_min_m':min(p['ground_m'] for p in data['ground_samples']),
            'grid_max_m':max(p['ground_m'] for p in data['ground_samples']),
            'placement_note':'Source footprints are EN metric rings. Bearing and center are fitted to named ranges, with measurement qualifications in the site reference.'},
        capture={'distance':240,'azimuth':220,'elevation':28,'target_height':14,'time_utc':'2026-03-20T11:00:00Z'})


ENTRIES = [
    _entry('neuschwanstein-castle','Schwangau','neuschwanstein',(-29,-11),334,
        'Limestone Palas,65m northern stair spire, red gateway, Knights House, Bower and two open courts.',
        {'overall_length_m':150,'north_stair_tower_height_m':65,'square_tower_wall_height_m':35},
        ['https://www.neuschwanstein.de/englisch/palace/history.htm','https://www.neuschwanstein.de/bilder/schloss/grundriss790.png','https://en.wikipedia.org/wiki/Neuschwanstein_Castle']),
    _entry('edinburgh-castle','Edinburgh','edinburgh',(10,-36),0,
        'Stepped upper and lower wards, Crown Square, Royal Palace, Great Hall, chapel, barracks and Half-Moon Battery.',
        {'mapped_great_hall_roof_approx_m':[26.3,12.7],'chapel_footprint_extent_m':[9.8,5.7]},
        ['https://www.edinburghcastle.scot/media/fpanlfr2/orientation-map.pdf','https://www.edinburghcastle.scot/see-and-do/highlights/the-great-hall/']),
    _entry('windsor-castle','Windsor','windsor',(0,0),8,
        'Round Tower on its motte, Upper Ward quadrangle and State Apartments, Lower Ward and St Georges Chapel.',
        {'round_tower_diameter_m':32.4,'upper_ward_approx_m':[157,143]},
        ['https://www.rct.uk/visit/windsor-castle','https://www.rct.uk/visit/windsor-castle/highlights/castle-precincts'],52.4),
    _entry('conwy-castle','Conwy','conwy',(3,-1),352,
        'Eight roofless round towers, four eastern stair turrets, two barbicans and surviving Great Hall arches.',
        {'site_scale_m_per_plan_pixel':.1875,'tower_height_m':21,'tower_diameter_m':12.2},
        ['https://cadw.gov.wales/sites/default/files/2019-05/Conwyteachersnotes_EN.pdf','https://cadw.gov.wales/visit/places-to-visit/castell-conwy']),
    _entry('malbork-castle','Malbork','malbork',(-37,-38),35,
        'Red-brick High Castle with cloister, church and dansker, Middle Castle and Grand Masters Palace, Lower Castle armoury.',
        {'high_castle_outer_approx_m':[51,61],'bell_tower_approx_height_m':64,'authored_axis_length_m':529},
        ['https://zamek.malbork.pl/en/map/','https://zamek.malbork.pl/en/home/visit/history-of-the-castle/']),
    _entry('alcazar-of-segovia','Segovia','segovia',(-2,-2),26,
        'Tapered palace prow, two open courts, slate spires and Juan II tower with twelve bartizans and stone sgraffito.',
        {'palace_axis_approx_m':138,'juan_ii_above_court_approx_height_m':35},
        ['https://www.alcazardesegovia.com/wp-content/uploads/2025/06/la_fortaleza_del_alcazar.pdf','https://www.turismodesegovia.com/es/monumentos/edificios-militares/real-alcazar']),
    _entry('alhambra','Granada','alhambra',(0,0),0,
        'Alcazaba, Nasrid palace courts, Charles V square palace with circular court, Partal, medina and ridge ramparts.',
        {'charles_v_side_m':63,'charles_v_court_diameter_m':30,'myrtles_court_m':[23,36],
         'myrtles_pool_m':[7.1,34],'comares_external_height_m':45,'lions_columns':124,'lions_court_m':[28.5,15.7]},
        ['https://www.alhambra-patronato.es/elemento-del-mes/patio-de-los-arrayanes-the-courtyard-of-the-myrtles','https://www.alhambra-patronato.es/edificios-lugares/salon-de-los-embajadores','https://www.alhambra-patronato.es/wp-content/uploads/2020/04/Cuaderno-Erase-2020-web.pdf','https://www.alhambra-patronato.es/en/edificios-lugares/palace-of-charles-v']),
    _entry('eltz-castle','Wierschem','eltz',(-11,-3),9,
        'Eight compact residential towers with slate roofs, projecting oriels and colored timber frame houses around a narrow court.',
        {'maximum_tower_height_m':35,'residential_towers':8,'natural_rock_spur_height_m':60},
        ['https://burg-eltz.de/en/the-castle']),
    _entry('chillon-castle','Veytaux','chillon',(-2,0),346,
        'Oval lakeside castle, three courts, square keep, tiled lakeside residential ranges and eastern round defensive towers.',
        {'island_length_m':100,'island_width_m':50,'keep_roof_approx_height_m':25},
        ['https://www.chillon.ch/en/castle/','https://www.chillon.ch/le-nouvel-escalier-du-donjon/'],372),
    _entry('bran-castle','Bran','bran',(-8,-8),34,
        'Irregular white plaster wings, tiled square towers, round stone tower and timber galleries around an open court.',
        {'surveyed_core_extent_approx_m':[49,39],'highest_roof_approx_m':32},
        ['https://bran-castle.com/pages/bran-fortress','https://bran-castle.com/pages/visitor-map']),
    _entry('predjama-castle','Predjama','predjama',(-49,-4),0,
        'Renaissance cave-castle beneath a measured limestone cliff, recessed rock chamber, stone buttresses and eastern bridge approach.',
        {'surveyed_building_extent_m':[32.5,25.2],'natural_cliff_height_m':123},
        ['https://www.postojnska-jama.eu/en/predjama-castle/','https://www2.postojnska-jama.eu/assets/PDF-dokumenti/Jamski-dvorec/PJ-B2B-brossura-ANG-2016-mail.pdf'],511.5),
    _entry('matsumoto-castle','Matsumoto','matsumoto',(3,-4),0,
        'Five connected black-and-white structures, curved tiled roofs, stone batter and vermilion open moon-viewing pavilion.',
        {'tenshu_height_m':29.4,'inui_kotenshu_height_m':16.8,'watari_yagura_height_m':12,
         'tatsumi_tsuke_yagura_height_m':14.7,'tsukimi_yagura_height_m':11.1},
        ['https://www.matsumoto-castle.jp/img/info/pamphlet2018_en.pdf'],590.5),
    _entry('osaka-castle','Osaka','osaka',(0,5),6,
        'Present keep with massive concave stone batter, five green roof tiers, white walls, black and gold top frieze and gilded shachi.',
        {'honmaru_ground_geo_m':[31.2,31.5],'summit_geo_m':85.9,'authored_height_m':54.6},
        ['https://www.osakacastle.net/toyotomi_stone_wall/column/2401.html'],31.3),
]


def _convex(points):
    p=sorted(set(map(tuple,np.asarray(points,float))))
    if len(p)<3:return []
    def cross(a,b,c):return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    lo=[];hi=[]
    for a in p:
        while len(lo)>1 and cross(lo[-2],lo[-1],a)<=1e-7:lo.pop()
        lo.append(a)
    for a in reversed(p):
        while len(hi)>1 and cross(hi[-2],hi[-1],a)<=1e-7:hi.pop()
        hi.append(a)
    return [list(a) for a in lo[:-1]+hi[:-1]]


def _strip(a,b,width):
    a,b=np.asarray(a),np.asarray(b);v=b-a
    if np.linalg.norm(v)<.25:return []
    n=np.array([-v[1],v[0]])/np.linalg.norm(v)*width/2
    return _convex([a+n,b+n,b-n,a-n])


def _footprint_pieces(points):
    """Ear clipping preserves concave wings and narrow projecting bridges."""
    p=np.asarray(points,float)
    signed=lambda v:sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(v,np.roll(v,-1,axis=0)))
    hull=_convex(p)
    if abs(abs(signed(p))-abs(signed(hull)))<.05:return [hull]
    if signed(p)<0:p=p[::-1]
    indices=list(range(len(p)));out=[]
    cross=lambda a,b:a[0]*b[1]-a[1]*b[0]
    while len(indices)>3:
        found=False
        for j,i in enumerate(indices):
            before,after=indices[j-1],indices[(j+1)%len(indices)]
            a,b,c=p[before],p[i],p[after]
            if cross(b-a,c-b)<=1e-7:continue
            inside=lambda q:min(cross(b-a,q-a),cross(c-b,q-b),cross(a-c,q-c))>=-1e-7
            if any(inside(p[k]) for k in indices if k not in (before,i,after)):continue
            out.append([a.tolist(),b.tolist(),c.tolist()]);indices.pop(j);found=True;break
        if not found:
            candidates=[(abs(cross(p[indices[j]]-p[indices[j-1]],p[indices[(j+1)%len(indices)]]-p[indices[j]])),j) for j in range(len(indices))]
            value,j=min(candidates)
            if value>1e-4:raise ValueError('non-simple mapped castle footprint')
            indices.pop(j)
    if len(indices)==3:out.append([p[k].tolist() for k in indices])
    return out


# Source ownership follows replacement buildings and wall strips. Activation
# bounds explicitly include these polygons; gardens beyond them remain native.
_SOURCE_BUILDINGS={
    'neuschwanstein':[221599087,221599090,221599096,221599097,221599098,223311650],
    'edinburgh':[290611982,467503845,511343921,41294347,41299975,41299976,41299978,41299983,446654897,467503844,467503846,41299985,41299987,41299992],
    'windsor':[92729166,131174658,131174659,131174662,131174663,131174665,131174667,131175692,131179513,131188501,131188506,131189024,131228046,1162821758],
    'conwy':[52467063],
    'malbork':[62144153,122122611,261512837,62144160,62144157,261513898,101914542,261513528,101915237,101915918,62682251,122122605,261513127,261512960,985854910,122122609,122123493,122122626,432786364],
    'segovia':[47800753,1176897232],
    'alhambra':[26935518,23472864,279110939,117369758,196500564,92528624,92528634,92528662,92528680,92528682,92528687,92528702,92528709,92528739,92528760,92528764,92528800,196485670,196485671,196485672,283661651],
    'eltz':[238981197,238981200,238981202,238981208,604621402,604621396,604621397,604621401,238981207],
    'chillon':[134933836,987352931,987352932,987352933,987354384,987354386,987357289,987357290,987357291,987357292,987360351,987361889,987361890,987379767,987379768,987379769,987379770,987379771,987379772,987379773,987379776,987379777],
    'bran':[179351361],
    'predjama':[426772755], 'matsumoto':[300077872], 'osaka':[34619038],
}
_SOURCE_WALLS={
    'edinburgh':[(4301292,True,4)],'windsor':[(23580556,True,3.5)],
    'alhambra':[(92528618,True,4)],
    'malbork':[(i,False,3.5) for i in [62144151,106152188,122122606,122123490,122123498,122123499,433802824,698950336,1173702785,122123495]],
}
_PRIMARY_ADDITIONS={
    'edinburgh':['https://portal.historicenvironment.scot/apex/f?p=1505:300:::::VIEWTYPE,VIEWREF:designation,LB48220'],
    'alhambra':['https://www.granada.org/inet/wfotos.nsf/ae8ed1fa9c644732c125754000465473/ef095026ffa62ccbc1256e2f007bad68!OpenDocument'],
    'eltz':['https://www.visitmosel.de/wandern/tour/moselkern-zur-burg-eltz-und-zurueck'],
    'osaka':['https://www.osakacastle.net/90/history/'],
    'windsor':['https://www.royal.uk/sites/default/files/media/sovereign_grant_annual_report_2016-17.pdf','https://www.rct.uk/visit/windsor-castle/the-fire-at-windsor-castle'],
    'predjama':['https://gis.arso.gov.si/arcgis/rest/services/Slovenija_DMR_D96TM/ImageServer','https://www.postojnska-jama.eu/assets/Uploads/2024/Agencije/Handbook-ENG.pdf'],
}
for entry in ENTRIES:
    source=Path(entry['audit']['source_file']).stem.removesuffix('-source')
    entry['references']+=_PRIMARY_ADDITIONS.get(source,[])+[plan_data(source)['source']]
    support_path=REFERENCES/f"{entry['name']}-ground-supports.json"
    support_data=None
    if support_path.exists():
        support_data=json.loads(support_path.read_text())
        entry['references']+=support_data['references']
        entry['audit']['foundation_support_file']=str(support_path.relative_to(Path(__file__).parents[2]))
        entry['audit']['foundation_contact']='Terrain-sampled support toes embed0.5m; side faces overlap architectural feet4cm, preserving floor and roof datums. Rock bedding relief remains an explicitly estimated secondary detail.'
    if source=='osaka':entry['references'].append('https://www.osakacastle.net/pdf/access/bf_map_eng_a4.pdf')
    polygons=[p for i in _SOURCE_BUILDINGS[source] for p in _footprint_pieces(plan_ring(source,i))]
    if support_data:
        # Trees rooted in the former smoothed DEM must not emerge through the
        # occupied rock crest. Slopes beyond that contour remain native.
        polygons.extend(_convex(np.asarray([v['top_xz'] for v in s['vertices']])) for s in support_data['supports'] if s['kind']=='rock')
    for ident,closed,width in _SOURCE_WALLS.get(source,[]):
        p=plan_ring(source,ident)
        for a,b in zip(p,np.roll(p,-1,axis=0) if closed else p[1:]):polygons.append(_strip(a,b,width))
    polygons=[p for p in polygons if len(p)>=3]
    entry['source_exclusion_polygons']=polygons
    all_points=np.concatenate([np.asarray(p) for p in polygons])
    entry['activation_bounds_xz']=[all_points.min(0).tolist(),all_points.max(0).tolist()]
    survey=terrain_data(source)
    entry['audit']['terrain_survey_file']=str((REFERENCES/f'{source}-terrain.json').relative_to(Path(__file__).parents[2]))
    entry['audit']['calibrated_anchor_ground_m']=survey['ground_m']
    entry['audit']['plan_file']=str((REFERENCES/f'{source}-plan.json').relative_to(Path(__file__).parents[2]))
    entry['audit']['source_ownership']='Convex mapped building pieces and narrow mapped wall strips; activation bounds include their exact extent.'
    if source in ['osaka','matsumoto']:
        plaza=[[-58,-85],[68,-85],[75,38],[10,53],[-30,43],[-58,12]] if source=='osaka' else [[-30,-25],[29,-25],[30,30],[-30,35]]
        entry['terrain_cuts']=[{'polygon_xz':plaza,'plane':[0,0,-.05],'feather_m':25 if source=='osaka' else 15}]
        entry['audit']['terrain_cut_evidence']='Lower-only plaza cuts remove a spatially blurred DSM keep mound extending beyond the footprint. Osaka surveyed honmaru31.2–31.5m contrasts with production50–56m near the keep and39.8m65m west; Matsumoto surrounding ground590.5m contrasts with598–599m mound. Mapped retaining walls bound the core plaza; feathering transitions outside it and lower moat terrain is retained.'
    if source=='eltz':
        entry['base']=140.4
        entry['water_cuts']=json.loads((REFERENCES/'eltz-water-cuts.json').read_text())['cuts']
        cuts=[]
        for x in range(-192,192,16):
            for z in range(-192,192,16):
                ring=np.array([[x,z],[x+16,z],[x+16,z+16],[x,z+16]],float)
                for ids in [[0,1,2],[0,2,3]]:
                    p=ring[ids];h=np.array([eltz_ground(*v)-140.4-.4 for v in p])
                    cuts.append({'polygon_xz':p.tolist(),'plane':np.linalg.solve(np.c_[p,np.ones(3)],h).tolist(),'feather_m':2})
        entry['terrain_cuts']=cuts
        entry['references']+=['https://geoshop.rlp.de/opendata-dgm1.html','https://geobasis-rlp.de/data/dgm1/current/tif/dgm1_32_381_5562_1_rp_2024.tif']
        entry['audit']['terrain_cut_evidence']='Official LVermGeoRP2024 DGM1 in DHHN2016: courtyard140.389m, riverside110–112m, flank118–121m. Production131–135m blends away the spur. Authored measured relief and lower-only16m cuts restore it within384m, with64m outer transition. Foundation interpolation beneath mapped roofs is capped at courtyard datum.'
    if source=='predjama':
        # Sampled cells only lower the broad interpolated DEM toward the national
        # relief. The authored vertical rock/cave replaces the heightfield edge.
        cuts=[]
        def corrected_ground(v):
            blend=np.clip(min(192-abs(v[0]),v[1]+224,160-v[1])/96,0,1)
            blend=blend*blend*(3-2*blend)
            return blend*predjama_ground(*v)+(1-blend)*terrain_height('predjama-transition',*v)
        cells=[]
        for x in range(-192,192,32):
            for z in range(-224,160,32):
                if -64<=x<64 and -64<=z<64:
                    cells.extend((x+dx,z+dz,16) for dx in [0,16] for dz in [0,16])
                else:cells.append((x,z,32))
        for x,z,step in cells:
            ring=np.array([[x,z],[x+step,z],[x+step,z+step],[x,z+step]],float)
            for ids in [[0,1,2],[0,2,3]]:
                p=ring[ids];heights=np.array([corrected_ground(v)-511.5-.3 for v in p])
                plane=np.linalg.solve(np.c_[p,np.ones(3)],heights)
                cuts.append({'polygon_xz':p.tolist(),'plane':plane.tolist(),'feather_m':2})
        cave=[[-27,-9],[-3,-23],[23,24],[-1,38]]
        cuts.append({'polygon_xz':cave,'plane':[0,0,-.3],'feather_m':3})
        entry['terrain_cuts']=cuts
        line=np.asarray(predjama_relief()['cliff_polyline_xz_m'])[1:-1]
        entry['source_exclusion_polygons'] += [_strip(a,b,8) for a,b in zip(line,line[1:])]
        entry['source_exclusion_polygons'].append([[-20,-18],[20,-18],[20,15],[-20,15]])
        all_points=np.concatenate([np.asarray(p) for p in entry['source_exclusion_polygons']])
        entry['activation_bounds_xz']=[all_points.min(0).tolist(),all_points.max(0).tolist()]
        entry['audit']['terrain_cut_evidence']='ARSO national1m relief entrance511.462m and local cliff crest586–593m supersede production565.628m smoothed anchor. Triangular lower-only cuts use16m cells at the castle and32m cells in the valley. The384m envelope blends to the measured production terrain over its outer96m, avoiding a steep artificial pit edge. A small lower-only cave floor allows the authored overhang.'
        entry['capture'].update(distance=240,azimuth=140,elevation=22,target_height=27)
