"""Original metric gates and arches; x east, y up, z north before heading.

Reference files distinguish published dimensions from photographic estimates.
All passages are actual apertures and every load-bearing foot extends below y=0.
"""
import math
from pathlib import Path

import numpy as np
from city_catalog.common import Mesh
from heritage_catalog.geometry import face, lathe, turn, column, stone_panel
from heritage_catalog.temples import _frustum, _hip_roof
from heritage_catalog.materials import (
    STONE_WALL, WEATHERED_LIMESTONE, VEINED_MARBLE, TIMBER_WALL,
    TILE_ROOF, PLASTER_WALL, SCULPTURE_METAL,
)

DEPENDENCIES = [Path(__file__).resolve().parents[1]/'heritage_catalog'/p
                for p in ('geometry.py', 'temples.py', 'materials.py')]
LIME = (203, 195, 174)
IVORY = (222, 218, 201)
GRANITE = (174, 176, 170)
RED = (174, 47, 32)
GREEN = (47, 111, 89)
BLUE = (51, 88, 111)
GOLD = (194, 156, 75)
DARK = (49, 52, 49)


def _foot(m, x, z, w, d, color=GRANITE, top=.12):
    m.box((x, (top-.6)/2, z), (w, top+.6, d), color, STONE_WALL)


def _portal(m, x, width, opening, spring, top, depth, lod, color=LIME, z=0):
    """Closed masonry volume above a semicircle; the opening stays empty."""
    pier = (width-opening)/2
    for s in (-1, 1):
        xx=x+s*(opening+ pier)/2
        m.box((xx, top/2, z), (pier, top, depth), color, WEATHERED_LIMESTONE)
        _foot(m,xx,z,pier+.12,depth+.16,color)
        if lod<2:
            for side in (-1,1):
                panel=Mesh();stone_panel(panel,xx,0,z-depth/2-.008,pier,top,lod,color)
                if side==1: panel=turn(panel,180,x*2,0,z*2)
                m.merge(panel)
    r=opening/2
    xx=np.linspace(-r,r,(24,16,10)[lod]+1)
    for a,b in zip(xx,xx[1:]):
        ya=spring+math.sqrt(max(0,r*r-a*a));yb=spring+math.sqrt(max(0,r*r-b*b))
        for side in (-1,1):
            face(m,[(x+a,ya,z+side*depth/2),(x+b,yb,z+side*depth/2),
                    (x+b,top,z+side*depth/2),(x+a,top,z+side*depth/2)],color,WEATHERED_LIMESTONE,(0,0,side))
        face(m,[(x+a,ya,z-depth/2),(x+b,yb,z-depth/2),(x+b,yb,z+depth/2),
                (x+a,ya,z+depth/2)],color,WEATHERED_LIMESTONE,(0,-1,0))
    m.box((x,top+.06,z),(width,.12,depth),color,WEATHERED_LIMESTONE)
    if lod<2:
        # Individually tinted radial voussoirs reveal how the arch bears load.
        n=(20,12)[lod]
        for i in range(n):
            a=math.pi*i/n+.008;b=math.pi*(i+1)/n-.008
            tint=tuple(v+(-5,0,4)[i%3] for v in color)
            for side in (-1,1):
                ring=[(x+rr*math.cos(t),spring+rr*math.sin(t),z+side*(depth/2+.025))
                      for rr,t in [(r,a),(r,b),(r+.35,b),(r+.35,a)]]
                face(m,ring,tint,WEATHERED_LIMESTONE,(0,0,side))


def _figure(m,x,y,z,h,lod,color=IVORY):
    # Faceted drapery and limbs communicate sculpture without borrowed artwork.
    _frustum(m,x,z,y,h*.63,h*.36,h*.25,h*.22,h*.2,color,VEINED_MARBLE)
    m.ellipsoid((x,y+h*.82,z),(h*.14,h*.18,h*.14),color,VEINED_MARBLE,(10,8,6)[lod],(6,4,3)[lod])
    for s in (-1,1):
        m.beam((x+s*h*.12,y+h*.57,z),(x+s*h*.3,y+h*.36,z-.03),h*.055,color,VEINED_MARBLE,(8,6,4)[lod])
    if lod==0:
        for j in (-1,0,1):
            m.beam((x+j*h*.065,y+h*.51,z-h*.105),(x+j*h*.09,y+.1,z-h*.13),h*.018,(color[0]-14,color[1]-14,color[2]-13),VEINED_MARBLE,4)


def _relief_strip(m,x,y,z,w,h,lod,color=IVORY):
    m.box((x,y,z),(w,h,.15),tuple(v-18 for v in color),WEATHERED_LIMESTONE)
    if lod<2:
        count=max(2,int(w/(.65 if lod==0 else 1.1)))
        for j in range(count):
            _figure(m,x+(j-(count-1)/2)*w/count,y-h*.43,z-.12,h*.8,lod,color)


