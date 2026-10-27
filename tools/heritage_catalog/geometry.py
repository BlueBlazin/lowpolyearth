"""Shared metric stonework; facade apertures face local -z before rotation."""
import math

import numpy as np

from city_catalog.common import Mesh
from city_catalog.expansion_heritage import (
    _arch as arch, _arch_points as arch_points, _dome as dome,
    _gable as gable, _lathe as lathe, _ring as ring,
    _stairs as stairs, _tube as tube, _turn as turn,
)

STONE = (206, 199, 180)
TRIM = (225, 219, 201)
SHADOW = (44, 49, 47)
LEAD = (91, 102, 105)
GOLD = (193, 153, 61)


def face(mesh, points, color, material=11, normal=(0, 1, 0)):
    points = np.asarray(points, float)
    # Scanline roof strips can start with a collapsed edge. Orient from the
    # first surviving fan triangle so backface culling preserves the roof.
    for i in range(1, len(points)-1):
        area = np.cross(points[i]-points[0], points[i+1]-points[0])
        if np.linalg.norm(area) > 1e-9:
            if np.dot(area, normal) < 0:
                points = points[::-1]
            break
    for i in range(1, len(points)-1):
        mesh.triangle(points[0], points[i], points[i+1], color, material)


def plan_triangles(polygon):
    """Triangulate a simple plan without filling re-entrant courtyard corners."""
    points = np.asarray(polygon, float)
    if np.linalg.norm(points[0]-points[-1]) < 1e-8:
        points = points[:-1]
    indices = list(range(len(points)))
    cross2 = lambda a, b: a[0]*b[1]-a[1]*b[0]
    area = sum(cross2(a,b) for a,b in zip(points,np.roll(points,-1,axis=0)))
    if area < 0:
        indices.reverse()
    while len(indices)>3:
        for k,j in enumerate(indices):
            i,h=indices[k-1],indices[(k+1)%len(indices)]
            a,b,c=points[[i,j,h]]
            area=cross2(b-a,c-b)
            if abs(area)<1e-8:
                indices.pop(k)
                break
            if area<0:
                continue
            if any(all(cross2(v-u,points[q]-u)>=-1e-8 for u,v in [(a,b),(b,c),(c,a)])
                   for q in indices if q not in (i,j,h)):
                continue
            yield points[[i,j,h]]
            indices.pop(k)
            break
        else:
            raise ValueError('plan must be a simple polygon without holes')
    if len(indices)==3:
        yield points[indices]


def plan_solid(mesh, polygon, bottom, top, color=STONE, material=11):
    """Closed vertical prism with a concave plan; heights are model metres."""
    points=np.asarray(polygon,float)
    for tri in plan_triangles(points):
        for y,normal in [(bottom,(0,-1,0)),(top,(0,1,0))]:
            face(mesh,[(x,y,z) for x,z in tri],color,material,normal)
    winding=np.sign(sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(points,np.roll(points,-1,axis=0))))
    for a,b in zip(points,np.roll(points,-1,axis=0)):
        edge=b-a
        face(mesh,[(a[0],bottom,a[1]),(b[0],bottom,b[1]),(b[0],top,b[1]),(a[0],top,a[1])],
             color,material,(winding*edge[1],0,-winding*edge[0]))


def column(mesh, x, z, bottom, height, radius, color=TRIM, lod=0):
    profile = [(bottom, radius*1.28), (bottom+.35, radius*1.28),
               (bottom+.55, radius), (bottom+height-.85, radius*.84),
               (bottom+height-.55, radius*1.1), (bottom+height-.35, radius*1.32),
               (bottom+height, radius*1.32)]
    lathe(mesh, (x, z), profile, color, 17, (16, 12, 8)[lod])


