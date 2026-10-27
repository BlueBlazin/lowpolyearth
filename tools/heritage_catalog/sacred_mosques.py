"""Seven distinct mosque plans in surveyed east/up/north metric coordinates.

Primary dimensions override provider height tags; architectural interpretation
and contradictory evidence are recorded beside the retained ODbL footprints.
"""
import json
import math
from functools import lru_cache
from pathlib import Path

import numpy as np
from heritage_catalog.materials import WEATHERED_LIMESTONE, VEINED_MARBLE, EARTHEN_PLASTER
from heritage_catalog.geometry import (Mesh, arch, arch_bay, column, dome, face,
    lathe, plan_solid, plan_triangles, ring, stairs, tube, turn, window)

REF=Path(__file__).with_name('references')/'sacred_mosques'
WHITE=(234,230,214); CREAM=(211,197,166); STONE=(189,183,168)
LEAD=(108,117,118); SHADOW=(41,48,47); GOLD=(198,163,72)
BLUE=(34,78,125); TURQUOISE=(41,137,151); GREEN=(37,100,79)


@lru_cache(None)
def survey(name):return json.loads((REF/(name+'-survey.json')).read_text())


def part(name,ident):return next(b for b in survey(name)['buildings'] if b['id']==ident)


def poly(name,ident,ring_index=0):return np.asarray(part(name,ident)['rings'][ring_index],float)


def center(name,ident):
    p=poly(name,ident);return (p.min(0)+p.max(0))/2


def measured_box(name,ident):
    p=poly(name,ident);edges=np.roll(p,-1,axis=0)-p
    axis=edges[np.argmax(np.linalg.norm(edges,axis=1))];axis/=np.linalg.norm(axis)
    normal=np.array([-axis[1],axis[0]]);q=np.column_stack([p@axis,p@normal]);lo=q.min(0);hi=q.max(0)
    c=axis*(lo[0]+hi[0])/2+normal*(lo[1]+hi[1])/2
    return c,hi[0]-lo[0],hi[1]-lo[1],math.degrees(math.atan2(-axis[1],axis[0]))


def boundary(points,interior=False):
    """Facade detail faces the empty side: exterior right, courtyard left."""
    p=np.asarray(points,float)
    area=sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))
    return p[::-1] if (area>0)==interior else p


def ranges(m,rings,y,color,bottom=None,material=82):
    """Horizontal even/odd roof tessellation keeps every source court open."""
    pp=[np.asarray(p,float) for p in rings]
    zz=sorted(set(float(v[1]) for p in pp for v in p))
    edges=[(a,b) for p in pp for a,b in zip(p,np.roll(p,-1,axis=0)) if abs(a[1]-b[1])>1e-7]
    for z0,z1 in zip(zz,zz[1:]):
        if z1-z0<1e-6:continue
        zm=(z0+z1)/2
        crossing=sorted([(a[0]+(zm-a[1])*(b[0]-a[0])/(b[1]-a[1]),a,b) for a,b in edges if min(a[1],b[1])<zm<max(a[1],b[1])],key=lambda v:v[0])
        for left,right in zip(crossing[::2],crossing[1::2]):
            at=lambda edge,z:edge[1][0]+(z-edge[1][1])*(edge[2][0]-edge[1][0])/(edge[2][1]-edge[1][1])
            p=[(at(left,z0),y,z0),(at(right,z0),y,z0),(at(right,z1),y,z1),(at(left,z1),y,z1)]
            # A strip can taper to a single vertex; orient each nonzero half.
            face(m,[p[0],p[1],p[2]],color,material)
            face(m,[p[0],p[2],p[3]],color,material)
    if bottom is not None:
        for index,p in enumerate(pp):
            area=sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(p,np.roll(p,-1,axis=0)))
            for a,b in zip(p,np.roll(p,-1,axis=0)):
                e=b-a
                if np.linalg.norm(e)<1e-6:continue
                normal=(e[1]*np.sign(area)*(1 if index==0 else -1),0,-e[0]*np.sign(area)*(1 if index==0 else -1))
                if callable(bottom):
                    steps=max(1,math.ceil(np.linalg.norm(e)/6))
                    for j in range(steps):
                        aa=a+e*j/steps;bb=a+e*(j+1)/steps
                        face(m,[(aa[0],bottom(*aa),aa[1]),(bb[0],bottom(*bb),bb[1]),(bb[0],y,bb[1]),(aa[0],y,aa[1])],color,material,normal)
                else:
                    face(m,[(a[0],bottom,a[1]),(b[0],bottom,b[1]),(b[0],y,b[1]),(a[0],y,a[1])],color,material,normal)


def masonry(m,a,b,bottom,top,color,lod,material=82):
    """Coursed stone varies by block and ages toward the base; no random speckle."""
    a,b=np.asarray(a,float),np.asarray(b,float);v=b-a;length=np.linalg.norm(v)
    if length<2 or top-bottom<1:return
    v/=length;out=np.array([v[1],-v[0]])
    unit=(1.8,2.9,5.8)[lod] if material==82 else (.45,.9,2.5)[lod]
    course=(.8,1.4,2.8)[lod] if material==82 else (.22,.55,1.5)[lod]
    for row,y in enumerate(np.arange(bottom,top,course)):
        y1=min(top,y+course);offset=unit*.5*(row%2)
        for col,u in enumerate(np.arange(-offset,length,unit)):
            u0=max(0,u);u1=min(length,u+unit)
            if u1-u0<.05:continue
            shade=(-3,1,4,-1,2)[(col+row*2)%5]-int(4*max(0,1-(y-bottom)/3))
            c=tuple(max(0,min(255,v+shade)) for v in color)
            p=a+v*(u0+.007)+out*.012;q=a+v*(u1-.007)+out*.012
            face(m,[(p[0],y+.007,p[1]),(q[0],y+.007,q[1]),(q[0],y1-.007,q[1]),(p[0],y1-.007,p[1])],c,material,(out[0],0,out[1]))


def shell(m,name,ident,h,color,bottom=-1,material=82,lod=1):
    rings=part(name,ident)['rings'];ranges(m,rings,h,color,bottom,material)
    if material in (81,82):
        for index,p in enumerate(rings):
            p=boundary(p,interior=index>0)
            for a,b in zip(p,np.roll(p,-1,axis=0)):masonry(m,a,b,max(0,bottom),h,color,lod,material)



def finial(m,x,y,z,h,lod,crescent=True):
    lathe(m,(x,z),[(y,.13*h),(y+.2*h,.13*h),(y+.75*h,.035*h),(y+h,.01)],GOLD,95,(12,8,6)[lod])
    for level,r in [(.25,.15),(.47,.1)]:m.ellipsoid((x,y+level*h,z),(r*h,)*3,GOLD,95,(12,8,6)[lod],(6,4,3)[lod])
    if crescent:
        r=.22*h
        for a,b in zip(np.linspace(.3,math.tau-.3,(16,10,6)[lod]),np.linspace(.3,math.tau-.3,(16,10,6)[lod])[1:]):
            tube(m,(x+r*math.sin(a),y+h-r+r*math.cos(a),z),(x+r*math.sin(b),y+h-r+r*math.cos(b),z),.035*h,GOLD,95,5)