def _sanchi(lod):
    m=Mesh();sand=(177,153,113);light=(191,170,132)
    for s in (-1,1):
        x=s*2.35;_foot(m,x,0,1.5,1.5,sand)
        m.box((x,3.25,0),(1.04,6.5,1.04),sand,WEATHERED_LIMESTONE)
        for y,w,h in [(0.3,1.3,.6),(5.8,1.23,.26),(6.48,1.4,.36)]:
            m.box((x,y,0),(w,h,1.25),light,WEATHERED_LIMESTONE)
        # Four small elephant-like capital masses carry the lowest lintel.
        for dx in (-.37,.37):
            for z in (-.34,.34):
                m.ellipsoid((x+dx,6.91,z),(.3,.31,.45),light,WEATHERED_LIMESTONE,(10,8,6)[lod],4)
                m.beam((x+dx,6.85,z-.25),(x+dx,6.48,z-.48),.095,sand,WEATHERED_LIMESTONE,6)
        for side in (-1,1):
            if lod<2:
                for row in range(6):
                    _relief_strip(m,x,.65+row*.77,side*.56,.85,.6,lod,light)
    for level,y in enumerate((7.22,8.48,9.69)):
        w=9.2-level*.25
        # Shallow upswept ends, separated by real daylight between lintels.
        points=[(-w/2,y+.25),(-w*.38,y),(w*.38,y),(w/2,y+.25)]
        for (x0,y0),(x1,y1) in zip(points,points[1:]):
            face(m,[(x0,y0,-.43),(x1,y1,-.43),(x1,y1+.64,-.43),(x0,y0+.64,-.43)],light,WEATHERED_LIMESTONE,(0,0,-1))
            face(m,[(x0,y0,.43),(x1,y1,.43),(x1,y1+.64,.43),(x0,y0+.64,.43)],sand,WEATHERED_LIMESTONE,(0,0,1))
            face(m,[(x0,y0+.64,-.43),(x1,y1+.64,-.43),(x1,y1+.64,.43),(x0,y0+.64,.43)],light,WEATHERED_LIMESTONE)
            face(m,[(x0,y0,-.43),(x1,y1,-.43),(x1,y1,.43),(x0,y0,.43)],sand,WEATHERED_LIMESTONE,(0,-1,0))
        for x,yend,normal in [(-w/2,y+.25,(-1,0,0)),(w/2,y+.25,(1,0,0))]:
            face(m,[(x,yend,-.43),(x,yend,.43),(x,yend+.64,.43),(x,yend+.64,-.43)],sand,WEATHERED_LIMESTONE,normal)
        for x in (-2.35,0,2.35):
            if level<2:m.box((x,y+1.0,0),(.34,.77,.46),sand,WEATHERED_LIMESTONE)
        if lod<2:
            for x in np.linspace(-w*.34,w*.34,(13,7)[lod]):
                _figure(m,x,y+.07,-.48,.48,lod,sand)
    # Crown symbols remain below the attested approximate 10.6 m envelope.
    for x in (-2.35,2.35):m.ellipsoid((x,10.42,0),(.28,.18,.18),light,WEATHERED_LIMESTONE,8,4)
    return m


def _sungnyemun(lod):
    m=Mesh();_portal(m,0,28,4.8,3.0,7.1,12.0,lod,(177,174,158))
    for s in (-1,1):
        m.box((s*17,3.0,0),(6,6,4.6),GRANITE,STONE_WALL);_foot(m,s*17,0,6,4.6)
    for y,w,d,h in [(7.1,22,9.2,4.1),(13.0,18.9,8.0,3.5)]:
        m.box((0,y+.1,0),(w,.2,d),(128,91,65),TIMBER_WALL)
        for side in (-1,1):
            for x in np.linspace(-w/2+.5,w/2-.5,6):
                lathe(m,(x,side*(d/2-.4)),[(y,.29),(y+h,.24)],(155,61,44),TIMBER_WALL,(12,8,6)[lod])
            for yy,c in [(y+h-.7,GREEN),(y+h-.38,RED),(y+h-.14,BLUE)]:
                m.box((0,yy,side*d/2),(w,.25,.4),c,TIMBER_WALL)
            if lod<2:
                for x in np.linspace(-w/2+.4,w/2-.4,(31,17)[lod]):
                    m.box((x,y+h-.3,side*(d/2+.28)),(.18,.35,.63),GOLD if round(x*3)%3==0 else GREEN,TIMBER_WALL)
        _hip_roof(m,0,0,w+3.2,d+3.4,y+h,2.7,(69,76,76),(139,143,132),lod)
        if lod<2:
            for x in np.linspace(-(w+2)/2,(w+2)/2,(45,23)[lod]):
                for side in (-1,1):
                    m.beam((x,y+h+.62,side*(d+3.3)/2),(x*.7,y+h+1.55,side*d*.32),.055,(99,103,97),TILE_ROOF,4)
    m.box((0,15.1,-4.26),(1.0,2.15,.18),(38,35,29),TIMBER_WALL)
    if lod<2:
        for y in (14.5,15.1,15.7):m.box((0,y,-4.38),(.49,.24,.05),GOLD,TIMBER_WALL)
    return m