def balustrade(mesh, a, b, bottom, height=1.5, lod=0, color=TRIM):
    a, b = np.asarray(a, float), np.asarray(b, float)
    delta = b-a
    length = np.linalg.norm(delta)
    if length <= 0:
        raise ValueError('balustrade length must be positive')
    direction = delta/length
    gap = (1.05, 1.65, 2.7)[lod]
    count = max(1, int(length/gap))
    for t in np.linspace(0, 1, count+1):
        x, z = a+delta*t
        lathe(mesh, (x, z), [(bottom,.18), (bottom+.2,.18),
              (bottom+.4,.12), (bottom+height*.6,.21),
              (bottom+height-.2,.12), (bottom+height,.18)], color, 17, (8, 6, 4)[lod])
    for y in (bottom+.1, bottom+height):
        sub = Mesh()
        sub.box((0, y, 0), (length+.35,.25,.46), color, 17)
        angle = math.degrees(math.atan2(-direction[1], direction[0]))
        mesh.merge(turn(sub, angle, *(float(v) for v in [(a[0]+b[0])/2, 0, (a[1]+b[1])/2])))


def arch_bay(mesh, center, width, height, depth, pier=.75, color=STONE, lod=0, pointed=False):
    """Freestanding masonry bay with a real opening, not an opaque decal."""
    x, y, z = center
    opening = width-2*pier
    if opening <= 0 or height <= opening/2:
        raise ValueError('arch bay must leave a positive opening and spring height')
    for side in (-1, 1):
        mesh.box((x+side*(width-pier)/2, y+height/2, z), (pier,height,depth),color)
    half = opening/2
    spring = height-(opening*.866 if pointed else half)-.55
    xx = np.linspace(-half, half, (20, 12, 8)[lod]+1)
    def upper(u):
        rise = math.sqrt(max(0, opening*opening-(abs(u)+half)**2)) if pointed else math.sqrt(max(0,half*half-u*u))
        return y+spring+rise
    for i, (a, b) in enumerate(zip(xx, xx[1:])):
        ya, yb = upper(a), upper(b)
        c = tuple(min(255,max(0,v+(-3,0,3)[i%3])) for v in color)
        for side in (-1, 1):
            face(mesh, [(x+a,ya,z+side*depth/2),(x+b,yb,z+side*depth/2),
                (x+b,y+height,z+side*depth/2),(x+a,y+height,z+side*depth/2)],c,11,(0,0,side))
        face(mesh,[(x+a,ya,z-depth/2),(x+a,ya,z+depth/2),
            (x+b,yb,z+depth/2),(x+b,yb,z-depth/2)],color,11,(0,-1,0))
    mesh.box((x,y+height+.12,z),(width,.24,depth+.14),color,17)


def window(mesh, x, y, z, width, height, lod=0, color=TRIM, pointed=False, panes=True):
    arch(mesh, (x,y,z), width,height,.16,color,pointed,(20,12,8)[lod],SHADOW,17)
    if panes:
        # Muted stained or reflective glass stays distinct from its stone reveal.
        for dx in (-width/4, width/4):
            mesh.box((x+dx,y+height*.36,z-.035),(.07,height*.64,.09),color,17)
        for yy in (height*.28,height*.52):
            mesh.box((x,y+yy,z-.04),(width*.90,.08,.10),color,17)


def cross(mesh, center, height, width, radius, color=GOLD):
    x,y,z=center
    mesh.box((x,y+height/2,z),(radius,height,radius),color,16)
    mesh.box((x,y+height*.70,z),(width,radius,radius),color,16)


def stone_panel(mesh, x, y, z, width, height, lod=0, color=STONE):
    """Subtle course coloration on a -z masonry face without random noise."""
    cols=max(1,int(width/(3.0 if lod==0 else 5.0)))
    rows=max(1,int(height/(1.2 if lod==0 else 2.4)))
    for row in range(rows):
        shift=.5 if row%2 else 0
        for col in range(-1,cols+1):
            a=max(-width/2,(col+shift)*width/cols-width/2)
            b=min(width/2,(col+shift+1)*width/cols-width/2)
            if b-a <= .04:
                continue
            yy=y+row*height/rows
            shade=(-3,0,2,1,-1)[(row*7+col*3)%5]
            c=tuple(max(0,min(255,v+shade)) for v in color)
            face(mesh,[(x+a+.008,yy+.008,z),(x+b-.008,yy+.008,z),
                (x+b-.008,yy+height/rows-.008,z),(x+a+.008,yy+height/rows-.008,z)],c,11,(0,0,-1))
