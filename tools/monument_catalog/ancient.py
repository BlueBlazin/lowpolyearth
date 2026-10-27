"""Metric ancient monuments, with present-day ruins and original faceted sculpture.

Local +z is north before registry heading; sculpture faces -z. Evidence separates
published overall dimensions from estimated small carvings and stone silhouettes.
"""
import math

import numpy as np

from city_catalog.common import Mesh
from heritage_catalog.geometry import face, lathe, turn, tube
from heritage_catalog.materials import (
    STONE_WALL, WEATHERED_LIMESTONE, VEINED_MARBLE, SCULPTURE_METAL,
)

SARSEN = (151, 149, 134)
BLUESTONE = (102, 112, 112)
GNEISS = (139, 139, 127)
SANDSTONE = (150, 126, 107)
TUFF = (115, 105, 86)
BASALT = (66, 69, 65)
MARBLE = (202, 195, 178)


def _tint(color, seed, spread=12):
    shift = ((seed * 37 + 19) % (2 * spread + 1)) - spread
    return tuple(max(0, min(255, v + shift)) for v in color)


def _weathered_face(m, points, color, normal, lod, seed, material=WEATHERED_LIMESTONE):
    """Subdivide actual stone faces into muted mineral and lichen mottling."""
    p = np.asarray(points, float)
    n = (4, 2, 1)[lod]
    if len(p) != 4:
        face(m, p, _tint(color, seed), material, normal)
        return
    for i in range(n):
        for j in range(n):
            def at(u, v):
                return ((1-u)*(1-v)*p[0] + u*(1-v)*p[1]
                        + u*v*p[2] + (1-u)*v*p[3])
            k = seed + 13*i + 31*j
            c = _tint(color, k, 10)
            if k % 11 == 0:
                c = tuple(round(a*.78+b*.22) for a, b in zip(c, (162, 163, 128)))
            face(m, [at(i/n,j/n), at((i+1)/n,j/n),
                     at((i+1)/n,(j+1)/n), at(i/n,(j+1)/n)], c, material, normal)


def _stone(m, x, z, h, w, d, lod, seed, color=SARSEN, y=-.45, angle=0, pointed=False):
    # All rings retain an eight-sided battered silhouette at every LOD.
    ring = [(-.38,-.5),(.34,-.5),(.5,-.30),(.47,.32),(.29,.5),(-.33,.47),(-.5,.24),(-.49,-.30)]
    ys = [y, y+(h-y)*.20, y+(h-y)*.72, h]
    sizes = [1.0, 1.02, .92, .50 if pointed else .87]
    rings = []
    for j,(yy,s) in enumerate(zip(ys,sizes)):
        rings.append([(xx*w*s+.055*w*math.sin(seed+j),
                       yy-(.07*(h-y)*((seed+k)%3)/2 if j==3 else 0),
                       zz*d*s+.06*d*math.cos(seed+2*j)) for k,(xx,zz) in enumerate(ring)])
    sub = Mesh()
    for level in range(3):
        for k in range(8):
            q=(k+1)%8
            pts=[rings[level][k], rings[level+1][k], rings[level+1][q], rings[level][q]]
            mid=np.asarray(pts).mean(0)
            _weathered_face(sub,pts,_tint(color,seed,8),(mid[0],0,mid[2]),lod,seed+17*k+41*level)
    face(sub,rings[0],_tint(color,seed),WEATHERED_LIMESTONE,(0,-1,0))
    face(sub,rings[-1],_tint(color,seed+4),WEATHERED_LIMESTONE,(0,1,0))
    m.merge(turn(sub,angle,x=x,z=z))


# Petrie numbers preserve the existing gaps, rather than reconstructing 30 uprights.
STONEHENGE_UPRIGHTS = (1,2,3,4,5,6,7,10,11,16,21,22,23,27,28,29,30)
STONEHENGE_LINTELS = ((29,30),(30,1),(1,2),(4,5),(6,7),(21,22))


class _ComponentMesh(Mesh):
    """Track top-level stone merges, including following tenon geometry."""
    def __init__(self):
        super().__init__()
        self.starts=[]

    def merge(self,other):
        self.starts.append(len(self.triangles))
        super().merge(other)

    def components(self):
        result=[]
        for start,end in zip(self.starts,self.starts[1:]+[len(self.triangles)]):
            part=Mesh()
            part.triangles=self.triangles[start:end]
            result.append(part)
        return result