def cap(m,x,z,r,y,h,lod,color=LEAD,onion=False,half_heading=None,pattern=False,material=88):
    n=((32,20,12) if r<7 else (64,40,24))[lod];steps=((10,7,5) if r<7 else (16,10,7))[lod]
    profile=[]
    for j in range(steps+1):
        t=j/steps
        radius=r*(math.cos(t*math.pi/2)*(1+.25*math.sin(math.pi*t)) if onion else math.cos(t*math.pi/2))
        profile.append((y+h*t,max(.02,radius)))
    shade=lambda k,j:tuple(max(0,min(255,c+(-2,0,1,2)[(j//3+k)%4])) for c in color)
    if half_heading is None:
        lathe(m,(x,z),profile,shade,material,n)
    else:
        a0=math.radians(half_heading)-math.pi/2
        for k,((ya,ra),(yb,rb)) in enumerate(zip(profile,profile[1:])):
            for j in range(n//2):
                a=a0+j*math.pi/(n//2);b=a0+(j+1)*math.pi/(n//2)
                face(m,[(x+ra*math.cos(a),ya,z+ra*math.sin(a)),(x+ra*math.cos(b),ya,z+ra*math.sin(b)),(x+rb*math.cos(b),yb,z+rb*math.sin(b)),(x+rb*math.cos(a),yb,z+rb*math.sin(a))],shade(k,j),material,(math.cos((a+b)/2),.5,math.sin((a+b)/2)))
    if pattern:
        count=(48,32,16)[lod]
        for k in range(count):
            for direction in [-1,1]:
                path=[]
                for j,(yy,rr) in enumerate(profile):
                    theta=k*math.tau/count+direction*j/steps*1.4
                    path.append((x+(rr+.035)*math.cos(theta),yy+.04,z+(rr+.035)*math.sin(theta)))
                for a,b in zip(path,path[1:]):tube(m,a,b,.045,GOLD,85,4)
    if lod<2 and (r>=7 or lod==0):
        count=24 if r>10 else 6
        for i in range(count):
            a=i*math.tau/count
            if half_heading is not None and math.cos(a-math.radians(half_heading))<-.01:continue
            for (ya,ra),(yb,rb) in zip(profile,profile[1:]):
                tube(m,(x+(ra+.045)*math.cos(a),ya+.03,z+(ra+.045)*math.sin(a)),(x+(rb+.045)*math.cos(a),yb+.03,z+(rb+.045)*math.sin(a)),.07 if r<9 else .11,color,material,4)


def drum(m,x,z,r,y,h,lod,color=CREAM,windows=24,material=82):
    lathe(m,(x,z),[(y,r),(y+h,r),(y+h+.35,r+.3)],color,material,(48,32,20)[lod])
    for i in range(windows):
        a=i*math.tau/windows;q=Mesh()
        window(q,0,0,-r-.04,min(1.2,r*.12),h*.70,min(2,lod+1) if r<7 else lod,WHITE,False,False)
        m.merge(turn(q,math.degrees(a),x,y+.3,z))


def ottoman_minaret(m,x,z,h,lod,balconies=1,color=CREAM,radius=1.6,material=82):
    lathe(m,(x,z),[(-1,radius*1.7),(7,radius*1.7),(10,radius*1.05),(h-13,radius*.78)],color,material,(20,16,10)[lod])
    ys=np.linspace(h*.52,h-15,balconies) if balconies>1 else [h-15]
    for y in ys:
        lathe(m,(x,z),[(y-1.6,radius),(y-.35,radius*1.85),(y,radius*1.85),(y+1.5,radius*1.65)],WHITE,17,(32,20,12)[lod])
        for a in np.linspace(0,math.tau,(24,16,10)[lod],endpoint=False):
            m.box((x+radius*1.65*math.cos(a),y+.75,z+radius*1.65*math.sin(a)),(.18,1.5,.18),color,17)
        lathe(m,(x,z),[(y+.1,radius*1.7),(y+.26,radius*1.7)],WHITE,17,(32,20,12)[lod])
    lathe(m,(x,z),[(h-13,radius*.88),(h-11,radius*.9),(h-2.2,.04)],LEAD,88,(24,16,10)[lod])
    finial(m,x,h-2.2,z,1.7,lod)
    if lod<2:
        for a in np.linspace(0,math.tau,16,endpoint=False):
            tube(m,(x+radius*1.025*math.cos(a),11,z+radius*1.025*math.sin(a)),(x+radius*.80*math.cos(a),h-14,z+radius*.80*math.sin(a)),.045,WHITE,17,4)


def parapet(m,a,b,y,h,color,lod,merlons=True):
    a,b=np.asarray(a,float),np.asarray(b,float);v=b-a;length=np.linalg.norm(v)
    if length<.1:return
    q=Mesh();q.box((length/2,y+h/2,0),(length,h,.65),color)
    if merlons:
        for x in np.arange(.5,length,1.65):q.box((x,y+h+.35,0),(.75,.7,.7),color)
    m.merge(turn(q,math.degrees(math.atan2(-v[1],v[0])),a[0],0,a[1]))


def facade(m,a,b,y,h,lod,color=CREAM,bay=5,levels=2,pointed=True,material=82):
    a,b=np.asarray(a,float),np.asarray(b,float);v=b-a;length=np.linalg.norm(v)
    if length<bay:return
    if material in (81,82):masonry(m,a,b,y,y+h,color,lod,material)
    q=Mesh()
    for xx in np.arange(bay/2,length,bay):
        for row in range(levels):window(q,xx,y+1+row*h/levels,-.08,min(2.6,bay*.48),min(4.4,h/levels-1.5),lod,color,pointed,True)
    m.merge(turn(q,math.degrees(math.atan2(-v[1],v[0])),a[0],0,a[1]))


def arcades(m,a,b,y,height,depth,lod,color=WHITE,bay=5,domes=False):
    a,b=np.asarray(a,float),np.asarray(b,float);v=b-a;length=np.linalg.norm(v);count=max(1,round(length/bay));width=length/count
    q=Mesh()
    for i in range(count):
        x=(i+.5)*width
        arch_bay(q,(x,y,0),width,height,depth,.45,color,lod,True)
        if domes:cap(q,x,0,width*.47,y+height+.3,width*.4,lod,LEAD)
    m.merge(turn(q,math.degrees(math.atan2(-v[1],v[0])),a[0],0,a[1]))


def iwan(m,x,z,width,height,depth,lod,heading=0,tiled=True):
    q=Mesh();color=BLUE if tiled else CREAM
    arch_bay(q,(0,0,0),width,height,depth,width*.18,color,lod,True)
    # The deep open vault terminates in a recessed door; the intrados is visible.
    arch(q,(0,.2,depth/2-.05),width*.59,height*.82,.3,TURQUOISE if tiled else WHITE,True,(24,16,8)[lod],SHADOW,17)
    for side in (-1,1):
        q.box((side*width*.42,height/2,-depth/2-.08),(width*.1,height,.16),TURQUOISE if tiled else WHITE,17)
    for yy in [1,height-1.1]:q.box((0,yy,-depth/2-.13),(width,.55,.18),WHITE,17)
    if tiled and lod<2:
        for side in [-1,1]:
            for yy in np.arange(2,height-2,1.55):
                xx=side*width*.42
                face(q,[(xx,yy+.55,-depth/2-.18),(xx+.45,yy,-depth/2-.18),(xx,yy-.55,-depth/2-.18),(xx-.45,yy,-depth/2-.18)],GOLD,17,(0,0,-1))
        for row in range(4):
            for xx in np.linspace(-width*.28,width*.28,7-row):
                yy=height*(.72+.055*row)
                lathe(q,(xx,depth*.2-row*.3),[(yy,.6),(yy+.8,.05)],WHITE,17,6)
    q.triangles=[(p,c,85 if mat==11 and tiled else mat) for p,c,mat in q.triangles]
    m.merge(turn(q,heading,x,0,z))


def hagia_minaret_footprints():
    # The authored circular shafts are wider than the tiny source building rings.
    return [[(float(c[0]+r*1.72*math.cos(a)),float(c[1]+r*1.72*math.sin(a)))
             for a in np.linspace(0,math.tau,32,endpoint=False)]
            for ident,r in [(1411,2.0),(1410,2.0),(1409,1.6),(1408,1.4)]
            for c in [center('hagia-sophia',ident)]]


def hassan_hall_footprint():
    c,_,_,a=measured_box('hassan-ii-mosque',43)
    angle=math.radians(a);u=np.array([math.cos(angle),-math.sin(angle)]);v=np.array([-u[1],u[0]])
    return [list(c+u*x+v*z) for x,z in [(-100.9,-51.2),(100.9,-51.2),(100.9,51.2),(-100.9,51.2)]]


def _hagia(lod):
    m=Mesh();name='hagia-sophia';pink=(181,132,111);brick=(163,102,80)
    # Local tomb levels remain independent of the church's west-entry datum.
    local=json.loads((REF/'mosque-floor-calibrations.json').read_text())[name]['component_floor_m']
    for ident in _FOOTPRINTS[name]:
        if str(ident) not in local:
            ranges(m,[poly(name,ident)],-.03,CREAM,bottom=-4,material=WEATHERED_LIMESTONE)
    # Irregular buttress apron, high nave and four great arches retain mapped plans.
    shell(m,name,1415,22,pink,material=83,lod=lod);shell(m,name,1412,26.5,pink,material=83,lod=lod);shell(m,name,1416,40,CREAM,material=WEATHERED_LIMESTONE,lod=lod)
    for ident in [1404,1405,1406,1407,1422,1423,1427,1428,1429,1431,1432,1433,1434,1435,1436,1437,1438,1439]:
        b=part(name,ident);h=min(28,b['height']);shell(m,name,ident,h,pink,material=83,lod=lod)
        p=boundary(poly(name,ident))
        for a,bb in zip(p,np.roll(p,-1,axis=0)):facade(m,a,bb,0,h,lod,CREAM,5.5,max(1,int(h/9)),False)
    # Main rendered apron's plaster face needs its exposed window ranks;
    # provider rings are clockwise, so reverse before outward-facing details.
    for ident,y,h in [(1415,0,22),(1412,22,4.5)]:
        p=poly(name,ident)[::-1]
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            facade(m,a,b,y,h,lod,CREAM,6.0,2 if ident==1415 else 1,False,83)
    c=center(name,1426);drum(m,*c,16.1,40,3,lod,CREAM,40);cap(m,*c,16.1,43,13.0,lod,LEAD);finial(m,c[0],56,c[1],2,lod)
    for ident in [1424,1425]:
        cc=center(name,ident);out=cc-c;angle=math.degrees(math.atan2(out[1],out[0]))
        cap(m,*cc,16.4,26,13,lod,LEAD,half_heading=angle)
    for ident in [1418,1419,1441,1442]:
        cc=center(name,ident);out=cc-c
        cap(m,*cc,6.9,21,8.2,lod,LEAD,half_heading=math.degrees(math.atan2(out[1],out[0])))
    for side in [-1,1]:
        q=Mesh()
        face(q,[(side*22,24,-32),(side*22,24,32),(side*37.5,20,32),(side*37.5,20,-32)],LEAD,88,(side,1,0))
        for z in np.arange(-30,32,2.4):tube(q,(side*22,24.03,z),(side*37.5,20.03,z),.08,LEAD,88,4)
        m.merge(turn(q,123.3,c[0],0,c[1]))
    # Tall lateral tympana have nested window ranks between great buttresses.
    tc,tw,td,ta=measured_box(name,1416)
    for heading in [ta+90,ta+270]:
        q=Mesh()
        for row,count in [(0,7),(1,5),(2,3)]:
            for x in np.linspace(-(count-1)*2.3,(count-1)*2.3,count):window(q,x,26+row*4.0,-tw/2-.25,2.0,3.3,lod,CREAM,False,True)
        m.merge(turn(q,heading,tc[0],0,tc[1]))
    for ident,h,r,col in [(1411,66,2.0,CREAM),(1410,66,2.0,CREAM),(1409,62,1.6,brick),(1408,64,1.4,CREAM)]:
        cc=center(name,ident);material=81 if ident==1409 else WEATHERED_LIMESTONE
        ottoman_minaret(m,*cc,h,lod,1,col,r,material)
        lathe(m,tuple(cc),[(-4,r*1.7),(-1,r*1.7)],col,material,(20,16,10)[lod])
    # Southern Ottoman tombs and baptistery differ in size and roof height.
    for ident,body,h in [(1443,14,9),(1444,13,8),(1445,14,8),(1430,7,5),(1452,7,5)]:
        cc=center(name,ident);r=min(np.ptp(poly(name,ident),axis=0))/2
        y=local[str(ident)]-floor_datum(name)
        lathe(m,tuple(cc),[(y-1.5,r),(y+body,r)],CREAM,11,8 if ident in [1430,1452] else 16)
        for k in range(8):
            q=Mesh();window(q,0,2,-r-.10,2.0,3.6,lod,CREAM,False,True)
            if body>10:window(q,0,8,-r-.10,1.6,2.9,lod,CREAM,False,True)
            m.merge(turn(q,k*45,cc[0],y,cc[1]))
        cap(m,*cc,r,y+body,h,lod,LEAD);finial(m,cc[0],y+body+h,cc[1],1.2,lod)
    return m


def _blue(lod):
    m=Mesh();name='blue-mosque';stone=(203,197,178)
    shell(m,name,1496,14.5,stone,lod=lod)
    # Surveyed central square and four half-domes; primary mosque dimensions
    # reject conflicting provider tags that would lift the main dome to 66m.
    p=boundary(poly(name,1496));c=center(name,188)
    q=Mesh();q.box((0,23,0),(24,13,24),stone);m.merge(turn(q,46.6,c[0],0,c[1]))
    drum(m,*c,11.75,29.5,2.5,lod,stone,28);cap(m,*c,11.75,32,11,lod,LEAD);finial(m,c[0],43,c[1],2,lod)
    angle=46.6;axis=np.array([math.cos(math.radians(angle)),-math.sin(math.radians(angle))]);normal=np.array([-axis[1],axis[0]])
    for v in [axis,-axis,normal,-normal]:
        cc=c+v*11.9
        angle_out=math.degrees(math.atan2(v[1],v[0]))
        semi=[cc]+[cc+11.75*np.array([math.cos(t),math.sin(t)]) for t in np.linspace(math.radians(angle_out)-math.pi/2,math.radians(angle_out)+math.pi/2,25)]
        plan_solid(m,semi,14.5,22,stone)
        cap(m,*cc,11.75,22,11,lod,LEAD,half_heading=angle_out)
        for side in [-1,1]:
            pp=cc+v*5.0+np.array([-v[1],v[0]])*side*7.5
            cap(m,*pp,5.3,17,5.7,lod,LEAD,half_heading=math.degrees(math.atan2(v[1],v[0])))
    for a,b in zip(p,np.roll(p,-1,axis=0)):facade(m,a,b,0,14.5,lod,WHITE,5.3,2,False)
    # Roofless sahn surrounded by 26 small domes and real colonnade apertures.
    court=poly(name,1503,1);ranges(m,[court],.05,(218,214,200))
    for a,b in zip(court,np.roll(court,-1,axis=0)):arcades(m,a,b,0,9,5.2,lod,stone,6.1,True)
    for ident,h,count in [(175,64,3),(178,64,3),(180,64,3),(182,64,3),(184,54,2),(186,54,2)]:
        cc=center(name,ident);ottoman_minaret(m,*cc,h,lod,count,stone,1.5)
    cc=np.mean(court,axis=0)
    lathe(m,tuple(cc),[(0,3.5),(.8,3.5),(1,2.9),(3.2,2.9)],WHITE,11,8)
    for k in range(8):
        a=k*math.tau/8;column(m,cc[0]+2.9*math.cos(a),cc[1]+2.9*math.sin(a),3.2,3.4,.19,WHITE,lod)
    cap(m,*cc,3.8,6.5,2.4,lod,LEAD)
    return m


def gulf_minaret(m,x,z,h,lod,r=3.6):
    m.box((x,9,z),(r*2,18,r*2),WHITE)
    for y,rr in [(17,r*1.2),(34,r*.92),(h*.7,r*.8)]:
        lathe(m,(x,z),[(y-.8,rr*.8),(y,rr*1.28),(y+.5,rr*1.28),(y+2,rr*1.13)],WHITE,17,(32,20,12)[lod])
    lathe(m,(x,z),[(18,r),(34,r*.8),(35,r*.74),(h*.70,r*.63),(h*.72,r*.6),(h-12,r*.5)],WHITE,11,(16,12,8)[lod])
    drum(m,x,z,r*.55,h-12,4,lod,WHITE,8)
    cap(m,x,z,r*.65,h-8,5.7,lod,WHITE,True,material=17);finial(m,x,h-2.3,z,1.8,lod)
    for heading in [0,90,180,270]:
        q=Mesh();window(q,0,5,-r-.05,2.1,8,lod,WHITE,True)
        m.merge(turn(q,heading,x,0,z))


@lru_cache(None)
def zayed_platform():
    return json.loads((REF/'zayed-platform-plan.json').read_text())['ways']


def floor_datum(name):
    if name=='hassan-ii-mosque':return 9.50
    if name in ['hagia-sophia','shah-mosque','sultan-qaboos-grand-mosque']:
        return json.loads((REF/'mosque-floor-calibrations.json').read_text())[name]['floor_m']
    if name=='sheikh-zayed-grand-mosque':
        return json.loads((REF/'zayed-platform-ground.json').read_text())['floor_datum_m']
    return survey(name)['ground_m']


def _clip_axis(p,axis,value,greater):
    result=[]
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        da=float(a@axis-value);db=float(b@axis-value)
        if (da>=0)==greater:result.append(a)
        if (da>=0)!=(db>=0):result.append(a+(b-a)*da/(da-db))
    return np.asarray(result)


@lru_cache(None)
def zayed_toe_samples():
    data=json.loads((REF/'zayed-approach-toes.json').read_text())
    return {(round(p['east_m'],5),round(p['north_m'],5)):p['ground_m'] for p in data['ground_samples']}


def zayed_toe(x,z):
    key=(round(float(x),5),round(float(z),5))
    return min(-12,zayed_toe_samples()[key]-floor_datum('sheikh-zayed-grand-mosque')-.5)


def zayed_approach(lod=1):
    """Mapped eastern raised terrace and 52-step approach, nine-metre rise."""
    p=np.asarray(next(w['xz']for w in zayed_platform()if w['id']==391867933)[:-1])
    step=np.asarray(next(w['xz']for w in zayed_platform()if w['id']==167776873))
    a,b=step[1],step[0];axis=(b-a)/np.linalg.norm(b-a);start=a@axis;end=b@axis
    # The pedestrian way stops at the gallery threshold; bridge its mapped seam.
    p[6:9]-=axis*8.0
    limits=[min(p@axis)-1,*np.linspace(start,end,53),max(p@axis)+1]
    m=Mesh()
    for lo,hi in zip(limits,limits[1:]):
        q=_clip_axis(_clip_axis(p,axis,lo,True),axis,hi,False)
        if len(q)<3:continue
        y=-9*min(1,max(0,((lo+hi)/2-start)/(end-start)))
        ranges(m,[q],y,WHITE,bottom=zayed_toe,material=VEINED_MARBLE)
    # Pool-side raised walk, bounded by the mapped basin and eastern approach.
    walk=[[131.7,3.5],[155.5,8.5],[143.5,60.8],[120.1,55.6]]
    ranges(m,[walk],-.03,WHITE,bottom=zayed_toe,material=VEINED_MARBLE)
    return m


def zayed_perimeter_cuts():
    """Grade contaminated podium margins toward sampled exterior ground."""
    data=json.loads((REF/'zayed-perimeter-plan.json').read_text())
    u=np.asarray(data['rotation_basis']);u/=np.linalg.norm(u);v=np.array([-u[1],u[0]])
    transform=np.stack([u,v]);base=floor_datum('sheikh-zayed-grand-mosque')
    def corners(bounds):
        left,bottom,right,top=bounds
        return np.asarray([[left,bottom],[right,bottom],[right,top],[left,top]])@transform
    inner=corners(data['inner_bounds_uv']);outer=corners(data['outer_bounds_uv'])
    cuts=[{'polygon_xz':inner.tolist(),'plane':[0,0,-.15],'feather_m':1.0}]
    for i in range(4):
        j=(i+1)%4;p=np.asarray([inner[i],inner[j],outer[j],outer[i]])
        heights=[-.15,-.15,data['outer_cap_absolute_m']-base,data['outer_cap_absolute_m']-base]
        plane=np.linalg.solve(np.column_stack([p[:3],np.ones(3)]),heights[:3])
        cuts.append({'polygon_xz':p.tolist(),'plane':plane.tolist(),'feather_m':12.0})
    return cuts


def shah_qaboos_grades(name):
    """Keep sampled entrance aprons independent of the architectural floor datum."""
    data=json.loads((REF/'shah-qaboos-setting-ground.json').read_text())
    return data.get(name,{}).get('terrain_grades',[])


def mosque_setting_grades(name):
    """Repair only documented entrance aprons without moving architectural floors."""
    data=json.loads((REF/'setting-ground-calibrations.json').read_text())
    return data.get(name,{}).get('terrain_grades',[])


def mosque_cuts(name,owned):
    cuts=[{'polygon_xz':p,'plane':[0,0,-.15],'feather_m':1.0}for p in owned]
    if name=='hagia-sophia':
        local=json.loads((REF/'mosque-floor-calibrations.json').read_text())[name]['component_floor_m']
        cuts=[]
        for ident in _FOOTPRINTS[name]:
            q=Mesh();ranges(q,part(name,ident)['rings'],0,WHITE)
            level=local.get(str(ident),floor_datum(name))-floor_datum(name)-.15
            cuts.extend({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,level],'feather_m':1.0} for p,_,_ in q.triangles)
        cuts.extend({'polygon_xz':p,'plane':[0,0,-.15],'feather_m':1.0} for p in hagia_minaret_footprints())
    if name=='sultan-qaboos-grand-mosque':
        for p,_,_ in qaboos_podium().triangles:
            if np.cross(p[1]-p[0],p[2]-p[0])[1]>1e-8:
                cuts.append({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,float(p[0,1])-.15],'feather_m':1.0})
    if name=='sheikh-zayed-grand-mosque':
        # Cut the mapped approach to its individual treads, not one floor plane.
        cuts=[{'polygon_xz':p,'plane':[0,0,-.15],'feather_m':8.0}for p in ownership(name,approach=False)]
        for p,_,_ in zayed_approach().triangles:
            if np.cross(p[1]-p[0],p[2]-p[0])[1]>1e-8:
                cuts.append({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,float(p[0,1])-.15],'feather_m':6.0})
        cuts.extend(zayed_perimeter_cuts())
        for w in zayed_platform():
            if w['tags'].get('natural')=='water' and w['id']not in [391857749,391857750]:
                q=Mesh();ranges(q,[w['xz'][:-1]],0,WHITE)
                cuts.extend({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,-1.85],'feather_m':6.0}for p,_,_ in q.triangles)
    return cuts


def mosque_water_cuts(name):
    if name!='sheikh-zayed-grand-mosque':return []
    cuts=[]
    for w in zayed_platform():
        if w['tags'].get('natural')=='water' and w['id']not in [391857749,391857750]:
            q=Mesh();ranges(q,[w['xz'][:-1]],0,WHITE)
            cuts.extend({'polygon_xz':p[:,[0,2]].tolist(),'plane':[0,0,-.65],'feather_m':.3}for p,_,_ in q.triangles)
    return cuts


def _zayed(lod):
    m=Mesh();name='sheikh-zayed-grand-mosque';rs=part(name,359)['rings'];outer=np.asarray(rs[0]);court=np.asarray(rs[1])
    # The raised marble platform reaches low source ground below the arcades.
    ranges(m,[outer],-.03,WHITE,bottom=-12,material=VEINED_MARBLE)
    m.merge(zayed_approach(lod))
    # Measured prayer-hall range and pierced courtyard galleries are separate.
    ranges(m,[outer,court],17.0,WHITE,material=17)
    cc=np.mean(court,axis=0)
    for a,b in zip(court,np.roll(court,-1,axis=0)):
        if np.linalg.norm(b-a)>15:arcades(m,a,b,0,16.7,6,lod,WHITE,7.8)
    for a,b in zip(outer,np.roll(outer,-1,axis=0)):
        if np.linalg.norm(b-a)>20:arcades(m,a,b,0,16.7,2.4,lod,WHITE,7.8)
    for boundary in [court,outer]:
        for a,b in zip(boundary,np.roll(boundary,-1,axis=0)):
            v=b-a;length=np.linalg.norm(v)
            if length<20:continue
            count=max(1,round(length/7.8));unit=v/length;normal=np.array([-unit[1],unit[0]])
            for t in np.linspace(0,1,count+1):
                c=a+v*t
                for dx in ([0] if lod==2 else [-.42,.42]):
                    for dz in ([0] if lod==2 else [-.42,.42]):
                        pp=c+unit*dx+normal*dz
                        lathe(m,tuple(pp),[(0,.35),(.5,.35),(.7,.25),(14.9,.23),(15.2,.38)],WHITE,17,(12,8,6)[lod])
                        lathe(m,tuple(pp),[(15.2,.38),(15.5,.50),(15.9,.34)],GOLD,95,(12,8,6)[lod])
    # Raised main musalla under the three dominant domes lies west of the sahn.
    main=[[-127.3,11.9],[-89.5,-157.3],[-13,-140],[-51,30]]
    plan_solid(m,main,-1,18,WHITE)
    for ident,r,y,h,fh in [(1,16.3,49,30,5),(0,12.9,40,21,4),(3,12.9,40,21,4),(2,8,31,15,3),(4,6.65,24,12,3)]:
        c=center(name,ident)
        if ident in [0,1,3]:
            q=Mesh();q.box((0,(18+y-6)/2,0),(r*2.4,y-6-18,r*2.4),WHITE,17)
            for heading in [0,90,180,270]:
                f=Mesh()
                for xx in [-r*.72,0,r*.72]:
                    arch(f,(xx,20,-r*1.2-.08),r*.48,y-29,.22,WHITE,True,(24,16,8)[lod],(128,158,157),17)
                for yy in [19,y-7]:f.box((0,yy,-r*1.2-.14),(r*2.4,.6,.5),WHITE,17)
                q.merge(turn(f,heading))
            m.merge(turn(q,-12.4,c[0],0,c[1]))
        else:lathe(m,tuple(c),[(17,r*.92),(y-6,r*.92)],WHITE,17,24)
        drum(m,*c,r*.92,y-6,6,lod,WHITE,24);cap(m,*c,r,y,h,lod,WHITE,True,material=17);finial(m,c[0],y+h,c[1],fh,lod)
    for ident in range(322,359):
        c=center(name,ident);r=min(np.ptp(poly(name,ident),axis=0))/2
        y=23 if ident>=356 else 20
        drum(m,*c,r*.95,17,y-17,lod,WHITE,12);cap(m,*c,r,y,r*1.15,lod,WHITE,True,material=17);finial(m,c[0],y+r*1.15,c[1],1.5,lod)
    # Forty secondary roof domes complete the 82-dome ensemble. Their centers
    # interpolate the gaps of the measured main-hall domes; this is recorded.
    a=np.array([-100.,-7.]);along=np.array([.217,-.976]);across=np.array([.976,.217])
    for row in range(10):
        for side in [-1,1]:
            for off in [20,29]:
                c=a+along*row*13+across*side*off
                cap(m,*c,3.8,18,5.5,lod,WHITE,True,material=17)
    for w in zayed_platform():
        if w['tags'].get('tower:type')=='minaret':
            p=np.asarray(w['xz'][:-1]);x,z=(p.min(0)+p.max(0))/2
            gulf_minaret(m,x,z,107,lod,3.4)
    ranges(m,[court],.07,(239,237,225))
    if lod<2:
        # Original colored stone floral motifs echo the documented inlaid court.
        for dx in [-32,0,32]:
            for dz in [-44,0,44]:
                x,z=cc+np.array([dx,dz])
                for k in range(8):
                    a=k*math.tau/8;petal=[]
                    for t in np.linspace(0,math.tau,10):
                        rr=3.8+3.7*math.cos(t);perp=1.2*math.sin(t)
                        petal.append((x+rr*math.cos(a)-perp*math.sin(a),.09,z+rr*math.sin(a)+perp*math.cos(a)))
                    face(m,petal,[(156,93,111),(204,162,61),(119,155,120)][(dx+dz)%3],17)
    return m


def hip_roof(m,x,z,w,d,y,h,lod,color=GREEN):
    ridge=max(0,w-d)/2
    for side in [-1,1]:
        face(m,[(x-w/2,y,z+side*d/2),(x+w/2,y,z+side*d/2),(x+ridge,y+h,z),(x-ridge,y+h,z)],color,85,(0,1,side))
        face(m,[(x+side*w/2,y,z-d/2),(x+side*w/2,y,z+d/2),(x+side*ridge,y+h,z)],color,85,(side,1,0))
    if lod<2:
        for xx in np.arange(-w/2+1,w/2,1.6):
            if abs(xx)<ridge:
                for side in [-1,1]:tube(m,(x+xx,y+.03,z+side*d/2),(x+xx,y+h+.03,z),.035,(63,124,98),17,4)


def _hassan(lod):
    m=Mesh();name='hassan-ii-mosque';sand=(211,191,160);c,w,d,a=measured_box(name,43)
    # The hall is aligned at ~3.3 degrees while its square minaret is diagonal.
    # FMH2 publishes the principal floor at9.50mNGM and the successive
    # structural levels at36.40/47.40/55.40/60.90mNGM, including the sliding roof.
    for ident in [44,41,2,48,49,50,51,52]:
        ranges(m,[poly(name,ident)],0,sand,bottom=-10,material=82)
    ranges(m,[hassan_hall_footprint()],0,sand,bottom=-10,material=82)
    q=Mesh();q.box((0,13.45,0),(200,26.9,100),sand)
    for aa,bb in [((-100,-50),(100,-50)),((100,50),(-100,50)),((100,-50),(100,50)),((-100,50),(-100,-50))]:masonry(q,aa,bb,0,26.9,sand,lod)
    for yy in [1,4,24.4,26.4]:q.box((0,yy,0),(201,.7,101),WHITE,17)
    q.box((0,32.4,0),(182,11,82),sand);q.box((0,41.9,0),(169,8,69),sand)
    hip_roof(q,0,0,169,69,45.9,4.8,lod)
    # The movable central roof is represented closed and stays below tower crown.
    q.box((0,48,0),(76,4,48),sand);hip_roof(q,0,0,78,50,50,1.4,lod)
    for side in [-1,1]:
        for xx in np.arange(-90,96,12):
            f=Mesh();arch(f,(xx,3,-50.08),7,15,.35,WHITE,True,(24,16,8)[lod],(99,86,67),17)
            f.box((xx,21.3,-50.2),(9,.8,.3),GREEN,17)
            for off in [-4.8,4.8]:f.box((xx+off,12,-50.25),(.9,22,1.2),WHITE,17)
            q.merge(turn(f,0 if side<0 else 180))
        for xx in [-83,-60,-36,36,60,83]:
            for zz in [-43,43]:
                p=Mesh();p.box((0,25.5,0),(13,4,10),sand);hip_roof(p,0,0,14,11,27.5,3,lod);q.merge(turn(p,0,xx,0,zz))
    for heading in [90,270]:
        f=Mesh()
        for xx in [-32,-16,0,16,32]:arch(f,(xx,3,-100.06),8.2,18,.4,WHITE,True,(24,16,8)[lod],SHADOW,17)
        q.merge(turn(f,heading))
    m.merge(turn(q,a,c[0],0,c[1]))
    # The 25m square shaft, green lattice panels and stacked lantern are specific
    # to Hassan II. Foundation's 190.5m structural height excludes the jamour.
    tc,tw,td,ta=measured_box(name,41);q=Mesh()
    q.box((0,84,0),(25,168,25),sand)
    for yy in [2,9,27,83,147,162,168]:q.box((0,yy,0),(26.2,1.0,26.2),WHITE,17)
    for heading in [0,90,180,270]:
        f=Mesh();arch(f,(0,5,-12.57),10.8,20,.45,WHITE,True,(24,16,8)[lod],SHADOW,17)
        f.box((0,91,-12.57),(16,103,.16),GREEN,17)
        for xx in [-9,9]:f.box((xx,91,-12.8),(1,105,.4),WHITE,17)
        # Raised interlaced diamond tracery: geometry, not a flat green stripe.
        for yy in np.arange(43,138,(5,5,10)[lod]):
            for xx in [-6,-2,2,6]:
                for sign in [-1,1]:tube(f,(xx-2,yy,-12.85),(xx+2,yy+sign*5,-12.85),.18,CREAM,17,4)
        for xx in [-6,0,6]:window(f,xx,149,-12.62,3,9,lod,WHITE,True,False)
        q.merge(turn(f,heading))
    q.box((0,171.5,0),(21,7,21),GREEN)
    q.box((0,181.5,0),(15,16,15),sand)
    for heading in [0,90,180,270]:
        f=Mesh();window(f,0,175,-7.6,7,11,lod,WHITE,True);q.merge(turn(f,heading))
    q.box((0,190,0),(16.5,1,16.5),WHITE)
    hip_roof(q,0,0,17,17,190.5,4,lod)
    finial(q,0,194.5,0,15.5,lod,False)
    m.merge(turn(q,ta,tc[0],0,tc[1]))
    for ident in [2,48,49,50,51,52]:shell(m,name,ident,12 if ident==2 else 15,sand)
    return m


def earth_pylon(m,x,z,w,d,y,h,lod,color):
    # Bevelled, tapering mud faces preserve softened corners without dense mesh.
    rings=[]
    for yy,scale in [(y,1),(y+h*.72,.75),(y+h,.48)]:
        ww,dd=w*scale/2,d*scale/2;b=min(ww,dd)*.25
        rings.append([(x+xx,yy,z+zz) for xx,zz in [(-ww+b,-dd),(ww-b,-dd),(ww,-dd+b),(ww,dd-b),(ww-b,dd),(-ww+b,dd),(-ww,dd-b),(-ww,-dd+b)]])
    for a,b in zip(rings,rings[1:]):
        for i in range(8):
            j=(i+1)%8;mid=(np.array(a[i])+a[j])/2
            face(m,[a[i],a[j],b[j],b[i]],color,11,(mid[0]-x,0,mid[2]-z))
    face(m,rings[-1],color)
    lathe(m,(x,z),[(y+h,min(w,d)*.24),(y+h+min(2,min(w,d)*.38),.04)],color,11,(12,8,6)[lod])


def _djenne(lod):
    m=Mesh();name='great-mosque-of-djenne';clay=(178,136,98);wood=(86,66,42)
    rs=part(name,6448)['rings'];p=boundary(rs[0]);court=boundary(rs[1],True)
    plan_solid(m,[[-70,-39],[-7,-39],[-3,25],[-69,25]],-1,2.7,clay)
    ranges(m,rs,11.5,clay,2.7,EARTHEN_PLASTER);ranges(m,[court],2.75,(184,146,107))
    # Three unequal qibla towers are in the mapped east facade projections.
    for x,z,w,d,h in [(-10,8.5,7.3,6.0,12.9),(-12.2,-5.5,6.3,6,13.8),(-13.5,-20,5.7,6,12.4)]:
        earth_pylon(m,x,z,w,d,2.7,h,lod,clay)
        m.ellipsoid((x,2.7+h+min(2,min(w,d)*.38)+.27,z),(.2,.34,.2),(224,208,176),17,8,5)
        for yy in np.arange(5,h+2,1.65):
            for dz in [-1.8,0,1.8]:
                tube(m,(x+w*.31,yy,z+dz),(x+w*.31+1.55,yy+.05,z+dz),.11,wood,84,(6,5,4)[lod])
    for aa,bb in zip(p,np.roll(p,-1,axis=0)):
        v=bb-aa;length=np.linalg.norm(v)
        if length<3:continue
        for t in np.arange(1.8,length,4.2):
            x,z=aa+v*t/length;earth_pylon(m,x,z,1.65,1.65,2.7,9.6,lod,clay)
            normal=np.array([v[1],-v[0]])/length
            for yy in [5.0,7.3,9.5]:
                for off in [-.25,.25]:
                    a=np.array([x,z])+v/length*off
                    tube(m,(a[0],yy,a[1]),(a[0]+normal[0]*1.25,yy+.06,a[1]+normal[1]*1.25),.10,wood,84,5 if lod==0 else 4)
    for a,b in zip(court,np.roll(court,-1,axis=0)):facade(m,a,b,2.7,8.8,lod,clay,4,1,True,83)
    # Ceramic roof vents remain small rounded forms; they are not domes.
    for x in [-34,-28,-22]:
        for z in np.arange(-28,17,4.8):lathe(m,(x,z),[(11.5,.38),(11.9,.5),(12.2,.32)],(166,115,78),11,(10,8,6)[lod])
    for z in [-30,14]:
        q=Mesh();stairs(q,(0,-5),6.0,5,2.7,9,clay);m.merge(turn(q,90,-67,0,z))
    return m


def persian_minaret(m,x,z,h,lod):
    lathe(m,(x,z),[(-1,1.65),(7,1.65),(h-8,1.28)],lambda k,j:TURQUOISE if j%5 else BLUE,85,(32,20,12)[lod])
    for y in [6,h*.42,h*.65,h-8]:lathe(m,(x,z),[(y,1.72),(y+.8,1.72)],GOLD if y==6 else WHITE,17,(32,20,12)[lod])
    lathe(m,(x,z),[(h-8,1.3),(h-6.5,2.25),(h-5.4,2.25),(h-5.3,1.3),(h-1.5,1.3)],CREAM,17,(32,20,12)[lod])
    for k in range(8):
        a=k*math.tau/8;column(m,x+1.3*math.cos(a),z+1.3*math.sin(a),h-5,3.7,.16,TURQUOISE,lod)
    cap(m,x,z,1.85,h-1.5,1.5,lod,TURQUOISE,True)


def _shah(lod):
    m=Mesh();name='shah-mosque';rs=part(name,63)['rings'];ranges(m,rs,8.5,(185,158,119),-1)
    ranges(m,[rs[0]],-.03,(185,158,119),bottom=-5,material=81)
    for ident,h in [(31,10),(32,21),(41,13),(42,13)]:shell(m,name,ident,h,(190,165,125))
    # Four iwans frame the sanctuary-aligned courtyard. The square-facing portal
    # has its independently measured heading, about 45 degrees off that axis.
    for ident,h in [(45,28),(48,30),(49,18),(50,18),(51,18)]:
        c,w,d,a=measured_box(name,ident)
        court=np.mean(np.asarray(rs[1]),axis=0);direction=c-court
        front=np.array([-math.sin(math.radians(a)),-math.cos(math.radians(a))])
        if np.dot(front,-direction)<0:a+=180
        if ident==45:a=math.degrees(math.atan2(-(poly(name,45)[1]-poly(name,45)[0])[1],(poly(name,45)[1]-poly(name,45)[0])[0]))+90
        iwan(m,*c,w,h,6 if ident==45 else 8,lod,a,True)
    c=center(name,23);drum(m,*c,12.5,21,9,lod,TURQUOISE,24,85);cap(m,*c,13.4,30,21,lod,TURQUOISE,True,pattern=True,material=85);finial(m,c[0],51,c[1],1,lod)
    for ident in [35,36]:
        c=center(name,ident);cap(m,*c,8.8,13,9,lod,(166,164,119),False,pattern=True,material=85)
    for ident in [43,44,46,47]:
        c=center(name,ident);persian_minaret(m,*c,48,lod)
    for index,courtyard in enumerate(rs[1:]):
        p=np.asarray(courtyard);ranges(m,[p],.03,(201,196,176))
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            if np.linalg.norm(b-a)<=22:continue
            if index==0:
                v=b-a;length=np.linalg.norm(v);unit=v/length
                gap=20 if length>65 else 15;mid=(a+b)/2
                arcades(m,a,mid-unit*gap/2,0,8.2,2.7,lod,BLUE,5.0)
                arcades(m,mid+unit*gap/2,b,0,8.2,2.7,lod,BLUE,5.0)
            else:arcades(m,a,b,0,8.2,2.7,lod,(176,155,120),5.0)
    # Long rectangular reflecting pool follows the prayer-court axis.
    q=Mesh();q.box((0,.10,0),(11,.2,22),WHITE);q.box((0,.22,0),(10,.04,21),(64,127,136),15)
    cc=np.mean(np.asarray(rs[1]),axis=0);m.merge(turn(q,31,cc[0],0,cc[1]))
    return m


def qaboos_minaret(m,x,z,h,lod,main=False):
    r=5.45 if main else 2.9
    m.box((x,h*.18,z),(r*2,h*.36,r*2),CREAM)
    lathe(m,(x,z),[(h*.36,r*.91),(h*.38,r*.91),(h*.38,r*.76),(h*.75,r*.67),(h*.78,r*.9),(h*.80,r*.9),(h*.83,r*.7),(h-6,r*.60)],CREAM,11,(16,12,8)[lod])
    for y in [h*.36,h*.76,h*.81]:lathe(m,(x,z),[(y,r*.96),(y+.65,r*.96)],WHITE,17,(24,16,10)[lod])
    cap(m,x,z,r*.70,h-6,4.5,lod,WHITE,True,material=17);finial(m,x,h-1.5,z,1.2,lod)
    for heading in [0,90,180,270]:
        q=Mesh();window(q,0,8,-r-.04,r,12 if main else 6,lod,WHITE,True)
        m.merge(turn(q,heading,x,0,z))


def qaboos_podium():
    data=json.loads((REF/'mosque-floor-calibrations.json').read_text())['sultan-qaboos-grand-mosque']
    m=Mesh();ranges(m,[data['podium_xz']],-.03,CREAM,bottom=-12,material=82)
    p=np.asarray(next(w['xz']for w in data['approach_ways']if w['id']==1448186367)[:-1])
    # Six broad steps connect the mapped north entrance to the1.8m raised sahn.
    for lo,hi in zip(np.linspace(65,82.5,7)[:-1],np.linspace(65,82.5,7)[1:]):
        q=_clip_axis(_clip_axis(p,np.array([0,1]),lo,True),np.array([0,1]),hi,False)
        if len(q)<3:continue
        y=-1.8*min(1,(hi-65)/17.5)
        ranges(m,[q],y,CREAM,bottom=-12,material=82)
    return m


def _qaboos(lod):
    m=Mesh();name='sultan-qaboos-grand-mosque';rs=part(name,518)['rings']
    m.merge(qaboos_podium())
    ranges(m,rs,16.7,CREAM,-1)
    # Main hall measured 72m outside the OSM wall, documented 74.4m envelope.
    c=center(name,508)
    m.box((c[0],18,c[1]),(69.5,2.6,69.5),CREAM)
    drum(m,*c,13.5,19.3,3.7,lod,CREAM,24)
    cap(m,*c,13.5,23,19.85,lod,(199,197,174),material=88)
    # Distinctive golden external lattice sits clear of the inner dome shell.
    if lod<2:
        for k in range(32 if lod==0 else 24):
            a=k*math.tau/(32 if lod==0 else 24)
            for direction in [-1,1]:
                path=[]
                for t in np.linspace(0,1,16):
                    r=13.66*math.cos(t*math.pi/2);theta=a+direction*t*.8
                    path.append((c[0]+r*math.cos(theta),23+19.9*t,c[1]+r*math.sin(theta)))
                for aa,bb in zip(path,path[1:]):tube(m,aa,bb,.13,GOLD,17,4)
    finial(m,c[0],42.85,c[1],7.15,lod)
    for courtyard in rs[1:]:
        ranges(m,[courtyard],.08,(225,216,193))
        for a,b in zip(courtyard,np.roll(courtyard,-1,axis=0)):arcades(m,a,b,0,9.5,2.4,lod,WHITE,4.6)
    outside=boundary(rs[0])
    for a,b in zip(outside,np.roll(outside,-1,axis=0)):
        facade(m,a,b,0,16.7,lod,WHITE,6.1,2,True);parapet(m,a,b,16.7,1.0,CREAM,lod,True)
    for ident in [149,377,378,379,382,383,419,422,423,424,425,426,427,512,514,513]:
        p=poly(name,ident);c,w,d,a=measured_box(name,ident)
        h=8.0 if w>40 else 10.5
        if w>35 and d<15:
            v=(p[np.argmax(np.linalg.norm(p-c,axis=1))]-c);v/=np.linalg.norm(v)
            # Long riwaqs use an open arcade along their surveyed long axis.
            axis=np.array([math.cos(math.radians(a)),-math.sin(math.radians(a))])
            arcades(m,c-axis*w/2,c+axis*w/2,0,h,d,lod,CREAM,5.3)
        else:shell(m,name,ident,h,CREAM)
    for ident in [374,375,376,509,510]:
        c=center(name,ident);qaboos_minaret(m,*c,91.5 if ident==375 else 45,lod,ident==375)
    return m


BUILDERS={'hagia-sophia':_hagia,'blue-mosque':_blue,'sheikh-zayed-grand-mosque':_zayed,
          'hassan-ii-mosque':_hassan,'great-mosque-of-djenne':_djenne,
          'shah-mosque':_shah,'sultan-qaboos-grand-mosque':_qaboos}
_FOOTPRINTS={
 'hagia-sophia':[1408,1409,1410,1411,1415,1404,1405,1406,1407,1422,1423,1427,1428,1429,1431,1432,1433,1434,1435,1436,1437,1438,1439,1443,1444,1445,1430,1452],
 'blue-mosque':[1503,175,178,180,182,184,186], 'sheikh-zayed-grand-mosque':[359],
 'hassan-ii-mosque':[44,41,2,48,49,50,51,52],
 'great-mosque-of-djenne':[6448], 'shah-mosque':[63],
 'sultan-qaboos-grand-mosque':[518,149,377,378,379,382,383,419,422,423,424,425,426,427,512,514,513,374,375,376,509,510],
}


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('LOD must be 0,1,2')
    m=BUILDERS[name](lod)
    wall={'hagia-sophia':83,'blue-mosque':82,'sheikh-zayed-grand-mosque':VEINED_MARBLE,
          'hassan-ii-mosque':82,'great-mosque-of-djenne':EARTHEN_PLASTER,'shah-mosque':81,
          'sultan-qaboos-grand-mosque':82}[name]
    roof={'hagia-sophia':88,'blue-mosque':88,'sheikh-zayed-grand-mosque':VEINED_MARBLE,
          'hassan-ii-mosque':85,'great-mosque-of-djenne':EARTHEN_PLASTER,'shah-mosque':85,
          'sultan-qaboos-grand-mosque':91}[name]
    m.triangles=[(p,c,wall if mat==11 else roof if mat==12 else mat) for p,c,mat in m.triangles]
    roles=({17:VEINED_MARBLE,82:VEINED_MARBLE} if name=='sheikh-zayed-grand-mosque' else
           {17:EARTHEN_PLASTER,83:EARTHEN_PLASTER} if name=='great-mosque-of-djenne' else
           {82:WEATHERED_LIMESTONE} if name in ['hagia-sophia','blue-mosque'] else {})
    m.triangles=[(p,c,roles.get(mat,mat)) for p,c,mat in m.triangles]
    return m


def ownership(name,approach=True):
    m=Mesh()
    for ident in _FOOTPRINTS[name]:ranges(m,part(name,ident)['rings'],0,WHITE)
    if name=='hagia-sophia':
        for p in hagia_minaret_footprints():ranges(m,[p],0,WHITE)
    if name=='hassan-ii-mosque':ranges(m,[hassan_hall_footprint()],0,WHITE)
    # These courts have authored paving. Owning them also lowers contaminated
    # source ground beneath that paving while leaving the surrounding city alone.
    court_parent={'blue-mosque':1503,'sheikh-zayed-grand-mosque':359,'hassan-ii-mosque':2,
                  'great-mosque-of-djenne':6448,'shah-mosque':63,
                  'sultan-qaboos-grand-mosque':518}.get(name)
    if court_parent is not None:
        for p in part(name,court_parent)['rings'][1:]:ranges(m,[p],0,WHITE)
    if name=='sheikh-zayed-grand-mosque' and approach:
        m.triangles.extend((p,c,mat)for p,c,mat in zayed_approach().triangles if np.cross(p[1]-p[0],p[2]-p[0])[1]>1e-8)
    if name=='sultan-qaboos-grand-mosque':
        m.triangles.extend((p,c,mat)for p,c,mat in qaboos_podium().triangles if np.cross(p[1]-p[0],p[2]-p[0])[1]>1e-8)
    return [[[float(v[0]),float(v[2])] for v in p] for p,_,_ in m.triangles]


_RESEARCH=json.loads((REF/'research.json').read_text())['sites']
ENTRIES=[]
for name in BUILDERS:
    s=survey(name);r=_RESEARCH[name];p=ownership(name)
    ENTRIES.append({'name':name,'city':r['city'],'lat':s['lat'],'lon':s['lon'],'heading':0.0,
      'base':floor_datum(name),'suppress_buildings':True,'suppress_roads':name in ['hassan-ii-mosque','shah-mosque','sultan-qaboos-grand-mosque'],'suppress_trees':True,
      'source_exclusion_polygons':p,'terrain_flatten':[],
      'terrain_cuts':mosque_cuts(name,p),'water_cuts':mosque_water_cuts(name),
      'terrain_grades':mosque_setting_grades(name)+shah_qaboos_grades(name),
      'references':r['references']+['https://www.openstreetmap.org/copyright'],
      'description':r['layout'],'attribution':'Survey plans © OpenStreetMap contributors, via Protomaps, ODbL 1.0. Architectural geometry originally authored for Vearth.',
      'capture':{'distance':480 if name in ['hassan-ii-mosque','sheikh-zayed-grand-mosque'] else 270,
                 'azimuth':135,'elevation':30,'target_height':25,'time_utc':'2026-03-20T10:00:00Z'},
      'audit':{'survey_file':str((REF/(name+'-survey.json')).relative_to(REF.parents[3])),
               'dimensional_evidence':r['published'],'dimension_uncertainty':r['uncertainty'],
               'orientation':'Source footprints in east/north metric coordinates; all component rotations baked into vertices.',
               'grounding':'Fixed production sampled datum with narrow lower-only cuts under authored masonry; no whole-site terrain flattening.'}})
    if name=='hagia-sophia':
        ENTRIES[-1]['audit']['grounding']='Estimated39.0m west-entry floor, independent local tomb thresholds, and buried masonry supports; calibration and raw approach samples in mosque-floor-calibrations.json.'
    if name in ['hassan-ii-mosque','shah-mosque','sultan-qaboos-grand-mosque']:
        ENTRIES[-1]['road_exclusion_polygons']=p
        ENTRIES[-1]['audit']['grounding']='Hassan II floor uses published 9.50m NGM with authored sea-level basement supports; Shah/Qaboos use clean approach calibration in mosque-floor-calibrations.json. Bounded cuts stay beneath authored masonry/paving.'
    if name in ['hassan-ii-mosque','sheikh-zayed-grand-mosque']:
        ENTRIES[-1]['audit']['approach_grounding']='Independent bounded apron grades repair source terrain discontinuities while retaining fixed architectural floors; measured context, inferred transition extents and controls are recorded in setting-ground-calibrations.json.'
    if name in ['shah-mosque','sultan-qaboos-grand-mosque']:
        ENTRIES[-1]['audit']['approach_grounding']='Independent bounded entrance grades retain fixed floors and the Qaboos raised platform; measured levels, mapped approach axes and estimated blend extents are recorded in shah-qaboos-setting-ground.json.'
