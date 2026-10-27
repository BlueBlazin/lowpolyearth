"""Original metric Gothic meshes; +z is the liturgical axis before heading.

Survey and published dimensional controls are recorded in references/sacred_gothic.
The material atlas supplies stone grain, slate and lead; metre-scale joints and
recesses remain geometry so that architectural rhythm survives distant mips.
"""
import math
import numpy as np
from city_catalog.common import Mesh
from heritage_catalog.geometry import face, lathe, ring, tube, turn, arch, arch_points, arch_bay, column, balustrade, plan_solid
from heritage_catalog.materials import STONE_WALL, SLATE_ROOF, METAL_ROOF, WINDOW, SCULPTURE_METAL, retag

TAU=math.tau
DARK=(40,43,42)
COLOGNE=(132,126,112)
C_TRIM=(157,150,130)
GRANITE=(171,165,147)
M_TRIM=(202,196,175)
LAJAS=(137,144,139)
L_TRIM=(217,216,198)
SLATE=(71,81,87)
LEAD=(111,126,130)


def _box(m,x,z,bottom,top,w,d,color):
    m.box((x,(bottom+top)/2,z),(w,top-bottom,d),color,STONE_WALL)


def _courses(m,x,z,w,bottom,top,lod,color,brick=2.0):
    if lod==2:return
    rows=max(1,int((top-bottom)/(1.1 if lod==0 else 2.2)))
    for j in range(rows):
        y0=bottom+(top-bottom)*j/rows;y1=bottom+(top-bottom)*(j+1)/rows
        count=max(1,int(w/(brick if lod==0 else brick*2)))
        edges=[-w/2]+[(-w/2+(i+.5*(j%2))*w/count) for i in range(1,count)]+[w/2]
        for i,(a,b) in enumerate(zip(edges,edges[1:])):
            tone=(-9,-3,2,6,-5,3)[(i*3+j*5)%6]
            c=tuple(max(0,min(255,v+tone))for v in color)
            face(m,[(x+a+.016,y0+.015,z),(x+b-.016,y0+.015,z),(x+b-.016,y1-.015,z),(x+a+.016,y1-.015,z)],c,STONE_WALL,(0,0,-1))


def _side(m,facade,side,x=0,z=0):
    if side<0:
        facade.triangles=[(p[[0,2,1]]*np.array([-1,1,1]),c,mat)for p,c,mat in facade.triangles]
    m.merge(turn(facade,90 if side<0 else -90,x,0,z))


def _roof(m,x,z,w,d,eave,peak,lod,color=SLATE,material=SLATE_ROOF):
    for s in (-1,1):
        face(m,[(x-w/2,eave,z+s*d/2),(x+w/2,eave,z+s*d/2),(x,peak,z+s*d/2)],GRANITE,STONE_WALL,(0,0,s))
        face(m,[(x+s*w/2,eave,z-d/2),(x+s*w/2,eave,z+d/2),(x,peak,z+d/2),(x,peak,z-d/2)],color,material,(s,1,0))
        if lod<2:
            for zz in np.linspace(z-d/2,z+d/2,max(2,int(d/(2 if lod==0 else 4)))+1):
                tube(m,(x+s*w/2,eave+.05,float(zz)),(x,peak+.05,float(zz)),.045,color,material,4)
    tube(m,(x,peak+.1,z-d/2),(x,peak+.1,z+d/2),.12,color,material,6)


def _lancet(m,x,y,z,w,h,lod,trim,glass=True,pointed=True):
    arch(m,(x,y,z),w,h,.16 if w<3 else .24,trim,pointed,(20,12,8)[lod],DARK,STONE_WALL)
    if lod==2:return
    # Colored panes stay subdued on the exterior and sit behind projecting mullions.
    palette=[(66,79,83),(84,66,60),(72,79,69),(100,90,65)]
    n=2 if w<4 else 4
    for i in range(n):
        for j in range(4 if lod==0 else 2):
            pw=w*.88/n;ph=h*.49/(4 if lod==0 else 2)
            xx=x-w*.44+(i+.5)*pw;yy=y+h*.03+(j+.5)*ph
            if glass:face(m,[(xx-pw*.43,yy-ph*.43,z-.01),(xx+pw*.43,yy-ph*.43,z-.01),(xx+pw*.43,yy+ph*.43,z-.01),(xx-pw*.43,yy+ph*.43,z-.01)],palette[(i+j*3)%4],WINDOW,(0,0,-1))
        if i<n-1:m.box((x-w*.44+(i+1)*w*.88/n,y+h*.32,z-.08),(.09,h*.61,.12),trim,STONE_WALL)
    if h>w*1.8:
        ring(m,(x,y+h*.70,z-.07),w*.23,.065,trim,(16,12,8)[lod],STONE_WALL,'xy')
        for dx in (-w*.23,w*.23):arch(m,(x+dx,y+h*.47,z-.07),w*.43,h*.30,.07,trim,True,(16,12,8)[lod],None,STONE_WALL)


def _rose(m,x,y,z,r,lod,trim):
    n=(40,24,12)[lod]
    for i in range(n):
        a=i*TAU/n;b=(i+1)*TAU/n
        face(m,[(x,y,z+.03),(x+r*math.cos(a),y+r*math.sin(a),z+.03),(x+r*math.cos(b),y+r*math.sin(b),z+.03)],[(65,78,86),(86,68,68),(93,85,67)][i%3],WINDOW,(0,0,-1))
    for rr in [r,r*.86,r*.28]:ring(m,(x,y,z-.08),rr,.10 if r<3 else .18,trim,n,STONE_WALL,'xy')
    petals=(12,12,8)[lod]
    for i in range(petals):
        a=i*TAU/petals
        tube(m,(x+r*.28*math.cos(a),y+r*.28*math.sin(a),z-.12),(x+r*.86*math.cos(a),y+r*.86*math.sin(a),z-.12),.07 if r<3 else .12,trim,STONE_WALL,4)
        if lod<2:ring(m,(x+r*.57*math.cos(a),y+r*.57*math.sin(a),z-.13),r*.18,.06,trim,(12,8)[lod],STONE_WALL,'xy')


def _pinnacle(m,x,z,y,h,lod,color):
    lathe(m,(x,z),[(y,h*.105),(y+h*.10,h*.12),(y+h*.43,h*.08),(y+h*.52,h*.115),(y+h*.93,.04),(y+h,.015)],color,STONE_WALL,8,angle=math.pi/8)
    if lod==0:
        for k in range(3):
            yy=y+h*(.6+k*.11);r=h*.09*(1-k*.23)
            for a in range(4):m.ellipsoid((x+r*math.cos(a*TAU/4),yy,z+r*math.sin(a*TAU/4)),(.18,.25,.18),color,STONE_WALL,6,3)


def _flying(m,z,side,inner,outer,y0,y1,lod,color):
    # A quadratic arch with open air underneath, plus its inclined compression cap.
    n=(16,10,6)[lod]
    points=[]
    for t in np.linspace(0,1,n+1):
        points.append((side*(inner+(outer-inner)*t),y0+(y1-y0)*t+4*math.sin(t*math.pi/2),z))
    for a,b in zip(points,points[1:]):tube(m,a,b,.52,color,STONE_WALL,6)
    tube(m,(side*inner,y0+1.4,z),(side*outer,y1+5.2,z),.48,color,STONE_WALL,6)


def _open_spire(m,x,z,bottom,top,r,lod,color):
    levels=(8,6,4)[lod];sides=8
    def radius(t):return r*(1-t)*(.93+.07*(1-t))+.05
    for i in range(sides):
        a=i*TAU/sides+math.pi/8;b=(i+1)*TAU/sides+math.pi/8
        for j in range(levels):
            t=j/levels;u=(j+1)/levels;rr=radius(t);ss=radius(u);y=bottom+(top-bottom)*t;yy=bottom+(top-bottom)*u
            p=(x+rr*math.cos(a),y,z+rr*math.sin(a));q=(x+ss*math.cos(a),yy,z+ss*math.sin(a));v=(x+rr*math.cos(b),y,z+rr*math.sin(b));vtop=(x+ss*math.cos(b),yy,z+ss*math.sin(b))
            tube(m,p,q,.20 if lod<2 else .28,color,STONE_WALL,6)
            tube(m,p,v,.16,color,STONE_WALL,4)
            # Each cone facet contains an open pointed tracery panel.
            mid=(a+b)/2;radial=np.array([math.cos(mid),0,math.sin(mid)]);tangent=np.array([-math.sin(mid),0,math.cos(mid)])
            path=[]
            for u0,v0 in arch_points(1,1,(12,8,6)[lod],True):
                fraction=.10+.80*v0;tt=t+(u-t)*fraction
                pos=np.array([x,bottom+(top-bottom)*tt,z])+radial*radius(tt)*math.cos(math.pi/8)+tangent*u0*radius(tt)*.59
                path.append(pos)
            for pa,pb in zip(path,path[1:]):tube(m,pa,pb,.13 if lod<2 else .17,color,STONE_WALL,4)
            if lod==0:
                mid=(np.array(p)+np.array(q))*.5
                m.ellipsoid(mid,(.29,.46,.29),color,STONE_WALL,6,3)
    _pinnacle(m,x,z,top-.6,1.0,lod,color)