def _lempuyang(lod,components=False):
    parts=[];c=(212,211,195);shadow=(150,155,145)
    # Two mirrored halves have straight inner edges and stepped outer silhouettes.
    for s in (-1,1):
        m=Mesh();inner=s*1.35
        for y,h,w,d in [(-.6,1.0,3.0,3.2),(.4,.35,3.25,3.5),(.75,1.4,2.5,2.7),
                           (2.15,.32,2.8,3.0),(2.47,2.0,2.15,2.4),(4.47,.33,2.4,2.7),
                           (4.8,1.65,1.8,2.05),(6.45,.28,2.03,2.3),(6.73,1.3,1.4,1.7),
                           (8.03,.28,1.63,1.95),(8.31,1.0,1.03,1.35),(9.31,.3,1.2,1.6),
                           (9.61,1.0,.7,1.0),(10.61,.26,.85,1.22),(10.87,.85,.43,.68)]:
            x=inner+s*w/2;m.box((x,y+h/2,0),(w,h,d),c if h>.35 else shadow,STONE_WALL)
            if lod<2 and h>.6 and y>0:
                for zz in (-d/2-.03,d/2+.03):
                    for j in range(3):
                        xx=x+(j-1)*w*.25
                        _frustum(m,xx,zz,y+h*.25,h*.45,w*.18,.20,w*.08,.14,(193,197,183),STONE_WALL)
        if lod<2:
            for y,w,d in [(2.5,2.15,2.4),(4.9,1.8,2.05),(6.8,1.4,1.7),(8.4,1.03,1.35)]:
                for side in (-1,1):
                    m.beam((inner+s*w,y,side*d*.4),(inner+s*(w+.45),y+.65,side*(d*.4+.25)),.13,c,STONE_WALL,6)
        parts.append(m)
        m=Mesh()
        _foot(m,s*4.4,-1.75,1.1,1.0,shadow)
        _figure(m,s*4.4,.12,-1.75,1.6,lod,shadow)
        parts.append(m)
    if components:return parts
    m=Mesh()
    for part in parts:m.merge(part)
    return m


def _ming(lod):
    m=Mesh();stone=(207,205,187);trim=(221,217,198)
    xs=(-12.9,-8.15,-3.4,3.4,8.15,12.9)
    heights=(7.6,9.6,11.1,11.1,9.6,7.6)
    for x,h in zip(xs,heights):
        _foot(m,x,0,1.55,4.5,stone)
        m.box((x,1.0,0),(1.5,2,3.8),stone,VEINED_MARBLE)
        m.box((x,h/2,0),(.93,h,1.0),trim,VEINED_MARBLE)
        for z in (-1.36,1.36):
            m.ellipsoid((x,2.43,z),(.43,.42,.66),stone,VEINED_MARBLE,(10,8,6)[lod],4)
            m.ellipsoid((x,2.8,z-.35),(.26,.31,.26),trim,VEINED_MARBLE,8,4)
        for yy in (3.0,h-.85,h-.32):m.box((x,yy,0),(1.24,.22,1.36),stone,VEINED_MARBLE)
        _hip_roof(m,x,0,2.8,2.4,h-.2,1.2,stone,trim,lod,VEINED_MARBLE)
    for j,(a,b) in enumerate(zip(xs,xs[1:])):
        x=(a+b)/2;w=b-a;h=min(heights[j:j+2])-.15
        for yy,hh in [(h-2.0,.7),(h-.6,.7)]:
            m.box((x,yy,0),(w,hh,.76),stone,VEINED_MARBLE)
            if lod<2:_relief_strip(m,x,yy,-.43,w-.5,hh*.8,lod,trim)
        _hip_roof(m,x,0,w+1.55,3.1,h+.2,1.6,stone,trim,lod,VEINED_MARBLE)
    return m