def _stonehenge(lod,components=False):
    m=_ComponentMesh() if components else Mesh()
    def position(number):
        a=math.radians((number-.5)*12)
        return 14.8*math.sin(a),14.8*math.cos(a)
    for number in STONEHENGE_UPRIGHTS:
        x,z=position(number)
        h=2.4 if number==11 else 4.05
        _stone(m,x,z,h,2.0+(number%3)*.10,1.2,lod,number,angle=(number-.5)*12)
        if lod<2 and number!=11:
            lathe(m,(x,z),[(3.93,.28),(4.22,.23)],_tint(SARSEN,number),WEATHERED_LIMESTONE,8)
    for a,b in STONEHENGE_LINTELS:
        pa=np.asarray(position(a));pb=np.asarray(position(b));mid=(pa+pb)/2
        ang=math.degrees(math.atan2(-(pb-pa)[1],(pb-pa)[0]))
        _stone(m,*mid,4.92,3.50,1.15,lod,a+100,y=4.0,angle=ang)
    for number in (8,9,12,14,15,19,25,26):
        x,z=position(number)
        _stone(m,x*.92,z*.92,.55,3.5,1.7,lod,number,angle=number*19)
    # Three standing trilithons and the surviving tall upright of the great one.
    for x,z,ang,height,seed in [(6.8,3.2,20,5.9,51),(5.6,-4.6,53,6.5,53),(-5.6,-4.6,-53,6.5,57)]:
        sub=Mesh()
        for sign in (-1,1):
            _stone(sub,sign*1.42,0,height,2.05,1.45,lod,seed+(sign+1)//2)
        _stone(sub,0,0,height+.95,4.95,1.55,lod,seed+100,y=height-.05)
        m.merge(turn(sub,ang,x=x,z=z))
    _stone(m,-1.2,-8.1,6.9,2.3,1.7,lod,56,angle=-6)
    _stone(m,1.1,-4.6,.80,6.3,2.2,lod,55,angle=10)
    _stone(m,-6.8,2.8,3.2,2.0,1.5,lod,60,angle=-22)
    _stone(m,-5.4,5.2,.75,5.4,1.9,lod,59,angle=61)
    _stone(m,-.2,-3.1,.18,4.8,1.0,lod,80,(126,133,118),angle=6)
    # Irregular surviving bluestones: upright, stump and recumbent states.
    for j,a in enumerate((12,34,63,90,112,134,162,189,219,246,275,306,329)):
        r=10.8;a=math.radians(a)
        _stone(m,r*math.sin(a),r*math.cos(a),(.65,1.65,2.05,1.12)[j%4],.9,.7,lod,31+j,BLUESTONE,pointed=True)
    for j,(x,z) in enumerate([(-3,1),(-3.3,-1.6),(-2.8,-4),(0,-5.5),(2.8,-4),(3.3,-1.6),(3,1)]):
        _stone(m,x,z,(1.3,2.1,.6)[j%3],.75,.65,lod,61+j,BLUESTONE)
    # Heel Stone remains at the northeast avenue, outside the central setting.
    _stone(m,-2.0,77,4.7,2.7,2.1,lod,96,angle=12,pointed=True)
    _stone(m,.5,30,.38,6.2,1.8,lod,95,(155,133,112),angle=10)
    if not components:
        return m
    parts=m.components()
    # Lintels and their touching uprights must translate as one rigid group.
    parent={n:n for n in STONEHENGE_UPRIGHTS}
    def root(n):
        while parent[n]!=n:
            n=parent[n]
        return n
    for a,b in STONEHENGE_LINTELS:
        parent[root(b)]=root(a)
    groups={}
    for number,part in zip(STONEHENGE_UPRIGHTS,parts):
        groups.setdefault(root(number),Mesh()).merge(part)
    offset=len(STONEHENGE_UPRIGHTS)
    for i,(a,b) in enumerate(STONEHENGE_LINTELS):
        assert root(a)==root(b)
        groups[root(a)].merge(parts[offset+i])
    return list(groups.values())+parts[offset+len(STONEHENGE_LINTELS):]


MOAI_HEIGHTS = (5.6,6.4,5.9,6.7,6.0,7.1,6.3,8.3,6.2,6.6,5.8,6.4,6.1,6.5,5.7)


def _profile_solid(m, sections, lod, color, seed):
    """Eight-sided horizontal sections keep the broad front and projecting jaw."""
    rings=[]
    for y,w,front,back in sections:
        rings.append([(-w*.40,y,front),(w*.40,y,front),(w*.50,y,front*.75),
                      (w*.5,y,back*.7),(w*.32,y,back),(-w*.32,y,back),
                      (-w*.5,y,back*.7),(-w*.5,y,front*.75)])
    for k in range(len(rings)-1):
        for j in range(8):
            q=(j+1)%8;pts=[rings[k][j],rings[k+1][j],rings[k+1][q],rings[k][q]]
            mid=np.asarray(pts).mean(0)
            _weathered_face(m,pts,color,(mid[0],0,mid[2]),lod,seed+11*j+19*k)
    face(m,rings[0],color,WEATHERED_LIMESTONE,(0,-1,0))
    face(m,rings[-1],_tint(color,3),WEATHERED_LIMESTONE,(0,1,0))


def _moai(lod,h,seed,pukao=False):
    m=Mesh();c=_tint(TUFF,seed,16)
    # Normalized anatomy: broad torso, tucked chin, huge angular head and sloped back.
    _profile_solid(m,[(0,.31,-.095,.105),(.12,.36,-.11,.105),(.40,.38,-.10,.085),
                      (.48,.30,-.075,.07),(.51,.18,-.083,.062)],lod,c,seed)
    _profile_solid(m,[(.47,.17,-.09,.05),(.54,.19,-.153,.065),(.70,.205,-.133,.08),
                      (.87,.225,-.113,.10),(1,.20,-.086,.082)],lod,c,seed+83)
    # Swept brows and a wedge-shaped nose are geometry at all distances.
    for s in (-1,1):
        tube(m,(s*.005,.858,-.139),(s*.096,.832,-.131),.019,_tint(c,1),WEATHERED_LIMESTONE,(8,6,4)[lod])
        face(m,[(s*.023,.838,-.139),(s*.088,.815,-.137),(s*.088,.79,-.139),(s*.024,.81,-.144)],
             _tint(c,7,20),WEATHERED_LIMESTONE,(0,0,-1))
    nose=[(-.025,.835,-.145),(.025,.835,-.145),(.036,.687,-.219),(-.036,.687,-.219)]
    face(m,nose,_tint(c,2),WEATHERED_LIMESTONE,(0,0,-1))
    face(m,[nose[0],nose[3],(-.036,.683,-.138)],_tint(c,6),WEATHERED_LIMESTONE,(-1,0,0))
    face(m,[nose[1],(.036,.683,-.138),nose[2]],_tint(c,4),WEATHERED_LIMESTONE,(1,0,0))
    face(m,[nose[3],nose[2],(.036,.683,-.138),(-.036,.683,-.138)],_tint(c,9),WEATHERED_LIMESTONE,(0,-1,0))
    m.box((0,.616,-.147),(.101,.028,.025),_tint(c,3),WEATHERED_LIMESTONE)
    m.box((0,.598,-.146),(.084,.018,.025),_tint(c,4),WEATHERED_LIMESTONE)
    m.box((0,.609,-.162),(.082,.004,.003),_tint(c,7,27),WEATHERED_LIMESTONE)
    for s in (-1,1):
        m.box((s*.111,.746,.018),(.027,.23,.042),_tint(c,8),WEATHERED_LIMESTONE)
        if lod<2:
            m.box((s*.126,.758,.014),(.008,.165,.011),_tint(c,9,27),WEATHERED_LIMESTONE)
            tube(m,(s*.17,.40,-.03),(s*.17,.17,-.06),.016,c,WEATHERED_LIMESTONE,6)
            tube(m,(s*.17,.17,-.06),(s*.04,.12,-.117),.013,c,WEATHERED_LIMESTONE,6)
            for j in range(4):
                tube(m,(s*.075,.125+j*.009,-.119),(s*.015,.12+j*.009,-.121),.0035,_tint(c,j),WEATHERED_LIMESTONE,4)
    if pukao:
        lathe(m,(0,0),[(.98,.15),(1.13,.16),(1.17,.145),(1.19,.095)],
              lambda k,j:_tint((142,73,51),k*17+j,12),WEATHERED_LIMESTONE,(20,14,10)[lod])
    return m.transform([[h,0,0,0],[0,h,0,0],[0,0,h,0],[0,0,0,1]])


def _tongariki(lod):
    m=Mesh()
    # A low rubble ramp joins the original volcanic-stone platform to ground.
    _profile_solid(m,[(-.7,100,-6.5,5.5),(.15,99,-6.5,5.5),(1.7,96,-2.8,4.3)],lod,BASALT,31)
    if lod<2:
        # Offset stone courses sit on the ramp plane, exposing real mortar gaps.
        count=(48,30)[lod]
        for row in range(3):
            y0=.16+row*.48; y1=y0+.43
            z0=-6.5+(y0-.15)*3.7/1.55-.026
            z1=-6.5+(y1-.15)*3.7/1.55-.026
            for j in range(count):
                x0=-37.3+j*74.6/count+.04; x1=x0+74.6/count-.09
                c=_tint((82,82,72),j+19*row,17)
                _weathered_face(m,[(x0,y0,z0),(x1,y0,z0),(x1,y1,z1),(x0,y1,z1)],
                                c,(0,1,-1),lod,j+17*row,STONE_WALL)
        for row in range(3):
            for j in range((64,40)[lod]):
                x=(j+.5)*96/(64,40)[lod]-48
                m.box((x,.28+row*.48,4.36), (96/(64,40)[lod]-.04,.43,.16),_tint(BASALT,j+17*row,17),STONE_WALL)
    for i,h in enumerate(MOAI_HEIGHTS):
        # From the inland face the second statue from the right has the sole pukao.
        m.merge(turn(_moai(lod,h,11+i*7,pukao=i==13),(-2,1,0,2,-1)[i%5],x=(i-7)*5.25,y=1.7,z=.65))
    return m


def _callanish(lod,components=False):
    m=_ComponentMesh() if components else Mesh()
    _stone(m,-.8,0,4.8,1.5,.35,lod,1,GNEISS,pointed=True)
    for j in range(13):
        a=math.tau*j/13
        _stone(m,6*math.sin(a),6.7*math.cos(a),2.3+(j%5)*.23,1.3,.48,lod,j+4,GNEISS,angle=j*17,pointed=True)
    for side,count in [(-1,10),(1,9)]:
        for j in range(count):
            distance=12+j*8.55
            x=math.sin(math.radians(17))*distance+side*(3+distance*.004)
            z=math.cos(math.radians(17))*distance
            _stone(m,x,z,(2.35+abs(j-4)*.23)*(.75 if side==1 else 1),1.12,.42,lod,30+j+(side+1)*9,GNEISS,angle=19+j*9,pointed=True)
    for angle,count,length in [(72,5,23.2),(180,5,27.2),(252,4,13)]:
        a=math.radians(angle)
        for j in range(count):
            r=8+(length-8)*j/(count-1)
            _stone(m,r*math.sin(a)-.8,r*math.cos(a)-1,2.1+.31*(j%4),1.15,.48,lod,60+j+angle,GNEISS,angle=j*31,pointed=True)
    # Low open tomb walls remain east of the central monolith.
    for x in (1.1,3.5):
        for j in range(7):_stone(m,x,-2.6+j*.75,.50,.7,.45,lod,100+j,GNEISS)
    return m.components() if components else m


BRODGAR_STANDING_SLOTS = (0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,21,24,28,31,35,40,43,46,50,54,57)


def _brodgar(lod,components=False):
    m=_ComponentMesh() if components else Mesh()
    for j,slot in enumerate(BRODGAR_STANDING_SLOTS):
        a=math.tau*slot/60
        _stone(m,52*math.sin(a),52*math.cos(a),2.1+(j*5%14)*.2,1.05+(j%4)*.15,.40+(j%3)*.08,lod,slot,SANDSTONE,angle=slot*6,pointed=True)
    for j,slot in enumerate((17,19,26,33,38,45,48,52,59)):
        a=math.tau*slot/60
        _stone(m,52*math.sin(a),52*math.cos(a),.28+(j%2)*.17,1.5,2.4,lod,90+j,SANDSTONE,angle=slot*6)
    return m.components() if components else m


def _plinth(m,width,depth,height,lod,color=MARBLE):
    m.box((0,-.35,0),(width+.4,.7,depth+.4),_tint(color,3),STONE_WALL)
    for y,h,w,d in [(0,.35,width,depth),(.35,.26,width-.25,depth-.25),
                     (.61,height-1.13,width-.8,depth-.8),(height-.52,.3,width-.15,depth-.15),(height-.22,.22,width,depth)]:
        m.box((0,y+h/2,0),(w,h,d),_tint(color,int(y*13)),VEINED_MARBLE)
    if lod<2:
        for side in range(4):
            sub=Mesh()
            for j in range(7):
                yy=height*.37+j*.20
                sub.box((0,yy,-(depth-.8)/2-.012),(width*.46,.027,.018),_tint(color,8,35),VEINED_MARBLE)
            m.merge(turn(sub,side*90))


def _saint(m,y,h,lod,paul):
    color=(80,104,91)
    # Contemporary bronze finial: robed saint with head, arms and identifying object.
    sub=Mesh()
    _profile_solid(sub,[(0,.72,-.30,.30),(.68,.60,-.28,.24),(1.75,.73,-.21,.20),(2.25,.62,-.20,.20)],lod,color,21)
    sub.ellipsoid((0,2.55,-.02),(.25,.32,.24),color,SCULPTURE_METAL,(12,8,6)[lod],(6,4,3)[lod])
    for sign in (-1,1):
        tube(sub,(sign*.32,2.06,0),(sign*.51,1.30,-.08),.13,color,SCULPTURE_METAL,(8,6,4)[lod])
    if paul:
        tube(sub,(.50,.25,-.09),(.51,1.45,-.09),.032,(104,122,100),SCULPTURE_METAL,5)
        tube(sub,(.36,1.26,-.09),(.65,1.26,-.09),.035,color,SCULPTURE_METAL,5)
    else:
        sub.box((-.48,1.3,-.09),(.16,.37,.09),(99,120,101),SCULPTURE_METAL)
    sub.transform([[h/2.87,0,0,0],[0,h/2.87,0,y],[0,0,h/2.87,0],[0,0,0,1]])
    # Carved robe meshes use bronze too.
    sub.triangles=[(p,c,SCULPTURE_METAL) for p,c,_ in sub.triangles]
    m.merge(sub)


def _roman_column(lod,paul=False):
    m=Mesh();base=8.0 if paul else 5.4;shaft=29.617 if paul else 29.78
    r=1.85 if paul else 1.83;turns=21 if paul else 23
    _plinth(m,6.2,6.2,base,lod)
    lathe(m,(0,0),[(base,2.4),(base+.4,2.4),(base+.7,r),
                     (base+shaft-.8,r*.91),(base+shaft-.35,2.25),(base+shaft,2.35)],
          lambda k,j:_tint(MARBLE,k*7+j,8),VEINED_MARBLE,(64,40,24)[lod])
    # The helical frieze is retained on all LODs; closer views resolve relief figures.
    segments=(36,24,16)[lod]
    for j in range(turns*segments):
        a=j*math.tau/segments;b=(j+1)*math.tau/segments
        yy=base+.8+(shaft-1.8)*j/(turns*segments)
        hh=(shaft-1.8)/(turns*segments);rr=r*(1-.09*(yy-base-.7)/(shaft-1.5))+.03
        tube(m,(rr*math.cos(a),yy,rr*math.sin(a)),(rr*math.cos(b),yy+hh,rr*math.sin(b)),
             .045 if paul else .033,(165,157,141),VEINED_MARBLE,4)
        if lod<2 and j%((2,3)[lod])==0:
            figure=Mesh();figure.box((0,yy+.32,-rr-.045),(.13,.35,.12),_tint(MARBLE,j,13),VEINED_MARBLE)
            figure.ellipsoid((0,yy+.55,-rr-.07),(.08,.09,.07),_tint(MARBLE,j+4),VEINED_MARBLE,6,3)
            if lod==0:
                tube(figure,(-.08,yy+.40,-rr-.06),(-.16,yy+.22,-rr-.09),.038,MARBLE,VEINED_MARBLE,4)
            m.merge(turn(figure,-math.degrees(a)-90))
    y=base+shaft
    m.box((0,y+.22,0),(4.9,.44,4.9),_tint(MARBLE,5),VEINED_MARBLE)
    lathe(m,(0,0),[(y+.44,1.1),(y+1.1,1.1),(y+1.25,.94)],MARBLE,VEINED_MARBLE,(24,16,12)[lod])
    _saint(m,y+1.25,3.25 if paul else 3.43,lod,paul)
    return m


def _obelisk_shaft(m,bottom,height,width,depth,top_width,color,lod):
    _profile_solid(m,[(bottom,width,-depth/2,depth/2),
                      (bottom+height,top_width,-depth*.32,depth*.32)],lod,color,83)


def _luxor(lod):
    m=Mesh();granite=(175,145,117)
    _plinth(m,5.9,5.9,8.4,lod,(143,125,116))
    _obelisk_shaft(m,8.4,21.1,2.45,2.45,1.52,granite,lod)
    for side in range(4):
        sub=Mesh()
        # Three hieroglyph columns follow taper; simplified signs are incised shadows.
        for row in range((38,25,15)[lod]):
            yy=9.2+row*19.6/(38,25,15)[lod];rad=1.225-(yy-8.4)/21.1*.44
            for col in (-1,0,1):
                xx=col*rad*.49;c=_tint((116,92,70),row+col,8)
                sub.box((xx,yy,-rad-.01),(.075,.26,.025),c,WEATHERED_LIMESTONE)
                sub.box((xx+.045,yy+.04,-rad-.012),(.13,.035,.025),c,WEATHERED_LIMESTONE)
                if lod<2 and row%3==0:
                    sub.ellipsoid((xx,yy+.22,-rad-.02),(.08,.08,.018),c,WEATHERED_LIMESTONE,6,3)
        m.merge(turn(sub,side*90))
    for a,b in [((- .76,29.5,-.784),(.76,29.5,-.784)),((.76,29.5,-.784),(.76,29.5,.784)),((.76,29.5,.784),(-.76,29.5,.784)),((-.76,29.5,.784),(-.76,29.5,-.784))]:
        mid=(np.asarray(a)+b)/2
        face(m,[a,b,(0,31.4,0)],(217,177,68),SCULPTURE_METAL,(mid[0],.5,mid[2]))
    return m


def _aksum(lod):
    m=Mesh();stone=(161,154,137)
    m.box((0,-.30,0),(4.2,.6,2.3),_tint(stone,1),STONE_WALL)
    _obelisk_shaft(m,-.05,22.2,2.8,1.35,2.02,stone,lod)
    # Rounded false-building summit, unlike an Egyptian pyramidion.
    front=[(-1.01,21.9),(-.99,23.0),(-.64,23.67),(0,24),(.64,23.67),(.99,23.0),(1.01,21.9)]
    for s in (-1,1):
        face(m,[(x,y,s*.42) for x,y in front],stone,WEATHERED_LIMESTONE,(0,0,s))
    for (x,y),(xx,yy) in zip(front,front[1:]):
        face(m,[(x,y,-.42),(xx,yy,-.42),(xx,yy,.42),(x,y,.42)],_tint(stone,3),WEATHERED_LIMESTONE,(x,1,0))
    for side in (0,180):
        sub=Mesh()
        for level in range(10):
            yy=1.3+level*2.00;half=1.4-(yy/22.2)*.39;zz=.675-(yy/22.2)*.24
            for sign in (-1,1):
                xx=sign*half*.52
                sub.box((xx,yy+.48,-zz-.006),(.47,.98,.025),(100,98,87),WEATHERED_LIMESTONE)
                if lod<2:
                    sub.box((xx,yy+.48,-zz-.025),(.31,.76,.045),_tint(stone,level),WEATHERED_LIMESTONE)
                    sub.box((xx,yy+.48,-zz-.052),(.035,.76,.035),(114,109,94),WEATHERED_LIMESTONE)
            sub.box((0,yy-.12,-zz-.04),(half*1.91,.13,.10),_tint(stone,level+13),WEATHERED_LIMESTONE)
            if lod==0:
                for xx in np.linspace(-half*.85,half*.85,6):
                    sub.box((xx,yy-.12,-zz-.10),(.13,.17,.16),_tint(stone,level+8),WEATHERED_LIMESTONE)
        sub.box((0,1.16,-.69),(.88,2.17,.10),(104,99,88),WEATHERED_LIMESTONE)
        sub.box((0,1.16,-.75),(.65,1.94,.05),_tint(stone,8),WEATHERED_LIMESTONE)
        m.merge(turn(sub,side))
    for side in (90,270):
        sub=Mesh()
        for level in range(10):
            yy=1.3+level*2;depth=.675-(yy/22.2)*.24
            zz=1.4-(yy/22.2)*.39
            sub.box((0,yy+.48,-zz-.012),(.38,.98,.04),(100,98,87),WEATHERED_LIMESTONE)
            sub.box((0,yy-.12,-zz-.04),(depth*1.45,.13,.10),_tint(stone,level+13),WEATHERED_LIMESTONE)
            if lod<2:
                sub.box((0,yy+.48,-zz-.04),(.23,.76,.04),_tint(stone,level),WEATHERED_LIMESTONE)
        m.merge(turn(sub,side))
    return m


def _iron_pillar(lod):
    m=Mesh();iron=(71,53,40)
    m.box((0,-.18,0),(1.1,.36,1.1),(152,119,92),STONE_WALL)
    profile=[(-.45,.215),(0,.21),(3.8,.17),(5.15,.155),(5.23,.22),
             (5.36,.22),(5.43,.18),(5.58,.24),(5.68,.24),(5.78,.18),
             (5.92,.21),(6.02,.18),(6.09,.125)]
    lathe(m,(0,0),profile,lambda k,j:_tint(iron,k*17+j,14),SCULPTURE_METAL,(40,28,16)[lod])
    if lod<2:
        for row in range(6):
            for col in range(9):
                yy=1.95+row*.053;xx=(col-4)*.027
                zz=-math.sqrt(.19**2-xx**2)-.002
                m.box((xx,yy,zz),(.018,.004,.003),(118,91,61),SCULPTURE_METAL)
        for j in range(8):
            a=math.tau*j/8
            tube(m,(.205*math.cos(a),5.44,.205*math.sin(a)),(.23*math.cos(a),5.70,.23*math.sin(a)),.032,iron,SCULPTURE_METAL,5)
    return m


def _ashoka(lod):
    m=Mesh();red=(167,113,78)
    lathe(m,(0,0),[(-.6,.58),(0,.57),(7.55,.40),(7.73,.47),(7.89,.75),
                     (8.25,.77),(8.60,.53),(8.80,.49),(8.90,.81),(9.08,.81)],
          lambda k,j:_tint(red,k*17+j,8),WEATHERED_LIMESTONE,(48,32,20)[lod])
    # One seated Asiatic lion facing north; no four-lion national-emblem substitute.
    lion=Mesh();n=(16,12,8)[lod];rings=(8,6,4)[lod]
    lion.ellipsoid((0,17.02,.17),(.43,.65,.39),red,WEATHERED_LIMESTONE,n,rings)
    lion.ellipsoid((0,17.63,-.17),(.44,.46,.34),_tint(red,5),WEATHERED_LIMESTONE,n,rings)
    lion.ellipsoid((0,17.67,-.43),(.27,.23,.23),red,WEATHERED_LIMESTONE,n,rings)
    lion.ellipsoid((0,17.68,-.60),(.16,.11,.10),_tint(red,4),WEATHERED_LIMESTONE,n,rings)
    for sign in (-1,1):
        tube(lion,(sign*.27,17.11,-.13),(sign*.27,16.43,-.29),.11,red,WEATHERED_LIMESTONE,n)
        lion.ellipsoid((sign*.27,16.45,-.37),(.15,.11,.21),red,WEATHERED_LIMESTONE,n,rings)
        lion.ellipsoid((sign*.29,17.97,-.20),(.12,.13,.08),_tint(red,7),WEATHERED_LIMESTONE,n,rings)
        lion.ellipsoid((sign*.21,17.78,-.43),(.045,.035,.025),(78,54,36),WEATHERED_LIMESTONE,6,3)
    if lod<2:
        for j in range(12):
            a=math.tau*j/12
            tube(lion,(.38*math.cos(a),17.62+.35*math.sin(a),-.22),(.42*math.cos(a),17.62+.42*math.sin(a),-.12),.043,_tint(red,j),WEATHERED_LIMESTONE,5)
        tube(lion,(.2,16.73,.44),(.40,16.45,.50),.065,red,WEATHERED_LIMESTONE,7)
    lion.transform([[1,0,0,0],[0,1.1162790698,0,-16.38*.1162790698],[0,0,1,0],[0,0,0,1]])
    m.merge(turn(lion,180,y=-7.3))
    return m


BUILDERS = {
    'stonehenge': _stonehenge,
    'ahu-tongariki': _tongariki,
    'callanish-stones': _callanish,
    'ring-of-brodgar': _brodgar,
    'trajan-column': lambda lod: _roman_column(lod),
    'marcus-aurelius-column': lambda lod: _roman_column(lod, True),
    'luxor-obelisk': _luxor,
    'aksum-obelisk': _aksum,
    'iron-pillar-delhi': _iron_pillar,
    'ashoka-pillar-vaishali': _ashoka,
}


def build(name,lod):
    if lod not in (0,1,2):
        raise ValueError('landmark LOD must be 0, 1, or 2')
    return BUILDERS[name](lod)


def build_components(name,lod):
    """Rigid support groups, in identical order at every LOD.

    Independent megaliths follow local terrain separately. Supported lintels,
    complete trilithons, each column, and the moai platform remain connected.
    These meshes use the unshifted architectural datum, as does build().
    """
    if lod not in (0,1,2):
        raise ValueError('landmark LOD must be 0, 1, or 2')
    if name in ('stonehenge','callanish-stones','ring-of-brodgar'):
        return BUILDERS[name](lod,components=True)
    return [build(name,lod)]


def _support_hulls(name):
    """Own only stone feet; open ground and its vegetation remain source-owned."""
    polygons=[]
    for part in build_components(name,0):
        points=sorted({(float(p[0]),float(p[2]))
                       for tri,_,_ in part.triangles for p in tri if p[1]<=.05})
        def cross(o,a,b):
            return (a[0]-o[0])*(b[1]-o[1])-(a[1]-o[1])*(b[0]-o[0])
        lower=[];upper=[]
        for p in points:
            while len(lower)>=2 and cross(lower[-2],lower[-1],p)<=0:
                lower.pop()
            lower.append(p)
        for p in reversed(points):
            while len(upper)>=2 and cross(upper[-2],upper[-1],p)<=0:
                upper.pop()
            upper.append(p)
        polygon=[list(p) for p in lower[:-1]+upper[:-1]]
        if len(polygon)<3:
            raise ValueError(f'{name}: stone support has no area')
        polygons.append(polygon)
    return polygons


def _entry(name,city,lat,lon,heading,center,half,height,description,references):
    x,z=center;hx,hz=half
    polygons=(_support_hulls(name) if name in ('stonehenge','callanish-stones','ring-of-brodgar')
              else [[[x-hx,z-hz],[x+hx,z-hz],[x+hx,z+hz],[x-hx,z+hz]]])
    return dict(name=name,city=city,lat=lat,lon=lon,heading=heading,base='terrain',
                suppress_buildings=True,suppress_roads=False,suppress_trees=True,
                references=references,
                attribution='Geometry and sculpture originally authored for Low Poly Earth; dimensions and reference notes credited in the local evidence file.',
                description=description,
                source_exclusion_polygons=polygons,
                suppress_bbox=[[x-hx,z-hz],[x+hx,z+hz]],
                ground_samples_file=f'tools/monument_catalog/references/ground/{name}.json',
                max_source_building_span_m=2*max(half)+5,
                audit=dict(survey_file=f'tools/monument_catalog/references/ancient/{name}.json',
                           architectural_dimensions_m=[2*hx,height,2*hz],
                           grounding_note='Raw terrain is unchanged. Stable rigid components are positioned against surveyed elevation, with their buried supports extended to meet it. Small carvings and irregular stone profiles are authored approximations; see evidence.'),
                capture=dict(distance=max(20,2.2*max(2*hx,2*hz,height)),azimuth=210,elevation=25,
                             target_height=height*.40,time_utc='2026-03-20T10:00:00Z'))


ENTRIES = [
    _entry('stonehenge','Amesbury',51.17885,-1.82618,49,[0,31],[18,50],7.45,
           'Present sarsen ruins with seventeen circle uprights, six outer lintels, three complete trilithons, fallen stones, bluestones and northeast Heel Stone.',
           ['https://www.english-heritage.org.uk/visit/places/stonehenge/history-and-stories/history/description/',
            'https://historicengland.org.uk/research/results/reports/6075/StonehengeLaserScan_ArchaeologicalAnalysis']),
    _entry('ahu-tongariki','Rapa Nui',-27.12583,-109.27694,110,[0,0],[51,7],10.0,
           'Fifteen varied inland-facing tuff moai on the volcanic masonry ahu, with modeled brow, nose, jaw, long ears and hands; one red-scoria pukao.',
           ['https://www.chile.travel/itinerarios/descubre-la-magia-de-rapa-nui-junto-a-sus-playas-volcanes-y-atractivos-lugares-ocultos/',
            'https://chiletravelofficial.cn/experiences/cultural-heritage/all/discover-12-moai-in-rapa-nui',
            'https://en.wikipedia.org/wiki/Ahu_Tongariki']),
    _entry('callanish-stones','Isle of Lewis',58.197541,-6.745145,0,[5,30],[30,61],4.8,
           'Lewisian-gneiss circle with central 4.8m monolith, nineteen-stone northern avenue, three shorter arms and low open tomb walls.',
           ['https://www.historicenvironment.scot/visit/all/calanais-standing-stones/history-and-stories/',
            'https://en.wikipedia.org/wiki/Callanish_Stones']),
    _entry('ring-of-brodgar','Orkney',59.0014818,-3.2297227,0,[0,0],[54,54],4.7,
           'Incomplete 104m sandstone circle with twenty-seven standing slabs, nine low surviving fragments and deliberately open missing sectors.',
           ['https://www.historicenvironment.scot/visit/all/ring-of-brodgar-stone-circle-and-henge/history-and-stories/',
            'https://i.rcahms.gov.uk/canmore-pdf/WP00006811.pdf']),
    _entry('trajan-column','Rome',41.89585,12.48430,40,[0,0],[3.6,3.6],39.86,
           'Carrara marble victory column with twenty-three helical relief turns, framed pedestal inscriptions, Doric capital and bronze Saint Peter.',
           ['https://www.turismoroma.it/it/node/1021']),
    _entry('marcus-aurelius-column','Rome',41.901081,12.479801,7,[0,0],[3.6,3.6],42.12,
           'Deeply banded marble narrative column, tall restored pedestal, Doric capital and bronze Saint Paul carrying a sword.',
           ['https://turismoroma.it/en/node/1031',
            'https://soprintendenzaspecialeroma.it/intervento/id-71-colonna-marco-aurelio/']),
    _entry('luxor-obelisk','Paris',48.86563,2.32124,32,[0,0],[3.4,3.4],31.4,
           'Tapering pink Aswan-granite obelisk with three incised inscription bands per face, Breton granite pedestal and golden pyramidion.',
           ['https://beauxartsparis.fr/sites/default/files/inline-files/Cahier-des-charges-appel-projet-Obelisque2.pdf',
            'https://www.paris.fr/pages/parhistoire-repond-a-vos-questions-sur-le-patrimoine-et-l-histoire-de-paris-3568']),
    _entry('aksum-obelisk','Aksum',14.13222,38.71972,12,[0,0],[2.6,1.7],24,
           'Returned 24m stele with stepped false-storey beams, paired recessed windows, false doors and a rounded summit.',
           ['https://whc.unesco.org/en/list/15',
            'https://courier.unesco.org/fr/articles/ethiopie-trois-millenaires-de-legendes-et-dhistoire']),
    _entry('iron-pillar-delhi','Delhi',28.5246556,77.1850694,0,[0,0],[.85,.85],6.09,
           'Slender dark wrought-iron shaft, forged ringed and fluted capital and restrained warm inscription strokes; below-ground shaft remains embedded.',
           ['https://www.delhitourism.gov.in/tourist_place/qutab_minar.html',
            'https://nehrusciencecentre.gov.in/wp-content/uploads/2024/01/SCI-MAIL-JAN-TO-MARCH-2024.pdf']),
    _entry('ashoka-pillar-vaishali','Vaishali',26.01400,85.109011,0,[0,0],[1.3,1.3],11.0,
           'Polished reddish sandstone monolith with inverted bell capital, circular abacus and a single seated north-facing Asiatic lion.',
           ['https://tourism.gov.in/sites/default/files/2020-04/Bihar.pdf',
            'https://vaishali.nic.in/tourist-place/ashoka-pillar/',
            'https://tourism.bihar.gov.in/en/destinations/vaishali/ashokan-pillar-kolhua-vaishali',
            'https://www.wikidata.org/wiki/Q19323579']),
]