def _cologne_tower(m,x,z,top,lod):
    _box(m,x,z,0,69,21.8,22.2,COLOGNE)
    for y in (1,20.3,45.8,69,96.8):_box(m,x,z,y,y+.5,22.5 if y<70 else 19.9,22.9 if y<70 else 19.9,C_TRIM)
    for angle in (0,90,180,270):
        f=Mesh();_courses(f,0,-11.13,21.7,0,69,lod,COLOGNE)
        for bottom,h in [(3,13),(25,17),(48,18)]:
            for xx in (-4.5,4.5):_lancet(f,xx,bottom,-11.3,6.2,h,lod,C_TRIM)
        for xx in (-10,0,10):
            _box(f,xx,-11.55,0,74,.65,1.15,C_TRIM)
            _pinnacle(f,xx,-11.55,68,10,lod,C_TRIM)
        m.merge(turn(f,angle,x,0,z))
    # Octagonal bell gallery has actual open arches on all eight sides.
    for i in range(8):
        a=i*TAU/8;xx=x+8.2*math.sin(a);zz=z+8.2*math.cos(a)
        bay=Mesh();arch_bay(bay,(0,70,0),6.85,26,1.1,1.15,COLOGNE,lod,True)
        _pinnacle(bay,-3.45,0,94,12,lod,C_TRIM)
        m.merge(turn(bay,math.degrees(a)+180,xx,0,zz))
    _open_spire(m,x,z,99,top-.4,9.2,lod,C_TRIM)


def _cologne(lod):
    m=Mesh()
    # Domplatte stays at the west portal datum while the surrounding streets
    # fall toward the choir. A solid shaped base carries every outer buttress;
    # its buried seven-metre depth clears the sampled coarse DEM interface.
    apron=[(-33.2,-80.5),(33.2,-80.5),(33.2,-52),(28.2,-52),
           (28.2,-21.8),(44.5,-21.8),(44.5,21.8),(28.2,21.8),(28.2,40)]
    apron += [(28.2*math.cos(a),40+28.2*math.sin(a)) for a in np.linspace(0,math.pi,17)[1:]]
    apron += [(-28.2,21.8),(-44.5,21.8),(-44.5,-21.8),(-28.2,-21.8),(-28.2,-52),(-33.2,-52)]
    plan_solid(m,apron,-7,.08,(143,140,129),STONE_WALL)
    for a,b in zip(apron,apron[1:]+apron[:1]):
        face(m,[(a[0],.10,a[1]),(b[0],.10,b[1]),(0,.10,0)],(164,162,149),8,(0,1,0))
        if lod<2:
            for y in (-4.8,-3.2,-1.6):
                tube(m,(a[0],y,a[1]),(b[0],y,b[1]),.035,(122,120,111),STONE_WALL,4)
    # North transept approach bridges the measured2.3m street-level drop.
    # Step foundations extend below the uneven lower landing rather than float.
    for i in range(15):
        x=-44.5-(i+.5)*.47;top=.06-(i+1)*2.3/15
        _box(m,x,0,-4,top,.49,18,(156,154,142))
    _box(m,-51.75,0,-4,-2.30,.8,18,(156,154,142))
    _box(m,0,-6,-1.2,1,48.5,139,COLOGNE)
    _box(m,0,-9,0,23,45.8,117,COLOGNE)
    _box(m,0,0,0,24,79,38.4,COLOGNE)
    _box(m,0,-10,23,47,15.5,119,COLOGNE)
    _box(m,0,0,24,47,79,15.5,COLOGNE)
    _roof(m,0,-10,16.5,119,47,61.1,lod,LEAD,METAL_ROOF)
    tr=Mesh();_roof(tr,0,0,16.5,79,47,61.1,lod,LEAD,METAL_ROOF);m.merge(turn(tr,90))
    for side in (-1,1):
        for a,b in [(-54,-19),(19,40)]:
            face(m,[(side*8,27,a),(side*8,27,b),(side*23,23.2,b),(side*23,23.2,a)],LEAD,METAL_ROOF,(side,1,0))
            if lod<2:
                for zz in np.arange(a,b,2.4):tube(m,(side*8,27.04,float(zz)),(side*23,23.24,float(zz)),.04,LEAD,METAL_ROOF,4)
    # Ambulatory plus seven radiating chapels; semicircular east termination.
    outline=[(-23,40),(23,40)]+[(23*math.cos(a),40+23*math.sin(a))for a in np.linspace(0,math.pi,15)]
    plan_solid(m,outline,0,23,COLOGNE,STONE_WALL)
    choir=[(-7.8,39),(7.8,39)]+[(7.8*math.cos(a),39+7.8*math.sin(a))for a in np.linspace(0,math.pi,9)]
    plan_solid(m,choir,23,47,COLOGNE,STONE_WALL)
    for i,a in enumerate(np.linspace(-math.pi/2,math.pi/2,7)):
        cx=19.8*math.sin(a);cz=40+19.8*math.cos(a)
        lathe(m,(cx,cz),[(0,5.2),(21,5.2)],COLOGNE,STONE_WALL,(12,10,8)[lod])
        lathe(m,(cx,cz),[(21,5.2),(23,4.7),(27,.03)],LEAD,METAL_ROOF,(12,10,8)[lod])
        f=Mesh();_lancet(f,0,5,-5.3,4.0,13,lod,C_TRIM);m.merge(turn(f,math.degrees(a)+180,cx,0,cz))
        # The outer flying-buttress pier carries its pinnacle independently
        # of the lower chapel roof; its toe overlaps the shaped Domplatte.
        pier=Mesh()
        for bottom,top,width in ((-.25,8,3.0),(8,16,2.8),(16,24.08,2.6)):
            _box(pier,0,0,bottom,top,width,2.2,COLOGNE)
        for y in (8,16,23.8):_box(pier,0,0,y,y+.22,3.15,2.35,C_TRIM)
        m.merge(turn(pier,math.degrees(a),27*math.sin(a),0,40+27*math.cos(a)))
        _pinnacle(m,27*math.sin(a),40+27*math.cos(a),24,17,lod,C_TRIM)
        tube(m,(8*math.sin(a),44,39+8*math.cos(a)),(27*math.sin(a),35,40+27*math.cos(a)),.6,C_TRIM,STONE_WALL,6)
    bays=[-49,-41,-33,-25,-17,17,25,33,40]
    for s in (-1,1):
        f=Mesh()
        for z in bays:
            _lancet(f,z,3,-23.18,5.2,16,lod,C_TRIM)
            _courses(f,z,-23.02,7.1,0,23,lod,COLOGNE)
        _side(m,f,s)
        for z in bays:
            _box(m,s*25.3,z,0,31,1.7,2.5,COLOGNE)
            _pinnacle(m,s*25.3,z,31,12,lod,C_TRIM)
            for bottom in (35,43):_flying(m,z,s,8,25.3,bottom,25,lod,C_TRIM)
            f=Mesh();_lancet(f,z,28,-7.95,5.8,17,lod,C_TRIM);_side(m,f,s)
            if lod<2:
                for y in (24.7,26.8):_box(m,s*9,z,y,y+.25,2.5,7.4,C_TRIM)
        for z in (-12,12):
            for xx in (27,35):
                _box(m,s*xx,z,23.8,27.08,2.9,2.9,COLOGNE)
                _pinnacle(m,s*xx,z,27,14,lod,C_TRIM)
    # Three pointed portals and the immense western window are separate from towers.
    _box(m,0,-76.9,0,48,16.3,2,COLOGNE)
    f=Mesh()
    for xx,w,h in [(0,10,17),(-17,8,14),(17,8,14)]:
        for k in range((4,3,1)[lod]):arch(f,(xx,0,-78.4-k*.23),w+k*.75,h+k*.8,.22,C_TRIM,True,(24,16,8)[lod],DARK if k==0 else None,STONE_WALL)
    _lancet(f,0,23,-78.35,13.5,23,lod,C_TRIM)
    face(f,[(-8.2,47,-78.35),(8.2,47,-78.35),(0,69.5,-78.35)],COLOGNE,STONE_WALL,(0,0,-1))
    _lancet(f,0,49,-78.5,5.5,14,lod,C_TRIM)
    for sx in (-1,1):tube(f,(sx*8.2,47,-78.5),(0,69.5,-78.5),.3,C_TRIM,STONE_WALL,6)
    if lod<2:
        for xx in (-28,-24,-10,-6,6,10,24,28):
            for yy in (5,11,17):
                _lancet(f,xx,yy,-78.35,1.1,3.2,lod,C_TRIM,False)
                lathe(f,(xx,-78.52),[(yy+.2,.30),(yy+1.8,.18)],C_TRIM,STONE_WALL,8)
                f.ellipsoid((xx,yy+2.08,-78.52),(.22,.3,.2),C_TRIM,STONE_WALL,8,4)
    m.merge(f)
    for s in (-1,1):
        _cologne_tower(m,s*17,-64.5,157.22 if s>0 else 157.18,lod)
        for z in (-75,-55):
            _box(m,s*29.2,z,0,49,3.1,3.1,COLOGNE)
            _box(m,s*29.2,z,49,68,2.0,2.0,COLOGNE)
            _pinnacle(m,s*29.2,z,68,9,lod,C_TRIM)
    for s in (-1,1):
        f=Mesh();_box(f,0,0,0,49,39.95,2,COLOGNE)
        _courses(f,0,-1.02,39.95,0,49,lod,COLOGNE)
        for xx in (-12,0,12):
            _lancet(f,xx,1,-1.12,7.6,17,lod,C_TRIM)
            _lancet(f,xx,26,-1.12,7.4,20,lod,C_TRIM)
        face(f,[(-19.97,49,-1),(19.97,49,-1),(0,69.95,-1)],COLOGNE,STONE_WALL,(0,0,-1))
        _lancet(f,0,50,-1.15,9,15,lod,C_TRIM)
        for xx in (-19,19):_pinnacle(f,xx,-1,48,16,lod,C_TRIM)
        tube(f,(-20,49,-1.2),(0,69.95,-1.2),.25,C_TRIM,STONE_WALL,6);tube(f,(0,69.95,-1.2),(20,49,-1.2),.25,C_TRIM,STONE_WALL,6)
        _side(m,f,s,s*40.3)
    # Iron crossing fleche, with a broad octagonal base and narrow pointed crown.
    lathe(m,(0,0),[(59,3.2),(65,3.2),(68,4.1),(72,3.5),(86,2.5),(108.8,.06)],LEAD,METAL_ROOF,8,angle=math.pi/8)
    for i in range(8):
        a=i*TAU/8;_pinnacle(m,3.4*math.cos(a),3.4*math.sin(a),70,8,lod,C_TRIM)
    _pinnacle(m,0,0,108.4,.72,lod,C_TRIM)
    return retag(m,{11:STONE_WALL,17:STONE_WALL})