def _swing(lod):
    m=Mesh();n=(20,14,10)[lod]
    for s in (-1,1):
        _foot(m,s*3.25,0,2.5,3.3,(207,205,192),.32)
        for z in (-.53,.53):
            lathe(m,(s*3.25,z),[(.32,.32),(17.8,.27),(20.45,.22)],RED,TIMBER_WALL,n)
            lathe(m,(s*3.25,z),[(.5,.4),(.82,.4),(.9,.31)],(138,36,27),TIMBER_WALL,n)
        m.box((s*3.25,17.8,0),(.6,.42,1.75),(139,34,24),TIMBER_WALL)
    # The carved crown curls upward at both ends; no modern swing seat is added.
    for z in (-.53,.53):
        for x0,x1 in zip(np.linspace(-5.6,5.6,17),np.linspace(-5.6,5.6,17)[1:]):
            y0=19.55+.055*x0*x0;y1=19.55+.055*x1*x1
            face(m,[(x0,y0,z-.16),(x1,y1,z-.16),(x1,y1+.6,z-.16),(x0,y0+.6,z-.16)],RED,TIMBER_WALL,(0,0,-1))
            face(m,[(x0,y0,z+.16),(x1,y1,z+.16),(x1,y1+.6,z+.16),(x0,y0+.6,z+.16)],(154,38,26),TIMBER_WALL,(0,0,1))
            face(m,[(x0,y0+.6,z-.16),(x1,y1+.6,z-.16),(x1,y1+.6,z+.16),(x0,y0+.6,z+.16)],RED,TIMBER_WALL)
            face(m,[(x0,y0,z-.16),(x1,y1,z-.16),(x1,y1,z+.16),(x0,y0,z+.16)],RED,TIMBER_WALL,(0,-1,0))
        for x in (-5.6,5.6):
            yend=19.55+.055*x*x
            face(m,[(x,yend,z-.16),(x,yend,z+.16),(x,yend+.6,z+.16),(x,yend+.6,z-.16)],RED,TIMBER_WALL,(x,0,0))
        if lod<2:
            for x in np.linspace(-4.8,4.8,(25,13)[lod]):
                m.box((x,19.88+.055*x*x,z-.2),(.13,.31,.08),(201,73,42),TIMBER_WALL)
    # Overall summit is 21.15 m; curvature and joint sizes are estimated.
    maximum=max(p[:,1].max() for p,_,_ in m.triangles)
    return m.transform([[1,0,0,0],[0,21.15/maximum,0,0],[0,0,1,0],[0,0,0,1]])


def _patuxai(lod):
    m=Mesh();concrete=(171,160,139);edge=(196,180,149)
    # Four portals surround a cross-shaped ground-level passage.
    for angle in (0,90,180,270):
        side=Mesh();_portal(side,0,24,10,10.2,22,5.2,lod,concrete,z=-9.4)
        side.box((0,23.4,-9.4),(25.2,2.8,5.5),edge,PLASTER_WALL)
        for x in (-8,8):
            _relief_strip(side,x,16.5,-12.12,3.0,3.0,lod,edge)
        m.merge(turn(side,angle))
    m.box((0,25.4,0),(24,2.8,24),concrete,PLASTER_WALL)
    for x in (-9.3,9.3):
        for z in (-9.3,9.3):
            m.box((x,29,z),(4.4,5,4.4),edge,PLASTER_WALL)
            _hip_roof(m,x,z,6.4,6.4,31,2.2,(97,111,99),edge,lod)
            lathe(m,(x,z),[(33.5,1.5),(35,1.15),(36.6,.65),(39,.03)],edge,PLASTER_WALL,(16,12,8)[lod])
            for y,r in [(34.1,1.48),(34.9,1.25),(35.7,1.02),(36.5,.78),(37.3,.54)]:
                lathe(m,(x,z),[(y-.09,r*.86),(y,r),(y+.16,r*.85)],edge,PLASTER_WALL,(16,12,8)[lod])
    for y,w,h in [(27,13.2,4),(31,11.4,3.2)]:
        m.box((0,y+h/2,0),(w,h,w),edge,PLASTER_WALL)
        for angle in (0,90,180,270):
            side=Mesh()
            for x in (-w*.29,0,w*.29):
                side.box((x,y+h*.45,-w/2-.025),(1.2,h*.64,.1),DARK,PLASTER_WALL)
            m.merge(turn(side,angle))
    _hip_roof(m,0,0,14.6,14.6,34.2,3.0,(91,109,98),edge,lod)
    lathe(m,(0,0),[(37.5,3.0),(39.5,2.2),(41.5,1.35),(43.1,.68),(45,.03)],edge,PLASTER_WALL,(24,16,10)[lod])
    for y,r in [(38.2,2.96),(39.3,2.6),(40.4,2.05),(41.5,1.55),(42.6,1.02),(43.6,.65)]:
        lathe(m,(0,0),[(y-.12,r*.85),(y,r),(y+.22,r*.85)],edge,PLASTER_WALL,(24,16,10)[lod])
    # Painted radial ceiling panels occupy the soffit above the crossing.
    if lod<2:
        for j in range((24,12)[lod]):
            a=math.tau*j/(24,12)[lod];b=math.tau*(j+1)/(24,12)[lod]
            face(m,[(0,23.975,0),(4.5*math.cos(a),23.975,4.5*math.sin(a)),(4.5*math.cos(b),23.975,4.5*math.sin(b))],(BLUE,GOLD,GREEN,(170,75,55))[j%4],PLASTER_WALL,(0,-1,0))
    return m


