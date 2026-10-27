"""Panelled swept envelopes and exposed framing, in model metres."""
import math
import numpy as np
from heritage_catalog.geometry import Mesh, face, plan_triangles, plan_solid
from heritage_catalog.materials import WINDOW, SCULPTURE_METAL, FLAT_ROOF, PANEL_WALL
from skyscraper_catalog.appearance import muted_frame, panel_tint


def ccw(points):
    p=np.asarray(points,float)
    if np.linalg.norm(p[0]-p[-1])<1e-6:p=p[:-1]
    if sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
    return p


def resample(points,count):
    p=ccw(points);q=np.roll(p,-1,axis=0);length=np.linalg.norm(q-p,axis=1)
    cum=np.r_[0,np.cumsum(length)];out=[]
    for t in np.linspace(0,cum[-1],count,endpoint=False):
        i=min(len(p)-1,np.searchsorted(cum,t,side='right')-1)
        out.append(p[i]+(q[i]-p[i])*(t-cum[i])/length[i])
    return np.asarray(out)


def rounded_square(width,depth=None,radius=5,steps=5):
    depth=width if depth is None else depth
    r=min(radius,width/2,depth/2);pts=[]
    for cx,cz,start in [(width/2-r,depth/2-r,0),(-width/2+r,depth/2-r,90),
                         (-width/2+r,-depth/2+r,180),(width/2-r,-depth/2+r,270)]:
        for a in np.linspace(start,start+90,steps,endpoint=False):
            a=math.radians(a);pts.append((cx+r*math.cos(a),cz+r*math.sin(a)))
    return ccw(pts)


def chamfer(width,depth=None,cut=5):
    d=width if depth is None else depth;w=width/2;d/=2;c=cut
    return ccw([(-w+c,-d),(w-c,-d),(w,-d+c),(w,d-c),(w-c,d),(-w+c,d),(-w,d-c),(-w,-d+c)])


def rotated(p,angle):
    a=math.radians(angle);return np.asarray(p)@np.array([[math.cos(a),math.sin(a)],[-math.sin(a),math.cos(a)]])


def roof(mesh,plan,y,color=(73,82,84),material=FLAT_ROOF):
    for t in plan_triangles(plan):face(mesh,[(x,y,z)for x,z in t],color,material)


def loft(mesh,ring_at,levels,lod,glass=(90,126,140),metal=(161,172,174),
         floor=4.1,spandrel=.55,mullion=.10,frame_material=PANEL_WALL,
         cap=True,glass_material=103):
    """Sweep the envelope with the production native high-rise facade finish.

    Native shaders own floor/window bands. Geometry supplies broad continuous
    facade strips and real structural ribs, never a second per-window grid.
    """
    controls=sorted(set(levels))
    pitch=(18.,30.,55.)[lod]
    levels=sorted(set(controls+[float(y)for a,b in zip(controls,controls[1:])
                                for y in np.arange(a+pitch,b,pitch)]))
    frame=muted_frame(metal)
    tint=panel_tint(glass_material,frame)
    for lo,hi in zip(levels,levels[1:]):
        a=ccw(ring_at(lo));b=ccw(ring_at(hi));assert len(a)==len(b)
        perimeter=float(np.linalg.norm(np.roll(a,-1,axis=0)-a,axis=1).sum())
        stride=max(1,round(len(a)*4.5/max(perimeter,1)))
        for j in range(len(a)):
            k=(j+1)%len(a)
            p=np.array([(a[j,0],lo,a[j,1]),(a[k,0],lo,a[k,1]),
                        (b[k,0],hi,b[k,1]),(b[j,0],hi,b[j,1])])
            normal=np.array([a[k,1]-a[j,1],0,a[j,0]-a[k,0]])
            if lod<2 and j%stride==0:
                width=max(.1,np.linalg.norm(a[k]-a[j]));u=min(.28,max(.32,mullion)/width)
                p1=p[0]*(1-u)+p[1]*u;p2=p[3]*(1-u)+p[2]*u
                face(mesh,[p[0],p1,p2,p[3]],frame,frame_material,normal)
                face(mesh,[p1,p[1],p[2],p2],tint,glass_material,normal)
            else:
                face(mesh,p,tint,glass_material,normal)
    if cap:roof(mesh,ring_at(levels[-1]),levels[-1])


def ribs(mesh,ring_at,levels,indices,radius=.16,color=(179,188,190),material=PANEL_WALL,lod=0):
    for i in indices:
        for a,b in zip(levels,levels[1:]):
            p=ring_at(a)[i];q=ring_at(b)[i]
            mesh.beam((p[0],a,p[1]),(q[0],b,q[1]),radius,color,material,4 if lod else 6)