def _mont_hall(m,x,z,w,d,bottom,eave,peak,lod,angle=0):
    sub=Mesh();_box(sub,0,0,bottom,eave,w,d,GRANITE);_roof(sub,0,0,w+.5,d+.5,eave,peak,lod)
    for a in (0,180):
        f=Mesh();_courses(f,0,-d/2-.025,w,bottom,eave,lod,GRANITE)
        for xx in np.linspace(-w*.35,w*.35,max(2,int(w/4))):
            for yy in np.arange(bottom+2,eave-2,5.5):_lancet(f,float(xx),float(yy),-d/2-.16,1.3,2.7,lod,M_TRIM,False,False)
        sub.merge(turn(f,a))
    m.merge(turn(sub,angle,x,0,z))


def _mont(lod):
    m=Mesh()
    # Present church platform at 78.6m; crypts retain the surrounding slopes.
    core=[(-18,-30),(13,-30),(18,-11),(18,12),(11,32),(-10,32),(-19,13)]
    plan_solid(m,core,-48,0,GRANITE,STONE_WALL)
    # Western terrace occupies the removed first three nave bays.
    terrace=[(-18,-62),(12,-62),(16,-53),(12,-28),(-18,-28)]
    plan_solid(m,terrace,-54,-.2,GRANITE,STONE_WALL)
    # The exposed west substructure contains the stacked Romanesque rooms.
    # Its openings face the bay, outside the terrace solid rather than inside it.
    _courses(m,-3,-62.08,30,-52,-.3,lod,GRANITE,2.2)
    for y,w,h in [(-40,1.1,2.7),(-27,2.0,4.9),(-13,2.0,4.4)]:
        for x in (-13,-6,1,8):
            _lancet(m,x,y,-62.28,w,h,lod,M_TRIM,False,False)
    for x in (-17.3,-9.8,-2.3,5.2,11.3):
        _box(m,x,-62.6,-53,-24,1.5,1.25,GRANITE)
        _box(m,x,-62.4,-24,-.2,1.05,.85,GRANITE)
    for y in (-34,-22,-8):_box(m,-3,-62.17,y,y+.3,30.6,.45,M_TRIM)
    for a,b in zip(terrace,terrace[1:]+terrace[:1]):
        if (a,b)==(terrace[3],terrace[4]):continue
        balustrade(m,a,b,0,.95,lod,M_TRIM)
    # The northern wall is at x=-18, not at the terrace's mean half-width.
    # Windows and coursing must remain outside that edge to survive depth testing.
    f=Mesh();_courses(f,-45,-18.12,34,-52,-.4,lod,GRANITE,2.0)
    for zz in (-56,-46,-35):
        for yy,w,h in [(-37,1.15,3.4),(-25,2.8,5.8),(-11,1.35,3.0)]:
            _lancet(f,zz,yy,-18.32,w,h,lod,M_TRIM,False,False)
    for zz in (-53,-40,-29):
        _box(f,zz,-19.0,-52,-23,2.0,1.9,GRANITE)
        _box(f,zz,-18.75,-23,-.4,1.25,1.3,GRANITE)
    for yy in (-31,-19,-4):_box(f,-45,-18.2,yy,yy+.28,34.4,.42,M_TRIM)
    _side(m,f,-1)
    # The southern edge tapers. Individual bays track the actual polygon line.
    for zz in (-56,-45,-34):
        edge=16-(zz+53)*4/25 if zz>=-53 else 12+(zz+62)*4/9
        f=Mesh()
        for yy in (-37,-25,-11):_lancet(f,zz,yy,-edge-.22,1.6,3.5,lod,M_TRIM,False,False)
        _side(m,f,1)
    # Romanesque nave: four surviving bays, lower side aisles, lead-colored slate.
    _box(m,0,-16,0,17,23,27,GRANITE)
    _box(m,0,-16,17,22,11.8,27,GRANITE)
    _roof(m,0,-16,12.5,27,22,28,lod)
    for s in (-1,1):
        _roof(m,s*8.6,-16,6.6,27,12.3,17,lod)
        for z in (-26,-19,-12,-5):
            f=Mesh();_lancet(f,z,15,-6.0,1.55,5.4,lod,M_TRIM,False,False);_side(m,f,s)
            _box(m,s*11.7,z,0,18,.9,1.2,GRANITE)
    _box(m,0,0,0,22,39.4,14,GRANITE)
    tr=Mesh();_roof(tr,0,0,14.5,39.4,22,28,lod);m.merge(turn(tr,90))
    # Gothic choir supported by the large-pillared crypt and radiating chapels.
    choir=[(-6.3,5),(6.3,5),(6.3,16),(4.5,22),(0,24),(-4.5,22),(-6.3,16)]
    plan_solid(m,choir,0,25,M_TRIM,STONE_WALL)
    roofpts=[(-6.8,5),(6.8,5),(6.8,16),(4.8,22),(0,24.5),(-4.8,22),(-6.8,16)]
    for a,b in zip(roofpts,roofpts[1:]+roofpts[:1]):face(m,[(a[0],25,a[1]),(b[0],25,b[1]),(0,31,12)],SLATE,SLATE_ROOF,(a[0]+b[0],1,a[1]+b[1]-24))
    for a in np.linspace(-math.pi/2,math.pi/2,7):
        x=10.7*math.sin(a);z=15+10.7*math.cos(a)
        lathe(m,(x,z),[(-22,3.1),(10,3.1),(12,2.8),(15,.03)],GRANITE,STONE_WALL,(12,10,8)[lod])
        f=Mesh();_lancet(f,0,1,-3.2,2.1,7.6,lod,M_TRIM);m.merge(turn(f,math.degrees(a)+180,x,0,z))
        _pinnacle(m,13.8*math.sin(a),15+13.8*math.cos(a),11,11,lod,M_TRIM)
        tube(m,(5.8*math.sin(a),23,15+5.8*math.cos(a)),(13.8*math.sin(a),17,15+13.8*math.cos(a)),.32,M_TRIM,STONE_WALL,6)
    for s in (-1,1):
        for z in (6,12,18):
            f=Mesh();_lancet(f,z,13,-6.5,3.0,10,lod,M_TRIM);_side(m,f,s)
    # Classical western facade, rebuilt1784: pediment and round-headed entrance.
    f=Mesh();_box(f,0,0,0,19,23.2,1.0,M_TRIM);_courses(f,0,-.53,23.2,0,19,lod,M_TRIM)
    _lancet(f,0,.1,-.7,3.6,6.2,lod,M_TRIM,False,False)
    for x in (-8.8,-4.6,4.6,8.8):_box(f,x,-.7,0,18,.7,.7,M_TRIM)
    _lancet(f,0,10,-.7,3.7,7,lod,M_TRIM,True,False)
    face(f,[(-11.6,19,-.6),(11.6,19,-.6),(0,27.5,-.6)],M_TRIM,STONE_WALL,(0,0,-1))
    m.merge(turn(f,0,0,0,-30))
    # Crossing tower and spire; gilded Michael is a small winged skyline figure.
    _box(m,0,0,22,38,12.0,12.0,GRANITE)
    for angle in (0,90,180,270):
        f=Mesh()
        for x in (-2.65,2.65):_lancet(f,x,29,-6.12,3.0,7.1,lod,M_TRIM,False,False)
        _box(f,0,-6.1,37,38,12.8,.8,M_TRIM)
        m.merge(turn(f,angle))
    lathe(m,(0,0),[(38,6.2),(41,5.2),(68,1.1),(72,.3)],SLATE,SLATE_ROOF,8,angle=math.pi/8)
    for i in range(8):
        a=i*TAU/8+math.pi/8
        tube(m,(5.3*math.cos(a),40,5.3*math.sin(a)),(.4*math.cos(a),72,.4*math.sin(a)),.11,M_TRIM,STONE_WALL,6)
        _pinnacle(m,5.5*math.cos(a),5.5*math.sin(a),39,8,lod,M_TRIM)
    gold=(200,166,62)
    lathe(m,(0,0),[(72,.3),(73,.65),(75.7,.4)],gold,SCULPTURE_METAL,8)
    m.ellipsoid((0,76.1,0),(.4,.52,.35),gold,SCULPTURE_METAL,8,4)
    for s in (-1,1):
        face(m,[(0,74.5,0),(s*2.0,76.3,.3),(s*1.7,74.1,.2)],gold,SCULPTURE_METAL,(0,0,-1))
        tube(m,(s*.3,75.4,0),(s*.8,76.3,0),.12,gold,SCULPTURE_METAL,5)
    tube(m,(.8,76,0),(1.1,78.5,0),.06,gold,SCULPTURE_METAL,5)
    # Northern Merveille: cellar/chaplaincy, scriptorium/guest hall, cloister/refectory.
    _box(m,-28.0,16.5,-45,-1,29,78,GRANITE)
    f=Mesh();_courses(f,16.5,-42.62,78,-44,-1,lod,GRANITE,2.6);_side(m,f,-1)
    # The cloister garden stays open; the refectory is the eastern roofed block.
    _box(m,-30,-3,-1,.1,18.5,24,(94,116,71))
    for x,z,w,d in [(-42,-3,4.8,32),(-17,-3,4.8,32),(-29.5,-17.2,20.5,4.4),(-29.5,11.2,20.5,4.4)]:
        _roof(m,x,z,w,d,4.1,6.0,lod)
    # Staggered paired Purbeck columns,137 shafts in the nearest LOD.
    for side in (-1,1):
        for z in np.linspace(-15.5,9.8,(22,14,8)[lod]):
            for off in (-.32,.32):column(m,-29.5+side*10.1+off,float(z),0,3.6,.115,(117,119,104),lod)
        for x in np.linspace(-38.7,-20.3,(12,9,6)[lod]):
            for off in (-.32,.32):column(m,float(x),-3+side*12.7+off,0,3.6,.115,(117,119,104),lod)
    if lod==0:column(m,-39.6,-15.7,0,3.6,.115,(117,119,104),lod)
    _mont_hall(m,-28,36,29,38,-1,8,17,lod)
    for z in np.arange(19,54,3.2):
        for s in (-1,1):
            f=Mesh();_lancet(f,float(z),.3,-14.66,1.35,6.8,lod,M_TRIM);_side(m,f,s,-28)
    for z in np.linspace(-20,55,16):
        _box(m,-44,float(z),-45,-29,2.8,2.3,GRANITE)
        _box(m,-43.55,float(z),-29,-14,1.9,1.9,GRANITE)
        _box(m,-43.15,float(z),-14,6,1.15,1.45,GRANITE)
        for yy in (-30,-18):
            f=Mesh();_lancet(f,float(z)+2.5,yy,-42.95,1.8,5.5,lod,M_TRIM,False,False);_side(m,f,-1)
    # The CMN northwest photograph shows the west Merveille end as rooms,
    # with a central large opening and three gabled upper gallery sections.
    _courses(m,-28,-22.64,29,-43,4,lod,GRANITE,1.8)
    for xx in (-39,-28,-17):
        for yy,w,h in [(-37,1.6,4.0),(-24,1.1,4.0),(-12,1.6,5.1)]:
            _lancet(m,xx,yy,-22.85,w,h,lod,M_TRIM,False,False)
        _box(m,xx,-22.9,-47,-1,.95,1.1,GRANITE)
    _lancet(m,-29,-14,-22.89,5.5,8.8,lod,M_TRIM,True,True)
    _lancet(m,-29,-38,-22.89,3.4,7.5,lod,M_TRIM,False,True)
    for yy in (-30,-19,-2):_box(m,-28,-22.8,yy,yy+.28,29.5,.45,M_TRIM)
    for xx in (-39,-29,-19):
        _box(m,xx,-21.8,-1,4.8,8.4,2.1,GRANITE)
        face(m,[(xx-4.2,4.8,-22.9),(xx+4.2,4.8,-22.9),(xx,11.0,-22.9)],GRANITE,STONE_WALL,(0,0,-1))
        for dx in (-1.45,1.45):_lancet(m,xx+dx,.4,-23.08,1.25,4.4,lod,M_TRIM,False,True)
    # The monks' north-south passage sits between the terrace and cloister.
    passage=Mesh();_box(passage,0,0,-38.4,3,10,13,GRANITE)
    _roof(passage,0,0,10.4,13.5,3,10,lod)
    for yy in (-13,-5):_lancet(passage,0,yy,-6.72,2.3,5.5,lod,M_TRIM,False,False)
    m.merge(turn(passage,90,-18.8,0,-28.2))
    # The photographed passage bears on an exposed granite shoulder between
    # terrace and Merveille. Its perimeter joins eight production ground samples.
    outer=[(-28,-47.105,-36),(-19,-47.079,-36),(-10,-45.951,-36),
           (-10,-44.525,-28),(-10,-43.273,-20),(-19,-44.352,-20),
           (-28,-45.379,-20),(-28,-46.989,-28)]
    inner=[(-25.6,-38,-33.5),(-19,-38,-33.5),(-12,-38,-33.5),
           (-12,-38,-28),(-12,-38,-22.9),(-19,-38,-22.9),
           (-25.6,-38,-22.9),(-25.6,-38,-28)]
    face(m,inner,(127,128,119),4,(0,1,0))
    for i in range(8):
        a,b=np.array(outer[i]),np.array(outer[(i+1)%8]);d,c=np.array(inner[i]),np.array(inner[(i+1)%8])
        for j in range((4,3,2)[lod]):
            u=j/(4,3,2)[lod];v=(j+1)/(4,3,2)[lod]
            lo=a*(1-u)+d*u;hi=a*(1-v)+d*v;lr=b*(1-u)+c*u;hr=b*(1-v)+c*v
            for p,t in [(lo,u),(lr,u),(hi,v),(hr,v)]:p[1]+=.55*math.sin(p[0]*.7+p[2]*.4)*math.sin(math.pi*t)
            face(m,[lo,lr,hr,hi],(126+i%3,127+i%3,117+i%3),4,(float(a[0]+b[0]+38),1,float(a[2]+b[2]+56)))
    # South-facing abbots' lodgings, stepped roofs, chatelet and round turrets.
    for spec in [(27,7,13,19,-47,-12,-3,0),(28,26,13,19,-47,-15,-5,-12),(20,41,14,15,-47,-17,-7,-25),(22,-15,12,17,-47,-15,-5,0),(-4,-43,15,26,-46,-15,-6,0)]:
        _mont_hall(m,*spec[:-1],lod,angle=spec[-1])
    for x,z,r,y in [(15,43,3,-30),(25,42,3,-30),(-42,55,2.7,-46)]:
        lathe(m,(x,z),[(y,r),(-10,r),(-8,r*1.2),(1,.03)],GRANITE,STONE_WALL,(20,14,10)[lod])
    for j in range((55,34,20)[lod]):
        t=j/(55,34,20)[lod];x=17+8*t;z=-20+61*t;y=-.5-29*t
        _box(m,x,z,y-1.2,y,4.8,2.0,M_TRIM)
    return retag(m,{11:STONE_WALL,17:STONE_WALL})