def _titus(lod):
    m=Mesh();_portal(m,0,13.5,5.36,5.62,10.7,4.75,lod)
    for y,h,w in [(10.8,.4,13.7),(11.25,.5,13.95),(13.0,3.0,13.5),(14.7,.42,13.9),(15.1,.6,14.05)]:
        m.box((0,y,0),(w,h,4.9),IVORY,WEATHERED_LIMESTONE)
    for s in (-1,1):
        for x in (-5.1,-3.25,3.25,5.1):column(m,x,s*2.52,.5,9.7,.35,IVORY,lod)
        _relief_strip(m,0,10.65,s*2.52,10.2,.54,lod)
        m.box((0,13.1,s*2.48),(6.2,1.65,.14),(190,181,158),WEATHERED_LIMESTONE)
        if lod<2:
            for row in range(4):
                for x in np.linspace(-2.6,2.6,20-row*2):
                    m.box((x,13.6-row*.32,s*2.57),(.12,.16,.045),(154,146,127),WEATHERED_LIMESTONE)
    if lod<2:
        for s in (-1,1):
            inner=Mesh();_relief_strip(inner,0,3.2,-.1,3.3,1.65,lod,IVORY)
            m.merge(turn(inner,s*90,s*2.68,0,0))
    return m


def _constantine(lod):
    m=Mesh();_portal(m,0,11.8,6.5,8.05,15.5,7.4,lod,IVORY)
    for x in (-9.4,9.4):_portal(m,x,7.0,3.4,5.7,15.5,7.4,lod,IVORY)
    for y,h,w in [(15.6,.4,26.1),(16.05,.5,26.4),(18.35,4.1,25.9),(20.7,.6,26.2)]:
        m.box((0,y,0),(w,h,7.7),LIME,WEATHERED_LIMESTONE)
    for side in (-1,1):
        for x in (-11.5,-5.7,5.7,11.5):
            column(m,x,side*4.02,.5,14.0,.47,(199,180,129),lod)
            m.box((x,15.75,side*4.02),(1.5,.5,1.5),IVORY,VEINED_MARBLE)
            _figure(m,x,16.0,side*4.02,2.1,lod,(201,194,175))
        m.box((0,18.5,side*3.89),(8.8,2.5,.13),(184,177,157),WEATHERED_LIMESTONE)
        for x in (-8.4,8.4):
            _relief_strip(m,x,18.4,side*3.9,3.5,2.1,lod,IVORY)
            for dx in (-1.0,1.0):
                # Red porphyry framing and pale medallion relief are distinct.
                m.ellipsoid((x+dx,12.5,side*3.83),(.83,.83,.13),(140,99,83),VEINED_MARBLE,(20,12,8)[lod],6)
                m.ellipsoid((x+dx,12.5,side*3.94),(.68,.68,.11),IVORY,VEINED_MARBLE,(20,12,8)[lod],6)
                if lod<2:_figure(m,x+dx,12.0,side*4.03,.9,lod,LIME)
            _relief_strip(m,x,10.7,side*3.86,4.5,.65,lod,IVORY)
    return m


def _alcala(lod):
    m=Mesh();gray=(185,184,174)
    for x in (-8.0,0,8.0):_portal(m,x,8.0,4.5,5.9,12.2,4.6,lod,gray)
    for side in (-1,1):
        x=side*16.7
        for dx in (-2.5,2.5):
            m.box((x+dx,5.6,0),(2.4,11.2,4.6),gray,STONE_WALL);_foot(m,x+dx,0,2.4,4.6,gray)
        m.box((x,10.0,0),(7.4,4.4,4.6),gray,STONE_WALL)
    for y,h,w in [(12.3,.35,41),(12.85,.6,41.4),(13.65,1.0,40.6)]:
        m.box((0,y,0),(w,h,5.0),IVORY,STONE_WALL)
    m.box((0,15.35,0),(10.2,2.5,5.2),gray,STONE_WALL)
    # Central pediment and crest, with asymmetric decoration on the two fronts.
    for s in (-1,1):
        face(m,[(-5.7,16.6,s*2.75),(5.7,16.6,s*2.75),(0,18.0,s*2.75)],IVORY,STONE_WALL,(0,0,s))
        for x in (-18.6,-12.3,-4.9,4.9,12.3,18.6):column(m,x,s*2.6,.3,11.9,.48,IVORY,lod)
        if s<0:
            for x in (-11,-7,7,11):_figure(m,x,14.2,s*1.9,2.2,lod)
        else:
            for x in (-13,-8,8,13):
                m.ellipsoid((x,14.7,s*1.8),(.9,.55,.7),IVORY,VEINED_MARBLE,10,5)
                m.beam((x,14.5,s*1.8),(x+.5,16.1,s*1.8),.17,IVORY,STONE_WALL,6)
    m.ellipsoid((0,17.6,-2.4),(1.1,1.25,.3),IVORY,VEINED_MARBLE,(16,12,8)[lod],6)
    lathe(m,(0,-2.4),[(18.4,.7),(18.8,.6),(19.4,.03)],IVORY,VEINED_MARBLE,10)
    return m