def band(mesh,ring_at,y,height,color=(60,70,74),material=PANEL_WALL):
    color=muted_frame(color)
    a=ccw(ring_at(y));b=ccw(ring_at(y+height))
    for j in range(len(a)):
        k=(j+1)%len(a);n=np.array([a[k,1]-a[j,1],0,a[j,0]-a[k,0]])
        n/=max(1e-9,np.linalg.norm(n))
        p=np.array([(a[j,0],y,a[j,1]),(a[k,0],y,a[k,1]),(b[k,0],y+height,b[k,1]),(b[j,0],y+height,b[j,1])])
        face(mesh,p+n*.35,color,material,n)


def diagonals(mesh,ring_at,heights,count,radius=.40,color=(179,186,182),lod=0,phase=0,curved=False):
    """Equal perimeter structural nodes, independent of glazing subdivision."""
    for row,(a,b)in enumerate(zip(heights,heights[1:])):
        low=resample(ring_at(a),count);high=resample(ring_at(b),count)
        for i,p in enumerate(low):
            for direction in (-1,1):
                q=high[(i+direction+phase)%count]
                if not curved:
                    mesh.beam((p[0],a,p[1]),(q[0],b,q[1]),radius,color,SCULPTURE_METAL,4 if lod else 6)
                    continue
                # A straight perimeter chord buries its middle inside a convex
                # curtain wall. Follow its surface and keep the steel centreline
                # proud; the same structural endpoints survive every LOD.
                steps=(8,6,4)[lod];points=[];normals=[]
                for j in range(steps+1):
                    t=j/steps;y=a+(b-a)*t
                    ring=resample(ring_at(y),count*steps)
                    index=(i*steps+(direction+phase)*j)%len(ring)
                    v=ring[index];tangent=ring[(index+1)%len(ring)]-ring[(index-1)%len(ring)]
                    normal=np.array([tangent[1],-tangent[0]])
                    normal/=max(1e-9,np.linalg.norm(normal));v=v+normal*radius*.7
                    points.append((v[0],y,v[1]))
                    normals.append(np.array([normal[0],0,normal[1]]))
                points=np.asarray(points);rings=[]
                for j,p0 in enumerate(points):
                    tangent=points[min(j+1,len(points)-1)]-points[max(0,j-1)]
                    side=np.cross(normals[j],tangent);side/=np.linalg.norm(side)
                    if lod:
                        rings.append([p0-side*radius,p0+side*radius])
                    else:
                        rings.append([p0+radius*v for v in [normals[j],side,-normals[j],-side]])
                for j,(low_ring,high_ring)in enumerate(zip(rings,rings[1:])):
                    if lod:
                        face(mesh,[low_ring[0],low_ring[1],high_ring[1],high_ring[0]],color,SCULPTURE_METAL,normals[j])
                    else:
                        for k in range(4):
                            outward=low_ring[k]+low_ring[(k+1)%4]-2*points[j]
                            face(mesh,[low_ring[k],low_ring[(k+1)%4],high_ring[(k+1)%4],high_ring[k]],color,SCULPTURE_METAL,outward)
                if not lod:
                    face(mesh,list(reversed(rings[0])),color,SCULPTURE_METAL,points[0]-points[1])
                    face(mesh,rings[-1],color,SCULPTURE_METAL,points[-1]-points[-2])


def grid_prism(mesh,plan,bottom,top,lod,glass=(94,122,131),metal=(170,178,175),floor=4.0,bay=2.2,spandrel=.65):
    p=ccw(plan)
    frame=muted_frame(metal)
    tint=panel_tint(103,frame)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        length=np.linalg.norm(b-a);count=min((12,7,4)[lod],max(1,round(length/(4.5,7.0,12.)[lod])))
        normal=(b[1]-a[1],0,a[0]-b[0])
        def strip(u,v,color,material):
            x=a+(b-a)*u;y=a+(b-a)*v
            face(mesh,[(x[0],bottom,x[1]),(y[0],bottom,y[1]),(y[0],top,y[1]),(x[0],top,x[1])],color,material,normal)
        if lod==2:strip(0,1,tint,103)
        else:
            pier=min(.30/count,.32/max(length,1e-6))
            for j in range(count):
                strip(j/count,j/count+pier,frame,PANEL_WALL)
                strip(j/count+pier,(j+1)/count,tint,103)
    roof(mesh,p,top)
    for tri in plan_triangles(p):face(mesh,[(x,bottom,z)for x,z in tri],frame,FLAT_ROOF,(0,-1,0))