def _lajas(lod):
    m=Mesh();front=-18.5
    # A narrow cliff supports the rock-backed apse. The outer rows rejoin the
    # sampled DEM; the near-vertical face is unresolved by its coarse pixels.
    # Finer irregular rock faces avoid a regular striped retaining-wall effect.
    xs=np.linspace(-27,25,(23,17,11)[lod])
    zs=np.array((17.9,24,35,45,55,68,80))
    left=np.array((-15,-13,-9,-5,0,12,26))
    right=np.array((-23,-20,-18,-12,-6,1,6))
    crest=np.array((8,11,15,18,22,25,27))
    ridge=np.array((-5,-8,-2,4,-1,5,0))
    depth_rows=(22,16,10)[lod]
    rows=[]
    for j,z0 in enumerate(np.linspace(zs[0],zs[-1],depth_rows)):
        l=float(np.interp(z0,zs,left));r=float(np.interp(z0,zs,right));peak=float(np.interp(z0,zs,crest))
        row=[]
        for x0 in xs:
            u=(x0+27)/52;rx=float(np.interp(z0,zs,ridge))
            t=(x0+27)/(rx+27) if x0<rx else (25-x0)/(25-rx)
            blend=math.sin(math.pi*.5*max(0,t))**1.4
            y=l+(r-l)*u+(peak-(l+r)/2)*blend
            # Broad eroded ledges vary the ridge line; end rows remain on the DEM.
            wave=math.sin(x0*.29+z0*.16)*math.sin(x0*.13-z0*.27)
            y+=3.8*wave*blend*math.sin(math.pi*(z0-zs[0])/(zs[-1]-zs[0]))
            z=z0+1.7*math.sin(x0*.37)*max(0,1-(z0-zs[0])/20)
            row.append(np.array((x0,y,z)))
        rows.append(row)
    def tone(p):
        v=3.5*math.sin(p[0]*.14+p[1]*.09)+2.0*math.cos(p[2]*.21-p[0]*.08)
        return tuple(int(c+v)for c in (109,114,99))
    for j in range(len(rows)-1):
        for i in range(len(xs)-1):
            a,b,c,d=rows[j][i],rows[j][i+1],rows[j+1][i+1],rows[j+1][i]
            for tri in [(a,b,c),(a,c,d)]:
                center=sum(tri)/3;normal=np.cross(tri[1]-tri[0],tri[2]-tri[0]);normal/=np.linalg.norm(normal)
                vegetated=abs(normal[1])>.62 and center[2]>25
                face(m,tri,(107,132,70) if vegetated else tone(center),3 if vegetated else 4,(0,1,0))
    face_rows=(18,13,8)[lod]
    cliff_grid=[]
    for j in range(face_rows+1):
        t=j/face_rows;row=[]
        for p in rows[0]:
            x,top,z=p;y=-43+(top+43)*t
            recess=(3.0*math.sin(x*.22+y*.18)+1.2*math.sin(x*.57-y*.37))*math.sin(math.pi*t)
            row.append(np.array((x,y,z+recess)))
        cliff_grid.append(row)
    for j in range(face_rows):
        for i in range(len(xs)-1):
            a,b,c,d=cliff_grid[j][i],cliff_grid[j][i+1],cliff_grid[j+1][i+1],cliff_grid[j+1][i]
            face(m,[a,b,c],tone((a+b+c)/3),4,(0,0,-1));face(m,[a,c,d],tone((a+c+d)/3),4,(0,0,-1))
    # The northwest chapel bears on the bank seen in the2023 survey photo.
    # This tongue stays north of the mapped river corridor; the coarse DEM and
    # riverbed feather cannot resolve its near-vertical rock face.
    bank_x=np.array((-24,-20,-16,-13,-10,-6.8))
    bank_z=np.array((-8.2,-3,5,12,17.9))
    front_top=np.array((-16,-11,-7.5,-6.5,-6.5,-6.5))
    bank=[]
    for z in bank_z:
        blend=(z+8.2)/26.1;row=[]
        for i,x in enumerate(bank_x):
            back=float(np.interp(x,[p[0]for p in rows[0]],[p[1]for p in rows[0]]))
            t=blend if i<2 else max(0,(z-12)/5.9)
            y=front_top[i]*(1-t)+back*t
            if i not in (0,5):y+=.35*math.sin(x*.7+z*.4)*math.sin(math.pi*t)
            row.append(np.array((x,y,z)))
        bank.append(row)
    for arow,brow in zip(bank,bank[1:]):
        for i in range(len(bank_x)-1):
            a,b,c,d=arow[i],arow[i+1],brow[i+1],brow[i]
            for tri in [(a,b,c),(a,c,d)]:face(m,tri,tone(sum(tri)/3),4,(0,1,0))
    front_bottom=[-52.4,-52.4,-52.4,-52.4,-52.344,-52.174]
    for i in range(len(bank_x)-1):
        a,b=bank[0][i],bank[0][i+1]
        for j in range((10,7,4)[lod]):
            lo=j/(10,7,4)[lod];hi=(j+1)/(10,7,4)[lod]
            def q(p,y,t):
                return (p[0]+.25*math.sin(t*9+p[0])*math.sin(math.pi*t),y+(p[1]-y)*t,-8.6+.4*t)
            tri=[q(a,front_bottom[i],lo),q(b,front_bottom[i+1],lo),q(b,front_bottom[i+1],hi),q(a,front_bottom[i],hi)]
            face(m,tri,tone(sum(np.array(p)for p in tri)/4),4,(0,0,-1))
    side_bottom=[-52.4,-49.804,-33.996,-20.637,-16.852]
    for i in range(len(bank_z)-1):
        a,b=bank[i][0],bank[i+1][0]
        face(m,[(a[0],side_bottom[i],a[2]),(b[0],side_bottom[i+1],b[2]),b,a],tone((a+b)/2),4,(-1,0,0))
    # Entrance datum is the bridge deck. The two open barrel arches descend into
    # the gorge;100m is the river-to-spire composition, not a100m nave.
    _box(m,.7,-38.4,-1.2,0,17,39.8,LAJAS)
    for z in (-58.3,-38.4,-18.5):_box(m,.7,z,-54,-43,17.8,2.8,LAJAS)
    for z in (-28.45,-48.35):
        archmesh=Mesh();arch_bay(archmesh,(0,-43,0),19.9,41.8,16.0,1.4,LAJAS,lod,False)
        m.merge(turn(archmesh,90,.7,0,z))
    for side in (-1,1):
        f=Mesh()
        for z in (-58.3,-38.4,-18.5):
            _box(f,z,-.05,-45,-.8,2.0,.6,L_TRIM)
            _pinnacle(f,z,-.05,1.2,2.5,lod,L_TRIM)
        _side(m,f,side,.7+side*8.3)
        balustrade(m,(.7+side*8.3,-58.3),(.7+side*8.3,-18.5),0,1.25,lod,L_TRIM)
        for z in np.linspace(-55,-22,8):
            _box(m,.7+side*8.3,float(z),0,1.8,.65,.65,L_TRIM)
            if lod<2:
                lathe(m,(.7+side*8.3,float(z)),[(1.8,.23),(2.6,.17)],L_TRIM,STONE_WALL,8)
                m.ellipsoid((.7+side*8.3,2.8,float(z)),(.17,.2,.17),L_TRIM,STONE_WALL,8,4)
    # The mapped horseshoe is a bridge path at the entrance datum. Native DEM
    # bridge profiles put it inside the crypt, so this connected walk owns it.
    walkway=[(-8.7,-7.3),(-8.2,-19.4),(1.5,-19.3),(10.0,-19.2),(10.0,-16.5),(14.8,-2.7)]
    for a,b in zip(walkway,walkway[1:]):
        d=np.array(b)-np.array(a);length=np.linalg.norm(d);normal=np.array((-d[1],d[0]))/length
        pa=np.array(a);pb=np.array(b)
        ring=[pa-normal*1.15,pb-normal*1.15,pb+normal*1.15,pa+normal*1.15]
        plan_solid(m,[tuple(p)for p in ring],-.45,.05,LAJAS,STONE_WALL)
        if abs(d[1])>3:
            # Choose the outward side by its distance from the church axis.
            side=1 if abs((pa+normal*1.15)[0]-.7)>abs((pa-normal*1.15)[0]-.7)else -1
            balustrade(m,pa+normal*side*1.15,pb+normal*side*1.15,.05,1.1,lod,L_TRIM)
            for t in (.2,.6):
                p=pa+(pb-pa)*t
                tube(m,(max(-7.3,min(8.7,p[0])),-3.0,p[1]),(p[0],-.5,p[1]),.26,LAJAS,STONE_WALL,6)
    # Rock-backed apse and crypt, with three horizontal levels visible from river.
    _box(m,.7,-1.2,-24,0,16,34.6,LAJAS)
    _box(m,.7,-4,-54,-24,15.5,29,LAJAS)
    for y in (-24,-16,-8,-.6):_box(m,.7,-1.2,y,y+.35,16.9,35.2,L_TRIM)
    for side in (-1,1):
        f=Mesh();_courses(f,-1.2,-8.05,34.6,-42,0,lod,LAJAS,1.1)
        for zz in (-15,-9,-3,3,9,14):
            for yy,hh in [(-35,8),(-20,3.5),(-13,3.0),(-7,4.0)]:
                _lancet(f,zz,yy,-8.2,1.8,hh,lod,L_TRIM,False,False)
        _side(m,f,side,.7)
    _box(m,.7,-2,0,12,15.8,33,LAJAS)
    _box(m,.7,-1,12,19,6.5,29,LAJAS)
    _roof(m,.7,-1,7.2,29,19,22,lod,(106,111,107),METAL_ROOF)
    for s in (-1,1):
        _roof(m,.7+s*5.7,-1,4.7,29,10.2,13,lod,(118,124,117),METAL_ROOF)
        f=Mesh();_courses(f,-1.5,-8.02,33,0,12,lod,LAJAS,1.0)
        for z in (-13,-8,-3,2,7,12):
            _lancet(f,z,1,-8.2,3.1,8.8,lod,L_TRIM)
            _lancet(f,z,13.1,-3.45,2.6,5.0,lod,L_TRIM)
        _side(m,f,s,.7)
        for z in (-16,-11,-6,-1,4,9,14):
            _box(m,.7+s*8,z,-8,12,.65,1.0,L_TRIM)
            _pinnacle(m,.7+s*8,z,12,5.0,lod,L_TRIM)
            _flying(m,z,s,3.6,8,18,11,lod,L_TRIM)
    # The grey body is articulated by pale limestone bands and white pinnacles.
    _box(m,.7,front+.7,0,11.5,16.5,2.0,LAJAS)
    _box(m,.7,front+.7,11.5,20.5,6.6,2.0,LAJAS)
    f=Mesh();_courses(f,.7,front-.33,16.5,0,11.5,lod,LAJAS,1.0)
    _courses(f,.7,front-.33,6.6,11.5,20.5,lod,LAJAS,1.0)
    for xx,w,h in [(.7,4.7,8),(-5.5,3.0,5),(6.9,3.0,5)]:
        for k in range((4,3,1)[lod]):arch(f,(xx,0,front-.55-.15*k),w+.5*k,h+.55*k,.16,L_TRIM,True,(24,16,8)[lod],DARK if k==0 else None,STONE_WALL)
        for j in (-1,1):_pinnacle(f,xx+j*(w/2+.5),front-.5,h-1,4,lod,L_TRIM)
    _rose(f,.7,15.7,front-.57,2.9,lod,L_TRIM)
    for yy in (4.2,9.6,11.4,20.0):_box(f,.7,front-.55,yy,yy+.23,17.4 if yy<12 else 7.1,.65,L_TRIM)
    for xx in (-7.55,-3.0,4.4,8.95):_box(f,xx,front-.52,0,11.6,.43,.62,L_TRIM)
    m.merge(f)
    # Central tower is taller than the two flanking spires, as measured elevation shows.
    _box(m,.7,front+2.5,20.4,31.8,6.0,6.0,LAJAS)
    for angle in (0,90,180,270):
        f=Mesh()
        for xx in (-1.6,0,1.6):_lancet(f,xx,22,-3.15,1.25,7.0,lod,L_TRIM)
        _rose(f,0,26.5,-3.2,1.1,lod,L_TRIM)
        for xx in (-3,3):_pinnacle(f,xx,-3,29.5,5,lod,L_TRIM)
        m.merge(turn(f,angle,.7,0,front+2.5))
    for yy in (21,30.8,32):_box(m,.7,front+2.5,yy,yy+.3,6.7,6.7,L_TRIM)
    _open_spire(m,.7,front+2.5,32.5,45.8,2.7,lod,L_TRIM)
    for x in (-5.7,7.1):
        _box(m,x,front+1.0,11.5,16.1,3.2,3.2,LAJAS)
        for a in (0,90,180,270):
            f=Mesh();_lancet(f,0,12,-1.7,2.0,3.2,lod,L_TRIM,False);m.merge(turn(f,a,x,0,front+1))
        _open_spire(m,x,front+1,16.2,24.8,1.65,lod,L_TRIM)
        for dx in (-1.8,1.8):_pinnacle(m,x+dx,front-.8,11.6,4.3,lod,L_TRIM)
    # Northern side chapel and ex-voto retaining wall stay asymmetric.
    _box(m,-10,-.5,-7,9,5.2,14.5,LAJAS)
    _roof(m,-10,-.5,5.7,14.5,9,11.8,lod,(117,123,119),METAL_ROOF)
    f=Mesh();_courses(f,0,-.1,33,0,7,lod,LAJAS)
    for xx in np.arange(-14,16,2.0):
        if lod<2:face(f,[(xx-.5,2,-.13),(xx+.5,2,-.13),(xx+.5,2.7,-.13),(xx-.5,2.7,-.13)],(189,184,160),STONE_WALL,(0,0,-1))
    m.merge(turn(f,0,-6,0,17.2))
    # The plaques form a retaining wall against the cliff. Variable-depth
    # foundations overlap the rock face instead of leaving its ends cantilevered.
    for a,b in zip(np.linspace(-22.5,10.5,12)[:-1],np.linspace(-22.5,10.5,12)[1:]):
        x=(a+b)/2
        rock_top=float(np.interp(x,[v[0] for v in rows[0]],[v[1] for v in rows[0]]))
        _box(m,float(x),17.5,min(-3,rock_top-2),7,float(b-a)+.02,1.2,LAJAS)
        if rock_top<-3:
            _box(m,float(x),18.15,rock_top-2,0,float(b-a)+.1,2.2,LAJAS)
    # Small far-abutment shrine; bridge remains visibly separate from the canyon.
    _box(m,.7,-60,-.9,4.0,3.0,2.3,L_TRIM)
    _roof(m,.7,-60,3.3,2.6,4,5.7,lod)
    _lancet(m,.7,.8,-61.25,1.3,2.6,lod,L_TRIM,False)
    # Preserve the mapped lower crossing with a bank-supported deck; the native
    # 64m height lattice spans over the gorge and cannot supply its abutment datum.
    import json
    from pathlib import Path
    foot=json.loads((Path(__file__).parent/'references/sacred_gothic/las-lajas-footbridge.json').read_text())
    base=foot['datum_m'];floor=foot['bridge_floor_relative_m']
    ground=[p['corrected_ground_m']-base for p in foot['ground_samples']]
    def support(poly,top,toes,color=LAJAS):
        center=np.mean(poly,axis=0)
        face(m,[(x,top,z)for x,z in poly],color,STONE_WALL)
        for i,j in [(0,1),(1,2),(2,3),(3,0)]:
            x,z=poly[i];xx,zz=poly[j];normal=((x+xx)/2-center[0],0,(z+zz)/2-center[1])
            face(m,[(x,toes[i],z),(xx,toes[j],zz),(xx,top,zz),(x,top,z)],color,STONE_WALL,normal)
    for i,poly in enumerate(foot['abutment_polygons']):
        support(poly,floor-.15,[min(floor-.5,g-.5)for g in ground[i*4:i*4+4]])
    aa,bb=np.array(foot['bridge_endpoints_xz_m']);direction=(bb-aa)/np.linalg.norm(bb-aa);normal=np.array([-direction[1],direction[0]])
    for i in range(12):
        ta,tb=i/12,(i+1)/12;a=aa+(bb-aa)*ta;b=aa+(bb-aa)*tb
        ya=floor+.2*math.sin(math.pi*ta);yb=floor+.2*math.sin(math.pi*tb)
        poly=[a-normal*1.2,b-normal*1.2,b+normal*1.2,a+normal*1.2]
        face(m,[(p[0],y,p[1])for p,y in zip(poly,[ya,yb,yb,ya])],(169,167,154),STONE_WALL)
        face(m,[(p[0],y-.35,p[1])for p,y in zip(poly,[ya,yb,yb,ya])],LAJAS,STONE_WALL,(0,-1,0))
        for side in (-1,1):
            p=a+normal*1.2*side;q=b+normal*1.2*side
            face(m,[(p[0],ya-.35,p[1]),(q[0],yb-.35,q[1]),(q[0],yb,q[1]),(p[0],ya,p[1])],LAJAS,STONE_WALL,(normal[0]*side,0,normal[1]*side))
            for h in (0.5,1.05):tube(m,(p[0],ya+h,p[1]),(q[0],yb+h,q[1]),.045,(76,81,77),METAL_ROOF,6 if lod==0 else 4)
            if i%(1 if lod==0 else 2)==0:tube(m,(p[0],ya,p[1]),(p[0],ya+1.05,p[1]),.05,(76,81,77),METAL_ROOF,6 if lod==0 else 4)
    rows=np.asarray(foot['approach_rows']);grades=np.asarray(ground[8:8+2*len(rows)]).reshape(-1,2)
    tops=np.asarray(foot['approach_tops_relative_m'])
    for i,(a,b) in enumerate(zip(rows,rows[1:])):
        top=max(tops[i],tops[i+1]);poly=[a[0],b[0],b[1],a[1]]
        support(poly,float(top),[float(min(top-.58,g-.5)) for g in [grades[i,0],grades[i+1,0],grades[i+1,1],grades[i,1]]],(154,154,143))
        for side in (0,1):
            p,q=a[side],b[side]
            tube(m,(p[0],tops[i]+1.0,p[1]),(q[0],tops[i+1]+1.0,q[1]),.045,(76,81,77),METAL_ROOF,4)
            if i%(3 if lod==0 else 6)==0:tube(m,(p[0],top,p[1]),(p[0],tops[i]+1.0,p[1]),.05,(76,81,77),METAL_ROOF,4)
    return retag(m,{11:STONE_WALL,17:STONE_WALL})