def _augusta(lod):
    m=Mesh();_portal(m,0,24,8.3,9.1,22.6,8.0,lod,IVORY)
    for y,h,w in [(22.7,.45,24.6),(23.3,.55,25.1),(24.1,.85,24.4)]:
        m.box((0,y,0),(w,h,8.4),LIME,WEATHERED_LIMESTONE)
    for x in (-10.0,-7.2,-4.9,4.9,7.2,10.0):column(m,x,-4.4,1.0,14.5,.58,IVORY,lod)
    for x in (-9.2,-5.8,5.8,9.2):
        m.box((x,16.6,-4.2),(2.1,.4,2.1),IVORY,VEINED_MARBLE)
        _figure(m,x,16.8,-4.25,3.1,lod)
    m.ellipsoid((0,18.7,-4.16),(2.45,2.25,.32),LIME,VEINED_MARBLE,(24,16,10)[lod],8)
    m.ellipsoid((0,18.8,-4.5),(1.1,1.5,.2),IVORY,VEINED_MARBLE,(20,12,8)[lod],6)
    for x,h in [(-3.0,3.7),(0,5.65),(3.0,3.7)]:
        m.box((x,24.65,0),(2.5,.5,2.3),IVORY,VEINED_MARBLE)
        _figure(m,x,24.9,0,h,lod)
    # Clock is on Rua Augusta's north face; allegorical sculpture faces the square.
    m.ellipsoid((0,19.0,4.12),(2.0,2.0,.1),LIME,STONE_WALL,(32,20,12)[lod],6)
    m.ellipsoid((0,19.0,4.23),(1.65,1.65,.08),(230,225,205),STONE_WALL,(32,20,12)[lod],6)
    for j in range(12):
        a=j*math.tau/12
        m.box((1.38*math.sin(a),19+1.38*math.cos(a),4.33),(.10,.2,.035),DARK,SCULPTURE_METAL)
    m.beam((0,19,4.36),(.72,19.9,4.36),.045,DARK,SCULPTURE_METAL,6)
    m.beam((0,19,4.36),(-.25,20.23,4.36),.035,DARK,SCULPTURE_METAL,6)
    return m


_BUILDERS = {'sanchi-north-torana':_sanchi, 'sungnyemun-gate':_sungnyemun,
             'lempuyang-gates-of-heaven':_lempuyang, 'ming-tombs-stone-archway':_ming,
             'bangkok-giant-swing':_swing, 'patuxai':_patuxai, 'arch-of-titus':_titus,
             'arch-of-constantine':_constantine, 'puerta-de-alcala':_alcala,
             'rua-augusta-arch':_augusta}


def build(name,lod):
    if lod not in (0,1,2): raise ValueError('LOD must be 0, 1 or 2')
    m=_BUILDERS[name](lod)
    if name=='ming-tombs-stone-archway':
        m.transform([[28.86/28.6,0,0,0],[0,14/13.67,0,0],[0,0,1,0],[0,0,0,1]])
    elif name=='patuxai':
        m.transform([[1,0,0,0],[0,49/45,0,0],[0,0,1,0],[0,0,0,1]])
    elif name=='rua-augusta-arch':
        m.transform([[.82,0,0,0],[0,1,0,0],[0,0,1.75,0],[0,0,0,1]])
    # Imported geometry primitives use legacy wall and trim ids.
    mapping={11:STONE_WALL,17:VEINED_MARBLE,12:TILE_ROOF,8:STONE_WALL}
    if name=='ming-tombs-stone-archway':mapping[SCULPTURE_METAL]=VEINED_MARBLE
    elif name=='sungnyemun-gate':mapping.update({SCULPTURE_METAL:TILE_ROOF,WEATHERED_LIMESTONE:STONE_WALL})
    elif name=='patuxai':mapping.update({SCULPTURE_METAL:PLASTER_WALL,WEATHERED_LIMESTONE:PLASTER_WALL})
    elif name=='puerta-de-alcala':mapping[WEATHERED_LIMESTONE]=STONE_WALL
    m.triangles=[(p,c,mapping.get(mat,mat)) for p,c,mat in m.triangles]
    return m