_BUILDERS={'cologne-cathedral':_cologne,'mont-saint-michel-abbey':_mont,'las-lajas-sanctuary':_lajas}

def build(name,lod):
    if lod not in (0,1,2):raise ValueError('LOD must be0,1,2')
    m=_BUILDERS[name](lod)
    if name=='cologne-cathedral':m.transform([[1,0,0,0],[0,1,0,0],[0,0,144.58/148.19471425,-1.0],[0,0,0,1]])
    return m


ENTRIES = [{'name': 'cologne-cathedral',
  'city': 'Cologne',
  'lat': 50.941312623858984,
  'lon': 6.958206784059214,
  'heading': 88.5,
  'base': 59.092,
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://www.koelner-dom.de/erleben/der-dom-in-zahlen',
                 'https://api.koelner-dom.de/uploads/2407_KW_Cologne_Cathedral_EN_9c90f29f5c.pdf'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Footprint and terrain placement evidence: © '
                 'OpenStreetMap contributors (ODbL), via Protomaps.',
  'terrain_flatten': [{'center_xz': [0, -7],
                       'half_extent': [26, 70],
                       'feather_m': 5,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 59.092},
                      {'center_xz': [0, 0],
                       'half_extent': [42, 20],
                       'feather_m': 4,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 59.092}],
  'terrain_cuts': [
      {'polygon_xz': [[-33.2,-79.6],[33.2,-79.6],[33.2,-51.7],[-33.2,-51.7]], 'plane':[0,0,-.12], 'feather_m':.6},
      {'polygon_xz': [[-28.2,-54],[28.2,-54],[28.2,38.1],[-28.2,38.1]], 'plane':[0,0,-.12], 'feather_m':.6},
      {'polygon_xz': [[-44.5,-22.3],[44.5,-22.3],[44.5,20.3],[-44.5,20.3]], 'plane':[0,0,-.12], 'feather_m':.6},
      {'polygon_xz': [[28.2, 38.02433], [27.65814, 43.39169], [26.0534, 48.55278], [23.44744, 53.30927], [19.94041, 57.47837], [15.66708, 60.89985], [10.79167, 63.44225], [5.50155, 65.00785], [0.0, 65.53649], [-5.50155, 65.00785], [-10.79167, 63.44225], [-15.66708, 60.89985], [-19.94041, 57.47837], [-23.44744, 53.30927], [-26.0534, 48.55278], [-27.65814, 43.39169], [-28.2, 38.02433]], 'plane':[0,0,-.12], 'feather_m':.6}],
  'max_source_building_span_m': 200,
  'description': 'Twin openwork157m spires, octagonal galleries, five-aisled nave, transept facades, seven '
                 'radiating chapels, flying buttresses and lead-panel roofs.',
  'capture': {'distance': 245,
              'azimuth': 240,
              'elevation': 22,
              'target_height': 55,
              'time_utc': '2026-06-20T13:30:00Z'},
  'source_exclusion_polygons': [[[-33.2, -80], [33.2, -80], [33.2, -52], [-33.2, -52]],
                                [[-28.2, -54], [28.2, -54], [28.2, 40], [-28.2, 40]],
                                [[-44.5, -22.3], [44.5, -22.3], [44.5, 20.3], [-44.5, 20.3]],
                                [[-28, 39], [28, 39], [28, 68], [-28, 68]]],
  'road_exclusion_polygons': [[[-33.2, -80], [33.2, -80], [33.2, -52], [-33.2, -52]],
                              [[-28.2, -54], [28.2, -54], [28.2, 40], [-28.2, 40]],
                              [[-44.5, -22.3], [44.5, -22.3], [44.5, 20.3], [-44.5, 20.3]],
                              [[-28, 39], [28, 39], [28, 68], [-28, 68]],
                              [[-52.2,-10],[-44.3,-10],[-44.3,8],[-52.2,8]]],
  'activation_bounds_xz': [[-54, -85], [48.2, 73]],
  'audit': {'survey_file': 'tools/heritage_catalog/references/sacred_gothic/cologne-cathedral-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/sacred_gothic/cologne-cathedral-ground.json',
            'ground_datum_m': 59.092,
            'survey_anchor_offset_east_north_m': [-5.2, 3.85],
            'architectural_dimensions_m': [86.25, 157.22, 144.58],
            'height_authority': 'Published dimensions or independently scaled architectural elevation; tile '
                                'building heights are not authoritative.'}},
 {'name': 'mont-saint-michel-abbey',
  'city': 'Le Mont-Saint-Michel',
  'lat': 48.63601135752507,
  'lon': -1.511576586624644,
  'heading': 75,
  'base': 78.6,
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://www.abbaye-mont-saint-michel.fr/decouvrir/l-eglise-abbatiale',
                 'https://www.abbaye-mont-saint-michel.fr/decouvrir/s-orienter-dans-le-monument',
                 'https://www.abbaye-mont-saint-michel.fr/en/discover/the-cloister-between-sky-and-sea'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Footprint and terrain placement evidence: © '
                 'OpenStreetMap contributors (ODbL), via Protomaps.',
  'terrain_flatten': [],
  'terrain_cuts': [],
  'max_source_building_span_m': 200,
  'description': 'Romanesque nave and Gothic choir over crypts, neo-Gothic fleche with gilded Michael, west '
                 'terrace, elevated open cloister, Merveille and stepped southern lodgings.',
  'capture': {'distance': 205,
              'azimuth': 230,
              'elevation': 28,
              'target_height': 15,
              'time_utc': '2026-06-20T13:30:00Z'},
  'source_exclusion_polygons': [[[-46, -24], [-12, -24], [-12, 59], [-46, 59]],
                                [[-20, -64], [16, -64], [16, -25], [-20, -25]],
                                [[-21, -30], [22, -30], [22, 36], [-21, 36]],
                                [[12, -25], [38, -25], [38, 46], [12, 46]]],
  'road_exclusion_polygons': [[[-46, -24], [-12, -24], [-12, 59], [-46, 59]],
                              [[-20, -64], [16, -64], [16, -25], [-20, -25]],
                              [[-21, -30], [22, -30], [22, 36], [-21, 36]],
                              [[12, -25], [38, -25], [38, 46], [12, 46]]],
  'activation_bounds_xz': [[-51, -69], [43, 64]],
  'audit': {'survey_file': 'tools/heritage_catalog/references/sacred_gothic/mont-saint-michel-abbey-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/sacred_gothic/mont-saint-michel-abbey-ground.json',
            'ground_datum_m': 78.6,
            'survey_anchor_offset_east_north_m': [-16.65, -23.2],
            'architectural_dimensions_m': [83, 78.5, 122],
            'height_authority': 'Published dimensions or independently scaled architectural elevation; tile '
                                'building heights are not authoritative.'}},
 {'name': 'las-lajas-sanctuary',
  'city': 'Ipiales',
  'lat': 0.805561,
  'lon': -77.58602583477384,
  'heading': 40.2,
  'base': 2615.905,
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['http://www.sciresit.it/article/view/13845',
                 'https://narino.gov.co/recorrido-virtual-templo-de-narino/'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Footprint and terrain placement evidence: © '
                 'OpenStreetMap contributors (ODbL), via Protomaps.',
  'terrain_flatten': [],
  'terrain_cuts': [{'polygon_xz': [[-18, -35], [20, -35], [20, -15], [-18, -15]],
                    'plane': [0, 0, -45],
                    'feather_m': 20},
                   {'polygon_xz': [[-8.9,-57],[9.5,-57],[9.5,-35],[-8.9,-35]], 'plane':[0,-1.25,-84.25], 'feather_m':2}] + [{'polygon_xz': [[-83.809, -3.626], [-30.688, -26.269], [-22.845, -7.871], [-75.966, 14.772]], 'plane': [0, 0, -52], 'feather_m': 22}, {'polygon_xz': [[-34.191, -25.234], [14.27, -33.642], [17.689, -13.936], [-30.772, -5.528]], 'plane': [0, 0, -52], 'feather_m': 22}, {'polygon_xz': [[13.428, -33.708], [47.865, -21.905], [41.381, -2.986], [6.944, -14.789]], 'plane': [0, 0, -52], 'feather_m': 22}, {'polygon_xz': [[45.864, -23.028], [76.64, 2.52], [63.865, 17.908], [33.089, -7.64]], 'plane': [0, 0, -52], 'feather_m': 22}],
  'water_cuts': [{'polygon_xz': [[-83.025, -1.786], [-29.903, -24.43], [-23.629, -9.711], [-76.751, 12.933]], 'plane': [0, 0, -49], 'feather_m': 20}, {'polygon_xz': [[-33.849, -23.263], [14.612, -31.671], [17.347, -15.907], [-31.114, -7.499]], 'plane': [0, 0, -49], 'feather_m': 20}, {'polygon_xz': [[12.78, -31.817], [47.217, -20.013], [42.029, -4.877], [7.592, -16.681]], 'plane': [0, 0, -49], 'feather_m': 20}, {'polygon_xz': [[44.587, -21.49], [75.362, 4.059], [65.142, 16.37], [34.367, -9.179]], 'plane': [0, 0, -49], 'feather_m': 20}],
  'max_source_building_span_m': 200,
  'description': 'Three unequal Gothic spires above a rock-backed sanctuary, three crypt levels,17m-wide '
                 'twin-arch bridge and bounded canyon excavation.',
  'capture': {'distance': 140,
              'azimuth': 210,
              'elevation': 17,
              'target_height': -5,
              'time_utc': '2026-03-20T17:00:00Z'},
  'source_exclusion_polygons': [[[-13, -20], [11, -20], [11, 18], [-13, 18]],
                                [[-23, 8], [-12, 8], [-12, 18], [-23, 18]],
                                [[-9, -62], [10, -62], [10, -18], [-9, -18]]] + [[[-10.931, -6.592], [-10.365, -20.29], [-5.969, -20.108], [-6.535, -6.41]], [[-8.977, -21.608], [2.323, -21.492], [2.277, -17.092], [-9.023, -17.208]], [[0.726, -21.509], [10.826, -21.39], [10.774, -16.991], [0.674, -17.11]], [[12.2, -20.0], [12.2, -15.7], [7.8, -15.7], [7.8, -20.0]], [[11.815, -17.978], [17.141, -2.667], [12.985, -1.222], [7.659, -16.533]]],
  'road_exclusion_polygons': [[[-13, -20], [11, -20], [11, 18], [-13, 18]],
                              [[-23, 8], [-12, 8], [-12, 18], [-23, 18]],
                              [[-9, -62], [10, -62], [10, -18], [-9, -18]]] + [[[-10.931, -6.592], [-10.365, -20.29], [-5.969, -20.108], [-6.535, -6.41]], [[-8.977, -21.608], [2.323, -21.492], [2.277, -17.092], [-9.023, -17.208]], [[0.726, -21.509], [10.826, -21.39], [10.774, -16.991], [0.674, -17.11]], [[12.2, -20.0], [12.2, -15.7], [7.8, -15.7], [7.8, -20.0]], [[11.815, -17.978], [17.141, -2.667], [12.985, -1.222], [7.659, -16.533]]],
  'activation_bounds_xz': [[-112, -69], [105, 85]],
  'audit': {'survey_file': 'tools/heritage_catalog/references/sacred_gothic/las-lajas-sanctuary-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/sacred_gothic/las-lajas-sanctuary-ground.json',
            'ground_datum_m': 2615.905,
            'survey_anchor_offset_east_north_m': [-28, 0],
            'architectural_dimensions_m': [17, 46, 35],
            'height_authority': 'Published dimensions or independently scaled architectural elevation; tile '
                                'building heights are not authoritative.'}}]

# Exact lower pedestrian crossing and its stair approach are now authored.
_lajas_entry=next(e for e in ENTRIES if e['name']=='las-lajas-sanctuary')
import json as _json
from pathlib import Path as _Path
_lajas_foot=_json.loads((_Path(__file__).parent/'references/sacred_gothic/las-lajas-footbridge.json').read_text())
_lajas_entry['terrain_cuts']+=_lajas_foot['approach_terrain_cuts']
_lajas_entry['road_exclusion_polygons']+=_lajas_foot['road_exclusion_polygons']
_lajas_entry['source_exclusion_polygons']+=_lajas_foot['road_exclusion_polygons']+_lajas_foot['abutment_polygons']
_lajas_entry['references']+=['https://laslajas.org/guia-turistica/','https://laslajas.org/aeropuerto-antonio-narino/']
_lajas_entry['audit']['lower_footbridge']='Exact mapped25.0575m crossing and connecting stair route; deck clearance and support dimensions are explicitly estimated in references/sacred_gothic/las-lajas-footbridge.json. Footing toes use317 production corrected-ground samples with0.5m embedding.'