def build_components(name,lod):
    """Unshifted rigid groups, preserving triangle order and material across LODs.

    Lempuyang's split towers have no connecting lintel. The stable group order
    is negative-x tower, its guardian, positive-x tower, then its guardian.
    Every other gateway retains the shared datum of its connected structure.
    """
    if lod not in (0,1,2):raise ValueError('LOD must be 0, 1 or 2')
    if name=='lempuyang-gates-of-heaven':return _lempuyang(lod,components=True)
    return [build(name,lod)]


def _entry(name, city, lat, lon, heading, half, height, references, description, placement, dimension_note, orientation_note):
    x,z=half
    polygon=[[-x,-z],[x,-z],[x,z],[-x,z]]
    return {'name':name,'city':city,'lat':lat,'lon':lon,'heading':heading%360,'base':'terrain',
            'suppress_buildings':True,'suppress_roads':False,'suppress_trees':True,
            'references':references,
            'attribution':'Original Low Poly Earth geometry. Placement evidence © OpenStreetMap contributors (ODbL 1.0). No reference images are embedded.',
            'description':description,'source_exclusion_polygons':[polygon],
            'road_exclusion_polygons':[],'suppress_bbox':[[-x,-z],[x,z]],
            'max_source_building_span_m':max(half)*3,
            'passage_probe':{'x':0,'y':2.0,'z_range':[-z-1,z+1]},
            'audit':{'survey_file':f'tools/monument_catalog/references/gateways/{name}-survey.json',
                     'placement_file':f'tools/monument_catalog/references/gateways/{placement}-placement.json',
                     'architectural_dimensions_m':[x*2,height,z*2],
                     'height_authority':dimension_note,'orientation_note':orientation_note,
                     'grounding_note':'The connected gateway stays rigid above surveyed unmodified terrain. Only its separate masonry footings extend downward to meet local support elevations; the through-passage has no infill slab and no terrain reshaping.'},
            'capture':{'distance':max(x*3,height*2.2),'azimuth':210,'elevation':23,'target_height':height*.4,
                       'time_utc':'2026-03-20T10:00:00Z'}}


ENTRIES = [
    _entry('sanchi-north-torana','Sanchi',23.479577,77.739645,10,[4.8,1.1],10.6,
           ['https://www.incredibleindia.gov.in/en/madhya-pradesh/bhopal/sanchi-stupa',
            'https://imp-art.org/articles/gateways-at-sanchi-stupa-i/',
            'https://www.openstreetmap.org/node/1124758825'],
           'Three separated sandstone lintels with upswept ends, narrative relief registers, carved square piers and elephant capitals.',
           'sanchi','Published approximate torana height 10.6 m. Width, depth, relief and capital dimensions are proportional authoring estimates.',
           'Mapped northern gate node; approximately 10 degrees clockwise from north, following the gate-to-stupa radial axis.'),
    _entry('sungnyemun-gate','Seoul',37.55998625,126.9753604,42,[20.2,6.5],20.12,
           ['https://english.visitseoul.net/attractions/sungnyemun-gate-namdaemun-gate/ENP022888',
            'https://www.openstreetmap.org/relation/10647050'],
           'Granite vaulted base, flanking wall stubs, two timber pavilion stories, charcoal tiled hip roofs and layered red-green-blue dancheong brackets.',
           'sungnyemun','20.12 m model height is a photographic estimate; official source confirms two-story timber superstructure and arched masonry base.',
           'Gate footprint center and approximately 42-degree clockwise facade axis derived from OSM relation outer ways; wall extensions are abbreviated.'),
    _entry('lempuyang-gates-of-heaven','Karangasem',-8.3913779,115.6306841,-69,[5.1,2.5],11.72,
           ['https://www.indonesia.travel/my/en/travel-ideas/pilgrimage/pura-lempuyang/',
            'https://www.openstreetmap.org/node/6080306159',
            'https://www.openstreetmap.org/way/647449208'],
           'White stone candi bentar with straight split faces, successively receding carved tiers, darker stone moldings and grounded guardians.',
           'lempuyang','11.72 m split-gate envelope, steps and ornament are photographic estimates; temple summit elevation is not used as gate height.',
           'Mapped Candi Bentar node; passage bearing follows adjoining OSM staircase (approximately west-northwest toward Mount Agung).'),
    _entry('ming-tombs-stone-archway','Beijing',40.23824805,116.2101385,18.6,[14.6,2.5],14.0,
           ['https://english.visitbeijing.com.cn/article/4KOdhGlm48J',
            'https://english.visitbeijing.com.cn/article/47OLcYDde76',
            'https://www.openstreetmap.org/way/677547734'],
           'Five open bays and six white-marble pillars, eleven stone rooflets, relief crossbeams and twelve lion-like pedestal sculptures.',
           'ming','Official width 28.86 m and height 14 m. Roof, animal, relief and support dimensions are estimated within that envelope.',
           'Center and 18.6-degree clockwise heading measured from the mapped stone paifang footprint, not the distant Ming Tombs visitor-center pin.'),
    _entry('bangkok-giant-swing','Bangkok',13.7518086,100.5012782,-6,[5.8,1.9],21.15,
           ['https://en.wikipedia.org/wiki/Giant_Swing',
            'https://publicspace.bangkok.go.th/Spaces/Info/A5123802-E88B-45A2-AEAE-DF0BC0A9FFE9',
            'https://www.openstreetmap.org/way/178353988'],
           'Paired tall red teak uprights with dark red collars, twin curved carved crown beams and pale support plinths.',
           'swing','Published height 21.15 m; span, joint dimensions and crown curvature are proportional estimates.',
           'Mapped circular memorial island center; estimated east-west beam axis, six degrees anticlockwise, following the Wat Suthat forecourt.'),
    _entry('patuxai','Vientiane',17.97063365,102.6186144,44.8,[12.8,12.8],49.0,
           ['https://www.visitlaos.org/explore-laos/vientiane/',
            'https://en.wikipedia.org/wiki/Patuxai',
            'https://citywalls.guru/vientiane/things-to-do/patuxay-victory-monument/',
            'https://www.openstreetmap.org/way/41566774'],
           'Four-way vaulted crossing, warm concrete and stucco relief, painted ceiling, upper galleries and five Lao roof towers.',
           'patuxai','49 m secondary-source height; upper stages are photo-proportioned. OSM height=174 m is rejected because it is elevation above sea level.',
           'Mapped square footprint center and 44.8-degree orientation align the crossing with Lane Xang Avenue; photographs determine the five-tower arrangement.'),
    _entry('arch-of-titus','Rome',41.8906722,12.4885842,-67.3,[7.1,3.1],15.4,
           ['https://colosseo.it/en/restauri/arch-of-titus/',
            'https://en.wikipedia.org/wiki/Arch_of_Titus',
            'https://www.openstreetmap.org/way/23913953'],
           'Single travertine-and-marble arch with articulated voussoirs, engaged columns, attic inscription registers and passage relief panels.',
           'titus','Published main body 13.5 x 15.4 x 4.75 m; clear opening 5.36 x 8.3 m. Projecting cornice and columns extend the envelope.',
           'Center and minus 67.3-degree heading from OSM arch footprint; passage follows the Via Sacra.'),
    _entry('arch-of-constantine','Rome',41.88976225,12.4906668,13,[13.4,4.95],21,
           ['https://colosseo.it/en/restauri/the-maintenance-of-the-arch-of-constantine/',
            'https://en.wikipedia.org/wiki/Arch_of_Constantine',
            'https://www.openstreetmap.org/way/23590989'],
           'Three distinct vaulted apertures, yellow stone columns, pale captive statues, porphyry-framed medallions and separate reused relief registers.',
           'constantine','Published body width 25.9 m, height 21 m and depth 7.4 m; carving and freestanding-column projections are proportional estimates.',
           'Mapped footprint center and approximately 13-degree clockwise heading; the nearby Colosseum center is not used as placement.'),
    _entry('puerta-de-alcala','Madrid',40.41998675,-3.6887244,260.4,[21,3.5],19.4,
           ['https://www.esmadrid.com/informacion-turistica/puerta-de-alcala',
            'https://www.openstreetmap.org/way/174805987'],
           'Three semicircular arches and two rectangular passages, granite body, pale pilasters, central pediment and differing sculpture on inner and outer facades.',
           'alcala','41.4 m cornice width and 19.4 m summit are approximate modeled dimensions; mapped footprint is approximately 43 m wide including projections.',
           'Mapped monument center, 80.4-degree long-axis bearing plus 180 degrees to put the more ornate outer facade toward the east.'),
    _entry('rua-augusta-arch','Lisbon',38.70843845,-9.13681735,-17.5,[10.5,9.4],30.55,
           ['https://www.visitportugal.com/en/destinos/lisboa/315757',
            'https://en.wikipedia.org/wiki/Rua_Augusta_Arch',
            'https://www.openstreetmap.org/way/41683195'],
           'Deep walkable barrel passage, six engaged columns, historical figures, sculpted coat of arms, allegorical crown group and a clock on the street-facing side.',
           'augusta','Approximate 30.55 m overall sculptural height; width and depth fitted to mapped arch footprint, with estimated detail dimensions. Conflicting OSM height tags are not used.',
           'Mapped monument center; minus 17.5-degree heading follows the facade facing Praca do Comercio; clock is on the opposite Rua Augusta face.'),
]
