"""Original static heritage architecture, authored from public dimensional references.

The metric meshes contain no downloaded geometry, textures, brands, or animation.
Placement and DEM evidence are recorded in output/city-catalog-expansion/heritage.
"""
import math
import numpy as np
from common import Mesh

TAU = math.tau
IVORY = (225, 221, 202)
MARBLE = (235, 233, 221)
DARK = (38, 53, 53)
GOLD = (207, 166, 66)
BLUE = (28, 79, 117)
TURQUOISE = (45, 149, 159)
RED = (156, 43, 35)
GREEN = (52, 78, 66)


def _face(m, p, color, material=11, normal=(0, 1, 0)):
    p = np.asarray(p, float)
    if np.dot(np.cross(p[1]-p[0], p[2]-p[0]), normal) < 0:
        p = p[::-1]
    for i in range(1, len(p)-1):
        m.triangle(p[0], p[i], p[i+1], color, material)


def _lathe(m, center, profile, color, material=11, n=32, angle=0., caps=True):
    """Profile rows are (height,radius); a clockwise ring gives outward walls."""
    if profile[-1][0] < profile[0][0]:
        profile = profile[::-1]
    cx, cz = center
    rings = [[(cx+r*math.cos(angle+j*TAU/n), y, cz+r*math.sin(angle+j*TAU/n))
              for j in range(n)] for y, r in profile]
    for k, (a, b) in enumerate(zip(rings, rings[1:])):
        for j in range(n):
            q = (j+1) % n
            c = color(k, j) if callable(color) else color
            m.quad(a[j], b[j], b[q], a[q], c, material)
    if caps:
        for ring, (y, r), normal in [(rings[0], profile[0], (0,-1,0)), (rings[-1], profile[-1], (0,1,0))]:
            if r > 0:
                for j in range(n):
                    _face(m, [(cx,y,cz),ring[j],ring[(j+1)%n]], color(0,j) if callable(color) else color, material, normal)


def _tube(m, a, b, radius, color, material=17, sides=6):
    # Joined decorative rails omit internal caps; all exposed long faces remain.
    a,b=np.asarray(a,float),np.asarray(b,float);axis=b-a;axis/=np.linalg.norm(axis)
    side=np.cross(axis,[0,1,0] if abs(axis[1])<.9 else [1,0,0]);side/=np.linalg.norm(side)
    up=np.cross(axis,side)
    ring=[radius*(math.cos(i*TAU/sides)*side+math.sin(i*TAU/sides)*up) for i in range(sides)]
    for i in range(sides):
        u,v=ring[i],ring[(i+1)%sides]
        m.quad(a+u,a+v,b+v,b+u,color,material)


def _ring(m, center, radius, tube, color, n=40, material=17, plane='xz'):
    c = np.array(center)
    for i in range(n):
        a,b=i*TAU/n,(i+1)*TAU/n
        d=lambda t: np.array((math.cos(t)*radius,0,math.sin(t)*radius)) if plane=='xz' else np.array((math.cos(t)*radius,math.sin(t)*radius,0))
        _tube(m,c+d(a),c+d(b),tube,color,material,6 if n>24 else 4)


def _arch_points(w, h, n=16, pointed=True):
    half=w/2; spring=h*(.52 if pointed else 1-w/(2*h))
    points=[(-half,0),(-half,spring)]
    for x in np.linspace(-half,half,n+1)[1:-1]:
        if pointed:
            rise=math.sqrt(max(0,w*w-(abs(x)+half)**2))/(math.sqrt(3)*half)
        else:
            rise=math.sqrt(max(0,1-(x/half)**2))
        points.append((float(x),spring+(h-spring)*rise))
    points.extend([(half,spring),(half,0)])
    return points


def _arch(m, center, w, h, trim=.3, color=IVORY, pointed=True, n=16, fill=DARK, material=17):
    """Recessed aperture facing local -z, with relief stone voussoirs."""
    x,y,z=center; p=_arch_points(w,h,n,pointed)
    if fill:
        # Fan center lies inside the convex arch; winding points toward -z.
        for a,b in zip(p,p[1:]+p[:1]):
            _face(m,[(x,y+h*.4,z+.04),(x+a[0],y+a[1],z+.04),(x+b[0],y+b[1],z+.04)],fill,15,(0,0,-1))
    if n <= 8:
        return
    for a,b in zip(p[:-1],p[1:]):
        _tube(m,(x+a[0],y+a[1],z),(x+b[0],y+b[1],z),trim,color,material,6 if n>12 else 4)


def _turn(mesh, angle, x=0., y=0., z=0.):
    a=math.radians(angle);c,s=math.cos(a),math.sin(a)
    return mesh.transform([[c,0,s,x],[0,1,0,y],[-s,0,c,z],[0,0,0,1]])


def _gable(m, x, z, w, depth, eave, peak, color, roof=(142,78,53)):
    m.box((x,eave/2,z),(w,eave,depth),color)
    for side in (-1,1):
        _face(m,[(x-w/2,eave,z+side*depth/2),(x+w/2,eave,z+side*depth/2),(x,peak,z+side*depth/2)],color,11,(0,0,side))
        _face(m,[(x+side*w/2,eave,z-depth/2),(x+side*w/2,eave,z+depth/2),(x,peak,z+depth/2),(x,peak,z-depth/2)],roof,12,(side,1,0))


def _dome(m, center, radius, rise, bottom, color, n=40, steps=10, pointed=False, ribs=None, polygon=None):
    cx,cz=center
    # A pointed meridian suits masonry domes; ellipsoidal shells suit metal roofs.
    profile=[]
    for j in range(steps+1):
        t=j/steps
        r=radius*(1-t*t) if pointed else radius*math.cos(t*math.pi/2)
        profile.append((bottom+rise*t,r if j<steps else .03))
    _lathe(m,(cx,cz),profile,color,12,n if polygon is None else polygon,angle=math.pi/8 if polygon==8 else 0)
    if ribs and not (polygon is None and n<=20 and radius<12):
        count=polygon or 12
        for i in range(count):
            a=i*TAU/count+(math.pi/8 if polygon==8 else 0)
            path=[(cx+(r+.07)*math.cos(a),y+.05,cz+(r+.07)*math.sin(a)) for y,r in profile]
            for p,q in zip(path,path[1:]):_tube(m,p,q,.23 if radius>12 else .12,ribs,17,6 if n>20 else 4)


def _stairs(m, center, width, run, rise, n=12, color=IVORY):
    x,z=center
    for i in range(n):
        h=rise*(i+1)/n
        m.box((x,h/2,z+run*(i+.5)/n),(width,h,run/n+.01),color,8)


def _balustrade(m, radius, y, n, gap=.1):
    # Radial stone posts stop around the four cardinal stair openings.
    for i in range(n):
        a=i*TAU/n
        if min(abs(math.sin(a)),abs(math.cos(a)))<gap:continue
        x,z=radius*math.cos(a),radius*math.sin(a)
        m.box((x,y+.55,z),(.38,1.1,.38),MARBLE,17)
        if n>40:
            m.ellipsoid((x,y+1.14,z),(.28,.2,.28),MARBLE,17,8,4)
        b=(i+1)*TAU/n
        if min(abs(math.sin(b)),abs(math.cos(b)))>=gap:
            for h in (.28,.9):m.beam((x,y+h,z),(radius*math.cos(b),y+h,radius*math.sin(b)),.13,MARBLE,17,5)


def _temple(lod):
    m=Mesh();n=(96,64,40)[lod]
    for radius,y in [(45,2.1),(36,4.2),(27,6.3)]:
        _lathe(m,(0,0),[(y-2.1,radius),(y,radius)],(205,205,193),17,n)
        _lathe(m,(0,0),[(y-.18,radius+.2),(y,radius+.2)],MARBLE,17,n)
        _balustrade(m,radius-.35,y,(112,64,28)[lod],.10)
    for angle in (0,90,180,270):
        st=Mesh()
        for start,end,top in [(-49,-43,2.1),(-40,-34,4.2),(-31,-25,6.3)]:
            _stairs(st,(0,start),7.7,end-start,top,(14,9,5)[lod])
        m.merge(_turn(st,angle))
    _lathe(m,(0,0),[(6.3,11.9),(18.4,11.9)],RED,11,n)
    for i in range(12):
        a=i*TAU/12
        col=Mesh();col.box((0,11.6,-12),(.76,10.6,.9),(166,48,37))
        col.box((0,10.3,-12.37),(3.5,6.6,.15),(57,65,47),15)
        for xx in [-1.75,1.75]:col.box((xx,10.3,-12.49),(.16,6.7,.18),RED)
        for yy in [7,10,13.6]:col.box((0,yy,-12.49),(3.6,.18,.18),RED)
        if lod<2:
            for xx in [-1.1,-.55,0,.55,1.1]:col.box((xx,11.7,-12.49),(.07,2.9,.13),GOLD,17)
        for yy in [15.2,16.5,17.6]:col.box((0,yy,-12.15),(5.9,.5,.7),BLUE)
        for dx in [-2,-1,0,1,2]:
            col.box((dx,16.4,-12.56),(.2,1.4,.15),GOLD,17)
        m.merge(_turn(col,math.degrees(a)))
    for radius,lo,hi,inner in [(18,18.3,24.6,8.5),(14.5,25.0,30.5,6.2),(10.8,31.0,37.0,.9)]:
        _lathe(m,(0,0),[(lo-.8,radius*.78),(lo-.1,radius*.92),(lo,radius)],(35,108,128),17,n)
        profile=[(lo,radius),(lo+.23,radius),(lo+.68,radius*.9),(lo+1.6,radius*.75),(hi-1.1,inner+1.4),(hi,inner)]
        _lathe(m,(0,0),profile,lambda k,j:(31+(j%3)*2,62+(j%3)*3,111+(j%3)*4),12,n)
        _ring(m,(0,lo+.18,0),radius,.13,(53,92,131),n,17)
        if lod<2:
            for i in range(n//2):
                a=i*TAU/(n//2)
                for (y,r),(yy,rr) in zip(profile,profile[1:]):m.beam((r*math.cos(a),y+.08,r*math.sin(a)),(rr*math.cos(a),yy+.08,rr*math.sin(a)),.04,(54,91,137),12,4)
    _lathe(m,(0,0),[(36.8,.8),(37.25,.9),(37.6,.5),(38.1,.2)],GOLD,16,20)
    return m


def _tile_border(m,x,y,z,w,h,lod,color=BLUE):
    m.box((x,y+h/2,z),(w,h,.24),color,17)
    for xx in [x-w/2+.35,x+w/2-.35]:m.box((xx,y+h/2,z-.15),(.28,h,.14),IVORY,17)
    for yy in [y+.35,y+h-.35]:m.box((x,yy,z-.15),(w,.28,.14),TURQUOISE,17)
    if lod<2:
        step=(2.0,3.1)[lod]
        for xx in np.arange(x-w/2+1,x+w/2-.5,step):
            for yy in np.arange(y+1,y+h-.5,step):
                _face(m,[(xx,yy+.42,z-.16),(xx+.42,yy,z-.16),(xx,yy-.42,z-.16),(xx-.42,yy,z-.16)],(189,179,121) if (int(xx+yy)%3==0) else TURQUOISE,17,(0,0,-1))


def _iwan(m, x,z,w,h,depth,lod):
    # Deep open entrance niche, with masonry spandrels around a pointed vault.
    archw=w*.56;archh=h*.80;wall=(w-archw)/2
    m.box((x-w/2+wall/2,h/2,z+depth/2),(wall,h,depth),(174,151,104))
    m.box((x+w/2-wall/2,h/2,z+depth/2),(wall,h,depth),(174,151,104))
    p=_arch_points(archw,archh,(22,16,10)[lod],True)[1:-1]
    for a,b in zip(p,p[1:]):
        _face(m,[(x+a[0],a[1],z),(x+b[0],b[1],z),(x+b[0],h,z),(x+a[0],h,z)],BLUE,17,(0,0,-1))
        _face(m,[(x+a[0],a[1],z),(x+a[0],a[1],z+depth),(x+b[0],b[1],z+depth),(x+b[0],b[1],z)],(125,149,148),17,(0,-1,0))
    _arch(m,(x,0,z+depth-.1),archw*.92,archh*.96,.25,TURQUOISE,True,18,(32,66,71))
    _arch(m,(x,0,z-.25),archw,archh,.45,IVORY,True,(26,18,12)[lod],None)
    for xx in [x-w/2+wall/2,x+w/2-wall/2]:_tile_border(m,xx,0,z-.12,wall-.15,h,lod)
    _tile_border(m,x,h-2.1,z-.16,w,2.1,lod)
    for xx in [x-w/2,x+w/2]:m.box((xx,h/2,z-.28),(.42,h,.5),TURQUOISE,17)


def _minaret(m,x,z,height,lod,r=2.1):
    n=(24,16,12)[lod]
    _lathe(m,(x,z),[(0,r*1.18),(2,r*1.12),(height-5,r*.75),(height-4,r*.96),(height-3.2,r*.96),(height-2.7,r*.65),(height,r*.62)],(170,146,103),11,n)
    for y in np.arange(3,height-4,4):_lathe(m,(x,z),[(y,r*(1-.25*y/height)+.07),(y+.65,r*(1-.25*y/height)+.07)],BLUE,17,n)
    for i in range(8):
        a=i*TAU/8;m.beam((x+math.cos(a)*r*.55,height-2.8,z+math.sin(a)*r*.55),(x+math.cos(a)*r*.55,height-.25,z+math.sin(a)*r*.55),.15,IVORY,17,6)
    _dome(m,(x,z),r*.78,1.4,height,BLUE,(24,16,12)[lod],5)


def _madrasa(width,depth,portalheight,lod,domes=False):
    m=Mesh();stone=(182,162,122);wingh=12.6
    # Courtyard is a real void surrounded by two-story teaching galleries.
    for x in [-width/2+4,width/2-4]:m.box((x,wingh/2,depth/2),(8,wingh,depth),stone)
    m.box((0,wingh/2,depth-4),(width,wingh,8),stone)
    for x in [-(width/4+6),width/4+6]:m.box((x,wingh/2,4),(width/2-12,wingh,8),stone)
    for side in (-1,1):
        for x in np.arange(15,width/2-3,5.6):
            for y in [1,7]:
                _arch(m,(side*x,y,-.18),4.0,4.8,.22,BLUE,True,(12,8,6)[lod],(54,81,79))
                if lod==0:_arch(m,(side*x,y+.25,-.25),3.45,4.2,.10,TURQUOISE,True,12,None)
    _iwan(m,0,-1,23,portalheight,8,lod)
    for x in [-width/2,width/2]:_minaret(m,x,1,portalheight+.8,lod)
    if domes:
        for x in [-width*.29,width*.29]:
            _lathe(m,(x,13),[(0,8.2),(12.5,8.2)],stone,11,8,math.pi/8)
            _lathe(m,(x,13),[(12.5,7.1),(20,7.1)],TURQUOISE,17,(40,24,16)[lod])
            _dome(m,(x,13),8.2,12,20,lambda k,j:(37+j%2*7,131+j%2*8,158+j%2*8),(40,24,16)[lod],(14,9,6)[lod],True,IVORY)
    for side in (-1,1):
        wall=Mesh()
        for z in np.arange(13,depth-6,6):
            for y in [1,7]:_arch(wall,(z-depth/2,y,0),3.8,4.6,.2,IVORY,True,(10,8,6)[lod],(70,94,88))
        m.merge(_turn(wall,side*90,side*(width/2-8)-side*.1,0,depth/2))
    return m


def _registan(lod):
    m=Mesh();m.box((-6,-.29,42),(214,.82,172),(181,167,140),8)
    # West/east schools face one another; the northern school closes the square.
    m.merge(_turn(_madrasa(56,82,31.5,lod,False),270,-31,0,0))
    m.merge(_turn(_madrasa(59,64,32,lod,True),90,40,0,0))
    north=_madrasa(72,68,30,lod,False)
    _lathe(north,(-29,33),[(0,10.5),(12.6,10.5)],(182,162,122),11,8,math.pi/8)
    _lathe(north,(-29,33),[(12.6,9),(19.8,9)],TURQUOISE,17,(40,24,16)[lod])
    _dome(north,(-29,33),10.5,13.0,19.8,TURQUOISE,(48,32,20)[lod],(14,10,7)[lod],False,GOLD)
    m.merge(_turn(north,0,0,0,55))
    for x in [-22,22]:m.box((x,.145,13),(1.1,.05,65),(209,195,169),8)
    for z in [-20,45]:m.box((0,.146,z),(46,.05,1.1),(209,195,169),8)
    return m


def _koutoubia(lod):
    m=Mesh();sand=(173,126,85);light=(205,168,119);n=(20,14,10)[lod]
    # A hypostyle mosque with repeated pitched aisles and an open courtyard.
    m.box((-39,-.19,-31),(88,.62,66),(172,146,108),8)
    for x in [-79,-1]:m.box((x,4,-33),(8,8,63),sand)
    m.box((-40,4,-60),(86,8,10),sand)
    m.box((-41,4,-7),(65,8,8),sand)
    for x in np.arange(-72,-4,5.2):
        _gable(m,x,-44,5.3,35,8.0,10.4,sand,(108,118,87))
    m.box((-44,.15,-20),(39,.3,19),(211,187,147),8)
    _lathe(m,(-44,-20),[(.3,3.2),(.8,3.2)],light,17,24)
    _lathe(m,(-44,-20),[(.8,2.8),(.9,2.8)],(60,129,136),1,24)
    m.box((0,26.5,0),(12.8,53,12.8),sand)
    for y in [2.2,19,31,46,51.5]:
        m.box((0,y,0),(13.3,.5,13.3),light)
    for a in [0,90,180,270]:
        side=Mesh()
        for x,y,w,h in [(0,6,4,10),(-3,22,3.4,8),(3,22,3.4,8),(0,34,7,9)]:
            _arch(side,(x,y,-6.48),w,h,.26,light,True,n,(112,77,53))
            if lod==0:_arch(side,(x,y+.6,-6.55),w*.76,h*.83,.14,sand,True,n,None)
        _tile_border(side,0,46,-6.61,11.8,4.5,lod,(74,116,88))
        if lod<2:
            for y in np.arange(33,44,2.2):
                for x in [-4.2,-1.4,1.4,4.2]:
                    for sign in [-1,1]:side.beam((x,y,-6.59),(x+sign*1.25,y+2.0,-6.59),.15,light,11,4)
        m.merge(_turn(side,a))
    # The lantern is smaller, with merlons and four graduated brass spheres.
    for a in [0,90,180,270]:
        wall=Mesh()
        for x in [-5.8,-3.5,-1.2,1.2,3.5,5.8]:
            wall.box((x,54,-6.2),(1.15,2,1.0),light)
        m.merge(_turn(wall,a))
    m.box((0,60,0),(7.6,14,7.6),sand)
    for a in [0,90,180,270]:
        side=Mesh();_arch(side,(0,56,-3.85),3.4,8.4,.22,light,True,n,(93,81,57));m.merge(_turn(side,a))
    m.box((0,67.2,0),(8.7,1.0,8.7),GREEN,12)
    _lathe(m,(0,0),[(67.7,3.8),(69.2,2.1),(70,0.3)],(116,144,105),12,4,angle=math.pi/4)
    m.beam((0,69,0),(0,77,0),.18,GOLD,16,8)
    for y,r in [(70.8,1.4),(73,1.0),(74.7,.67)]:m.ellipsoid((0,y,0),(r,r,r),GOLD,16,(20,14,10)[lod],8)
    return m


def _bellas(lod):
    m=Mesh();white=(228,226,213);ornament=(198,194,177);n=(36,24,16)[lod]
    m.box((0,.2,23),(80,.4,110),(187,182,168),8)
    m.box((0,12.5,23),(68,25,96),white)
    m.box((0,22,51),(54,14,48),white)
    for x in [-32,32]:m.box((x,11,-4),(13,22,51),white)
    m.box((0,11,-24),(43,22,9),white)
    for y in [2,7,21,25]:m.box((0,y,20),(76,.65,101),ornament,17)
    # The grand glazed entry arch and projecting portico define the south facade.
    _arch(m,(0,3,-29),20,18,.75,white,False,n,(64,90,92))
    for x in [-7.8,-3.9,0,3.9,7.8]:m.box((x,9.4,-29.25),(.24,13,.2),ornament,17)
    for x in [-23,-17,17,23]:
        m.beam((x,2.0,-30.5),(x,19.5,-30.5),.82,white,17,(16,12,8)[lod])
        m.box((x,19.8,-30.5),(2.5,1,2.5),ornament,17)
    m.box((0,21,-30),(52,1.2,4),white,17)
    for side in [-1,1]:
        for xx in [28,34]:
            _arch(m,(side*xx,4,-29.2),4.1,12,.35,ornament,False,n)
        for z in np.arange(-17,65,9):
            wall=Mesh();_arch(wall,(0,5,0),4.0,12,.32,ornament,False,n)
            m.merge(_turn(wall,-side*90,side*38.1,0,z))
            for zz in [z-3,z+3]:m.box((side*38.3,11.3,zz),(1.0,19,1),white,17)
    _lathe(m,(0,-3),[(23,18),(27,18)],ornament,17,n)
    # Banded amber/yellow dome tiles are modeled strips, with actual raised ribs.
    warm=lambda k,j: (223,154,47) if k<3 else (231,175,61) if k<6 else (233,195,87)
    _dome(m,(0,-3),18,19,27,warm,(64,40,24)[lod],(14,10,7)[lod],False,(143,119,64))
    for x in [-28,28]:
        _lathe(m,(x,-8),[(0,10.7),(20.5,10.7),(21,11.3)],white,17,n)
        _lathe(m,(x,-8),[(21,9),(24,9)],white,17,n)
        _dome(m,(x,-8),9,10,24,warm,n,(10,7,5)[lod],False,(170,137,79))
    # Original small silhouettes suggest the rooftop sculpture groups.
    for x in [-21,-12,12,21]:
        m.box((x,23,-30.3),(2.5,3,2.4),white,17)
        m.ellipsoid((x,25,-30.3),(.65,.85,.65),ornament,17,12,6)
        if lod<2:
            m.beam((x,24,-30.3),(x-1.3,25.6,-30.3),.25,white,17,6)
            m.beam((x,24,-30.3),(x+1.3,25.6,-30.3),.25,white,17,6)
    m.box((0,46.3,-3),(4.1,1.1,4.1),ornament,17)
    m.ellipsoid((0,48.1,-3),(1.1,1.4,.8),GOLD,16,16,8)
    for side in [-1,1]:
        _face(m,[(0,48.3,-3),(side*5.2,50.2,-3),(side*3.1,47.4,-3.7)],GOLD,16,(0,1,-1))
        _face(m,[(0,48.3,-3),(side*5.2,50.2,-3),(side*3.1,47.4,-3.7)],GOLD,16,(0,-1,1))
    _stairs(m,(0,-38),45,8,2.2,(16,10,6)[lod])
    return m


def _rose(m, x,y,z,r,lod):
    _ring(m,(x,y,z),r,.25,MARBLE,(40,28,18)[lod],17,'xy')
    _ring(m,(x,y,z+.02),r*.72,.13,GREEN,(32,24,16)[lod],17,'xy')
    _face(m,[(x,y,z+.07)]+[(x+r*.92*math.cos(i*TAU/40),y+r*.92*math.sin(i*TAU/40),z+.07) for i in range(41)],(48,73,74),15,(0,0,-1))
    for a in [i*TAU/12 for i in range(12 if lod<2 else 0)]:
        m.beam((x,y,z-.02),(x+r*.95*math.cos(a),y+r*.95*math.sin(a),z-.02),.13,MARBLE,17,5)


def _florence(lod):
    m=Mesh();n=(48,32,20)[lod];white=(229,224,207);rose=(171,124,112);terracotta=(151,76,49)
    # A long basilica joins the octagonal crossing and three radial tribunes.
    _gable(m,0,-67,37,83,28,43,white,terracotta)
    for side in [-1,1]:_gable(m,side*16,-67,10,83,18.5,25,white,terracotta)
    _lathe(m,(0,0),[(0,28),(52,28)],white,11,8,math.pi/8)
    for angle in [0,90,270]:
        apse=Mesh();_lathe(apse,(0,33),[(0,17),(22,17)],white,11,8,math.pi/8)
        _dome(apse,(0,33),17,10,22,terracotta,8,7,True,MARBLE,8)
        for xx in [-12,0,12]:
            _arch(apse,(xx,5,46.2),5.0,10,.4,GREEN,True,14)
        m.merge(_turn(apse,angle))
    for y in [3,10,19,27,36,45,51]:
        _lathe(m,(0,0),[(y,28.15),(y+.7,28.15)],GREEN,17,8,math.pi/8)
    _lathe(m,(0,0),[(51,28),(55,28)],white,17,8,math.pi/8)
    for a in range(0,360,45):
        oculus=Mesh();_rose(oculus,0,46.5,-25.98,3.3,lod)
        m.merge(_turn(oculus,a))
    # Eight terra-cotta sails and marble meridian ribs, not a hemisphere.
    _dome(m,(0,0),27.5,38,55,terracotta,8,(24,16,10)[lod],True,MARBLE,8)
    _lathe(m,(0,0),[(93,4),(96,4.4),(101,3.8),(107,2.0),(109,.6)],MARBLE,17,8,math.pi/8)
    for a in [i*TAU/8+math.pi/8 for i in range(8)]:
        m.beam((3.7*math.cos(a),95,3.7*math.sin(a)),(3.5*math.cos(a),101,3.5*math.sin(a)),.32,MARBLE,17,6)
    m.ellipsoid((0,110.1,0),(1.05,1.05,1.05),GOLD,16,16,8)
    m.beam((0,110.4,0),(0,114.5,0),.16,GOLD,16,6)
    m.beam((-1.25,112.9,0),(1.25,112.9,0),.13,GOLD,16,6)
    # Polychrome marble nave walls with paired pointed windows.
    for side in [-1,1]:
        for z in np.arange(-99,-27,12):
            wall=Mesh()
            _arch(wall,(0,8,0),5.1,12,.43,GREEN,True,(18,12,8)[lod])
            m.merge(_turn(wall,-side*90,side*18.62,0,z))
        for z in np.arange(-105,-28,9):m.box((side*18.7,14,z),(.55,28,.55),GREEN,17)
        for y in [2,7,18,25,28]:m.box((side*18.8,y,-67),(.6,.45,83),GREEN,17)
    # West front: three gables, three portals and the central wheel window.
    front=-109
    m.box((0,15,front),(38,30,1.5),white,17)
    for x,w,peak,eave in [(-13,12,32,24),(0,17,45,30),(13,12,32,24)]:
        _face(m,[(x-w/2,eave,front-.8),(x+w/2,eave,front-.8),(x,peak,front-.8)],white,17,(0,0,-1))
        for a,b in [((x-w/2,eave),(x,peak)),((x,peak),(x+w/2,eave))]:
            m.beam((a[0],a[1],front-1),(b[0],b[1],front-1),.42,GREEN,17,6)
    for x,w,h in [(-12,6.8,14),(0,9,20),(12,6.8,14)]:
        _arch(m,(x,1,front-1.0),w,h,.6,GREEN,True,(22,14,10)[lod])
        _arch(m,(x,1,front-1.2),w*.83,h*.9,.22,rose,True,(22,14,10)[lod],None)
    _rose(m,0,31,front-1.05,4.3,lod)
    for x in [-13,13]:_rose(m,x,23,front-1.05,2.4,lod)
    for y in [2,6,22,27]:m.box((0,y,front-1),(38,.5,.3),GREEN,17)
    for x in [-18.5,-6,6,18.5]:
        m.box((x,14.5,front-1),(.65,29,.65),GREEN,17)
        _lathe(m,(x,front-1),[(29,.7),(34,.25),(35,.03)],MARBLE,17,8)
    # Giotto's campanile remains independent of the nave and the Baptistery.
    tower=Mesh();tower.box((0,41.4,0),(14.2,82.8,14.2),white)
    for y in [2,8,15,22,30,37,45,53,62,71,81,83.5]:tower.box((0,y,0),(14.8,.65,14.8),GREEN,17)
    for angle in [0,90,180,270]:
        face=Mesh()
        for y in [9,17,25]:
            for x in [-3.4,0,3.4]:
                face.box((x,y,-7.21),(2.35,5.9,.22),rose,17)
                _arch(face,(x,y-2.3,-7.4),1.4,4.2,.18,MARBLE,True,10,(104,111,104))
        for y,h,count in [(35,9,2),(48,10,2),(63,15,3)]:
            for x in np.linspace(-3.4,3.4,count):_arch(face,(float(x),y,-7.25),2.65,h,.25,GREEN,True,(18,12,8)[lod])
        for x in [-6.8,6.8]:face.box((x,42,-7.45),(.5,84,.6),GREEN,17)
        mside=_turn(face,angle);tower.merge(mside)
    tower.box((0,84.0,0),(15.2,1.5,15.2),white,17)
    m.merge(_turn(tower,0,27.7,0,-100.8))
    return m


def _hallgrim(lod):
    m=Mesh();stone=(196,194,183);bright=(218,216,204);n=(18,12,8)[lod]
    m.box((0,-.29,25),(63,.82,82),(154,151,138),8)
    _gable(m,0,26,22,44,18,29,stone,(124,124,117))
    _lathe(m,(0,49),[(0,11),(21,11),(25,9.5),(29,1)],stone,11,(36,24,16)[lod])
    # Each facade flute is a hexagonal concrete shaft, stepped along a sweep.
    for side in [-1,1]:
        for x in np.arange(5.1,29.1,(1.2,1.7,2.4)[lod]):
            y=8+47*((30-x)/25)**2.2;z=.014*x*x
            _lathe(m,(side*x,z),[(0,.83),(y,.83)],bright if int(x*2)%2 else stone,11,6,math.pi/6)
        for z in np.arange(9,49,4.0):
            m.box((side*11.2,9,z),(1.2,18,1.5),bright,17)
            pane=Mesh();_arch(pane,(0,5,0),1.6,10,.15,bright,True,n,(78,92,92))
            m.merge(_turn(pane,-side*90,side*11.85,0,z+1.9))
    m.box((0,28.8,0),(8.6,57.6,9.0),stone)
    for x in [-4,-2,0,2,4]:m.box((x,30,-4.8),(.5,60,1.2),bright,17)
    # Clock/bell stage terminates in four slender concrete fins.
    m.box((0,60,0),(8.2,9.5,8.6),stone)
    for a in [0,90,180,270]:
        face=Mesh()
        _arch(face,(0,51,-4.36),3.3,9.0,.18,bright,True,n,(53,68,69))
        _ring(face,(0,64.0,-4.4),1.8,.13,(79,87,79),(28,20,14)[lod],17,'xy')
        face.beam((0,64,-4.5),(.1,65.35,-4.5),.10,(50,61,53),17,5)
        face.beam((0,64,-4.5),(1.08,63.4,-4.5),.10,(50,61,53),17,5)
        _face(face,[(-4.1,65,-4.3),(4.1,65,-4.3),(1.2,71.7,-4.3),(-1.2,71.7,-4.3)],bright,17,(0,0,-1))
        m.merge(_turn(face,a))
    for x in [-2.9,2.9]:m.box((x,67,0),(.65,12,8.4),bright,17)
    m.beam((0,69.8,0),(0,74.5,0),.18,bright,17,6)
    m.beam((-1.1,73,0),(1.1,73,0),.15,bright,17,6)
    _arch(m,(0,.6,-5.6),5.7,13.5,.5,bright,True,n,(58,70,70))
    _stairs(m,(0,-12),10,6,1.2,(10,7,4)[lod])
    return m


def _pagoda_roof(m,x,z,y,r,lod):
    n=12;parts=(4,2,1)[lod]
    # Twelve curved eave corners and lower center spans, with a closed underside.
    rings=[]
    for rr,yy in [(r*.48,y+1.8),(r*.67,y+.8),(r*.87,y+.22),(r,y+.6)]:
        ring=[]
        for i in range(n*parts):
            a=i*TAU/(n*parts);f=(i%parts)/parts
            radial=rr*(.965+.035*abs(2*f-1));h=yy+.28*(rr/r)**3*abs(2*f-1)
            ring.append((x+radial*math.cos(a),h,z+radial*math.sin(a)))
        rings.append(ring)
    for a,b in zip(rings,rings[1:]):
        for i in range(len(a)):
            j=(i+1)%len(a);_face(m,[a[i],a[j],b[j],b[i]],(197,92+(i%3)*3,39),12,(0,1,0))
    for i in range(len(rings[-1])):
        j=(i+1)%len(rings[-1]);a,b=rings[-1][i],rings[-1][j]
        _tube(m,a,b,.1,(226,175,73),17,5 if lod==0 else 4)
        _face(m,[(x,y,z),a,b],(131,56,32),12,(0,-1,0))
    if lod<2:
        for i in range(n):
            path=[ring[i*parts] for ring in rings]
            for a,b in zip(path,path[1:]):m.beam(a,b,.075,(234,161,58),17,5)


def _beast(m,x,z,dragon,lod):
    """Static sculptural entrance, with a genuinely open mouth passage."""
    n=(24,14,8)[lod];rings=(10,7,4)[lod];gold=(212,145,38);black=(41,49,38);green=(58,133,102);body=green if dragon else gold
    # An arched mouth shell leaves the dark walking tunnel open between jaws.
    for side in [-1,1]:m.ellipsoid((x+side*2.55,3.2,z),(1.35,2.65,3.1),body,11,n,rings)
    m.ellipsoid((x,5.5,z-.65),(3.25,1.5,2.5),body,11,n,rings)
    m.ellipsoid((x,1.1,z-1.4),(3.4,.65,3.1),(189,99,57),11,n,max(4,rings-2))
    m.box((x,1.7,z+1),(3.6,.2,7),(142,71,48),8)
    for side in [-1,1]:
        m.ellipsoid((x+side*1.82,5.5,z-2.6),(.57,.36,.25),IVORY,17,n,max(4,rings-2))
        m.ellipsoid((x+side*1.82,5.5,z-2.94),(.18,.23,.09),black,17,12,6)
        for zz in [-2,-.7,1.0]:
            _lathe(m,(x+side*2.0,z+zz),[(4.3,.32),(3.35,.03)],IVORY,17,8)
        if dragon:
            for a,b,r in [((x+side*1.9,6,z),(x+side*2.5,8.4,z+1),.3),((x+side*2.5,8.4,z+1),(x+side*3.3,9.3,z+2),.16),((x+side*2.4,7.9,z+.8),(x+side*1.8,8.9,z+1.4),.13)]:m.beam(a,b,r,(163,142,75),11,6)
            m.beam((x+side*2.3,3.9,z-3),(x+side*4.6,4.1,z-3.7),.16,IVORY,17,6)
            m.ellipsoid((x+side*2.9,5.2,z+.8),(.8,1.3,.4),RED,11,16,8)
        else:
            m.ellipsoid((x+side*2.4,6.5,z+.3),(.9,1,.5),gold,11,n,max(4,rings-2))
            m.ellipsoid((x+side*2.4,6.6,z-.12),(.45,.55,.15),black,11,12,6)
            for yy in [2.5,3.6,4.7]:m.beam((x+side*2.7,yy,z-2.3),(x+side*3.4,yy+.35,z-1),.22,black,11,6)
    if not dragon:
        for yy,w in [(5.8,1.7),(6.15,1.35),(6.45,.8)]:
            m.box((x,yy,z-2.15),(w,.15,.12),black,11)
    m.ellipsoid((x,4.9,z-3),(1.25,.65,.65),body if dragon else black,11,n,max(4,rings-2))
    for zz in np.arange(2.2,9,1.25 if lod<2 else 2.5):
        m.ellipsoid((x,3.5, z+zz),(2.4,2.3,1.4),body,11,(16,12,8)[lod],(8,6,4)[lod])
        if dragon:_lathe(m,(x,z+zz),[(5.4,.6),(6.5,.03)],GOLD,11,5)
        elif lod<2:
            for side in [-1,1]:m.beam((x+side*2.2,3,z+zz),(x+side*1.2,5.3,z+zz),.25,black,11,6)


# Surveyed OSM bridge centerlines in the model frame. The two mouths join
# before the zigzag reaches the measured 9.5m shore; the pagoda datum is 13.75m.
_PAGODA_PATHS = [
    [(-10.8,-15.5),(-8.13,-15.56),(-8.24,-25.22),(3.8,-32.96),
     (-7.87,-37.92),(4.24,-45.05),(-8.12,-50.55),(3.13,-57.69),
     (-8.36,-63.17),(-3.0,-66.83),(-1.56,-70.12),(-1.31,-74.99),(-1.31,-79)],
    [(10.8,-15.5),(6.16,-16.39),(5.27,-32.15),(3.8,-32.96)],
]


def _pagoda_road_exclusions():
    polygons=[]
    for path in _PAGODA_PATHS:
        for a,b in zip(path,path[1:]):
            a,b=np.array(a,dtype=float),np.array(b,dtype=float)
            forward=(b-a)/np.linalg.norm(b-a);side=np.array([-forward[1],forward[0]])*2.5
            lo,hi=a-forward*.5,b+forward*.5
            polygon=np.array([lo+side,lo-side,hi-side,hi+side])
            # Preserve the cross-shore route at z=-80; overlap its inner edge.
            polygon[:,1]=np.maximum(polygon[:,1],-78.4)
            polygons.append(polygon.tolist())
    # Native bridge offsets use miters; segment caps alone leave small outer
    # corner fragments at acute turns. Join squares stay inside the lake route.
    for x,z in _PAGODA_PATHS[0][2:-2]:
        polygons.append([[x-3.5,z-3.5],[x+3.5,z-3.5],
                         [x+3.5,z+3.5],[x-3.5,z+3.5]])
    return polygons


def _pagoda_walkway(m,lod):
    def height(z):return .85-4.95*min(1,max(0,(-z-25)/50))
    for path in _PAGODA_PATHS:
        for a,b in zip(path,path[1:]):
            p,q=np.array((a[0],height(a[1]),a[1])),np.array((b[0],height(b[1]),b[1]))
            d=q-p;length=np.linalg.norm(d);forward=d/length
            right=np.array([d[2],0,-d[0]]);right/=np.linalg.norm(right)
            up=np.cross(forward,right);center=(p+q)*.5
            bridge=Mesh();bridge.box((0,0,0),(2.8,.5,length+.12),(202,182,139),8)
            for side in [-1,1]:
                bridge.beam((side*1.25,1.1,-length/2),(side*1.25,1.1,length/2),.12,(188,58,39),11,6)
                for z in np.linspace(-length/2,length/2,max(2,int(length/(1.8 if lod<2 else 3)))):
                    bridge.beam((side*1.25,.2,z),(side*1.25,1.2,z),.10,(188,58,39),11,5)
            transform=np.eye(4);transform[:3,:3]=np.column_stack((right,up,forward));transform[:3,3]=center
            m.merge(bridge.transform(transform))
            pier_top=float(center[1]-.25)
            m.box((center[0],(pier_top-7)*.5,center[2]),(.7,pier_top+7,.7),(173,158,129),17)


def _dragon_tiger(lod):
    m=Mesh();n=(36,24,12)[lod]
    for x,dragon in [(-10.8,True),(10.8,False)]:
        _lathe(m,(x,0),[(-7,8.4),(1.0,8.4),(1.3,8.8)],(198,173,128),17,12,math.pi/12)
        _beast(m,x,-11,dragon,lod)
        for floor in range(7):
            y=1.3+floor*3.05;r=6.0-floor*.14
            _lathe(m,(x,0),[(y,r*.76),(y+2.45,r*.76)],(221,185,83),11,12,math.pi/12)
            _lathe(m,(x,0),[(y+.1,r),(y+.35,r)],(210,169,87),17,12,math.pi/12)
            for j in range(12):
                a=j*TAU/12+math.pi/12
                xx,zz=x+(r-.25)*math.cos(a),(r-.25)*math.sin(a)
                m.beam((xx,y+.2,zz),(xx,y+2.8,zz),.19,RED,11,(8,6,4)[lod])
                if lod<2:
                    b=a+TAU/12
                    m.beam((xx,y+1.1,zz),(x+(r-.25)*math.cos(b),y+1.1,(r-.25)*math.sin(b)),.08,RED,11,5)
                if lod<2 or j%2==0:
                    pane=Mesh();_arch(pane,(0,y+.6,0),1.15,1.4,.1,RED,False,8,(61,73,53))
                    m.merge(_turn(pane,270-math.degrees(a),x+r*.765*math.cos(a),0,r*.765*math.sin(a)))
            _pagoda_roof(m,x,0,y+2.05,r+1.5,lod)
        _lathe(m,(x,0),[(23.7,1.25),(24.5,.9),(25.6,.1)],GOLD,16,12)
    _pagoda_walkway(m,lod)
    return m


BUILDERS = {'temple-of-heaven':_temple, 'registan':_registan, 'koutoubia':_koutoubia,
            'palacio-bellas-artes':_bellas, 'florence-cathedral':_florence,
            'hallgrimskirkja':_hallgrim, 'dragon-tiger-pagodas':_dragon_tiger}


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('expected LOD 0, 1 or 2')
    return BUILDERS[name](lod)

# Local x points right along the front facade; local +z points into the site.
ENTRIES = [{'name': 'temple-of-heaven',
  'city': 'Beijing',
  'lat': 39.8822513364,
  'lon': 116.4066224804,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://english.beijing.gov.cn/specials/parktours/guidevisitors/templeofheaven/mustsee/202301/t20230112_2897330.html',
                 'https://english.visitbeijing.com.cn/article/47OMoxqKJUk'],
  'description': 'Three blue glazed circular roofs above a red timber hall and three white-marble '
                 'balustraded terraces.',
  'capture': {'distance': 175,
              'azimuth': 210,
              'elevation': 24,
              'target_height': 15,
              'time_utc': '2026-03-20T07:00:00Z'},
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [46, 46],
                       'feather_m': 6,
                       'sample_clearance_m': 18,
                       'fixed_base_geo': None}],
  'audit': {'source_anchor_latlon': [39.8822, 116.40655],
            'source_offset_east_north_m': [6.2, 5.7],
            'ground_center_m': 56.90886306762695,
            'grid_min_m': 48.868221282958984,
            'grid_max_m': 56.90886306762695,
            'perimeter_m': [54.525062561035156,
                            55.74445343017578,
                            55.94640350341797,
                            54.86394500732422,
                            50.8133544921875,
                            51.100975036621094,
                            48.88277816772461,
                            52.21222686767578],
            'perimeter_median_m': 53.36864471435547,
            'source_file': 'output/city-catalog-expansion/heritage/temple-of-heaven-source.json',
            'perimeter_samples': [{'x': -70,
                                   'z': -70,
                                   'lat': 39.88162088962488,
                                   'lon': 116.40580415316028,
                                   'ground_m': 54.525062561035156},
                                  {'x': 0,
                                   'z': -70,
                                   'lat': 39.88162088962488,
                                   'lon': 116.4066224804,
                                   'ground_m': 55.74445343017578},
                                  {'x': 70,
                                   'z': -70,
                                   'lat': 39.88162088962488,
                                   'lon': 116.40744080763973,
                                   'ground_m': 55.94640350341797},
                                  {'x': 70,
                                   'z': 0,
                                   'lat': 39.8822513364,
                                   'lon': 116.40744080763973,
                                   'ground_m': 54.86394500732422},
                                  {'x': 70,
                                   'z': 70,
                                   'lat': 39.88288178317513,
                                   'lon': 116.40744080763973,
                                   'ground_m': 50.8133544921875},
                                  {'x': 0,
                                   'z': 70,
                                   'lat': 39.88288178317513,
                                   'lon': 116.4066224804,
                                   'ground_m': 51.100975036621094},
                                  {'x': -70,
                                   'z': 70,
                                   'lat': 39.88288178317513,
                                   'lon': 116.40580415316028,
                                   'ground_m': 48.88277816772461},
                                  {'x': -70,
                                   'z': 0,
                                   'lat': 39.8822513364,
                                   'lon': 116.40580415316028,
                                   'ground_m': 52.21222686767578}]}},
 {'name': 'registan',
  'city': 'Samarkand',
  'lat': 39.65475,
  'lon': 66.9757,
  'heading': 348.5,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://uzbekistan.travel/en/o/registan/', 'https://whc.unesco.org/en/list/603'],
  'description': 'Three facing tiled madrasas with recessed pointed iwans, two-story courtyard '
                 'galleries, banded minarets and turquoise domes.',
  'capture': {'distance': 310,
              'azimuth': 174,
              'elevation': 30,
              'target_height': 17,
              'time_utc': '2026-03-20T10:00:00Z'},
  'terrain_flatten': [{'center_xz': [-7.0, 42.0],
                       'half_extent': [106.5, 86.0],
                       'feather_m': 3.0,
                       'sample_clearance_m': 36.0,
                       'fixed_base_geo': None}],
  'audit': {'source_anchor_latlon': [39.65475, 66.9757],
            'source_offset_east_north_m': [0, 0],
            'ground_center_m': 720.9641723632812,
            'grid_min_m': 712.2172241210938,
            'grid_max_m': 725.0906982421875,
            'perimeter_m': [718.4688720703125,
                            716.4002075195312,
                            724.4366455078125,
                            720.9097900390625,
                            716.6775512695312,
                            715.3189086914062,
                            712.9865112304688,
                            715.7576293945312],
            'perimeter_median_m': 716.5388793945312,
            'source_file': 'output/city-catalog-expansion/heritage/registan-source.json',
            'suppression_note': 'East feather edge102.5m stops before mappedChorsu market103.07m; long '
                                'westschool depth82m andeastschool64m follow source footprints. '
                                'Reference samples clear Chorsu dome extent133.34m.',
            'perimeter_samples': [{'x': -152.5,
                                   'z': -83.0,
                                   'lat': 39.653743611025185,
                                   'lon': 66.97415155772052,
                                   'ground_m': 718.4688720703125},
                                  {'x': -7.0,
                                   'z': -83.0,
                                   'lat': 39.654004878750975,
                                   'lon': 66.97581288509231,
                                   'ground_m': 716.4002075195312},
                                  {'x': 138.5,
                                   'z': -83.0,
                                   'lat': 39.65426614647677,
                                   'lon': 66.97747421246409,
                                   'ground_m': 724.4366455078125},
                                  {'x': 138.5,
                                   'z': 42.0,
                                   'lat': 39.655369386940194,
                                   'lon': 66.97718383370646,
                                   'ground_m': 720.9097900390625},
                                  {'x': 138.5,
                                   'z': 167.0,
                                   'lat': 39.656472627403616,
                                   'lon': 66.97689345494882,
                                   'ground_m': 716.6775512695312},
                                  {'x': -7.0,
                                   'z': 167.0,
                                   'lat': 39.656211359677826,
                                   'lon': 66.97523212757704,
                                   'ground_m': 715.3189086914062},
                                  {'x': -152.5,
                                   'z': 167.0,
                                   'lat': 39.65595009195203,
                                   'lon': 66.97357080020525,
                                   'ground_m': 712.9865112304688},
                                  {'x': -152.5,
                                   'z': 42.0,
                                   'lat': 39.65484685148861,
                                   'lon': 66.97386117896288,
                                   'ground_m': 715.7576293945312}]}},
 {'name': 'koutoubia',
  'city': 'Marrakech',
  'lat': 31.6240866561,
  'lon': -7.9932806311,
  'heading': 338.45,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://visitmarrakech.com/en/listing/koutoubia-mosque/',
                 'https://publ.icomos.org/publicomos/jlbSai?base=technica&file=1183.pdf&html=Bur&path=marrakech_guide.pdf'],
  'description': 'The restrained sandstone minaret with carved blind arches, green tile registers and '
                 'graduated brass finials beside a roof-aisled courtyard mosque.',
  'capture': {'distance': 225,
              'azimuth': 100,
              'elevation': 25,
              'target_height': 28,
              'time_utc': '2026-03-20T15:00:00Z'},
  'terrain_flatten': [{'center_xz': [-40, -31],
                       'half_extent': [50, 37],
                       'feather_m': 4,
                       'sample_clearance_m': 18,
                       'fixed_base_geo': None}],
  'audit': {'source_anchor_latlon': [31.62418, -7.9932],
            'source_offset_east_north_m': [-7.65, -10.35],
            'ground_center_m': 465.4794616699219,
            'grid_min_m': 464.8203125,
            'grid_max_m': 469.0,
            'perimeter_m': [468.0,
                            467.0274963378906,
                            468.4463806152344,
                            468.8146667480469,
                            464.99993896484375,
                            465.7996826171875,
                            464.8203125,
                            467.31072998046875],
            'perimeter_median_m': 467.1691131591797,
            'source_file': 'output/city-catalog-expansion/heritage/koutoubia-source.json',
            'perimeter_samples': [{'x': -112,
                                   'z': -90,
                                   'lat': 31.622960686229163,
                                   'lon': -7.994030159550762,
                                   'ground_m': 468.0},
                                  {'x': -40,
                                   'z': -90,
                                   'lat': 31.62319920064023,
                                   'lon': -7.993324327029673,
                                   'ground_m': 467.0274963378906},
                                  {'x': 32,
                                   'z': -90,
                                   'lat': 31.623437715051296,
                                   'lon': -7.992618494508584,
                                   'ground_m': 468.4463806152344},
                                  {'x': 32,
                                   'z': -31,
                                   'lat': 31.62393262504867,
                                   'lon': -7.992846911860516,
                                   'ground_m': 468.8146667480469},
                                  {'x': 32,
                                   'z': 28,
                                   'lat': 31.624427535046046,
                                   'lon': -7.993075329212449,
                                   'ground_m': 464.99993896484375},
                                  {'x': -40,
                                   'z': 28,
                                   'lat': 31.624189020634983,
                                   'lon': -7.993781161733538,
                                   'ground_m': 465.7996826171875},
                                  {'x': -112,
                                   'z': 28,
                                   'lat': 31.623950506223917,
                                   'lon': -7.994486994254627,
                                   'ground_m': 464.8203125},
                                  {'x': -112,
                                   'z': -31,
                                   'lat': 31.623455596226542,
                                   'lon': -7.994258576902695,
                                   'ground_m': 467.31072998046875}]}},
 {'name': 'palacio-bellas-artes',
  'city': 'Mexico City',
  'lat': 19.4352605254,
  'lon': -99.1412995094,
  'heading': 9.2,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://inba.gob.mx/recinto/1/',
                 'https://palacio.inba.gob.mx/recinto/salaprincipal',
                 'https://www.dgcs.unam.mx/boletin/bdboletin/2022_107.html'],
  'description': 'A white marble cultural palace with a great amber dome, two side cupolas, monumental '
                 'arched glazing and sculpted roof silhouettes.',
  'capture': {'distance': 220,
              'azimuth': 214,
              'elevation': 26,
              'target_height': 22,
              'time_utc': '2026-03-20T21:00:00Z'},
  'terrain_flatten': [{'center_xz': [0, 23],
                       'half_extent': [43, 59],
                       'feather_m': 3,
                       'sample_clearance_m': 20,
                       'fixed_base_geo': None}],
  'audit': {'source_anchor_latlon': [19.4352, -99.1412],
            'source_offset_east_north_m': [-10.45, 6.7],
            'ground_center_m': 2240.70458984375,
            'grid_min_m': 2238.13330078125,
            'grid_max_m': 2247.629150390625,
            'perimeter_m': [2238.288818359375,
                            2238.192626953125,
                            2241.71240234375,
                            2244.248779296875,
                            2245.138671875,
                            2238.574951171875,
                            2238.749267578125,
                            2240.152587890625],
            'perimeter_median_m': 2239.450927734375,
            'source_file': 'output/city-catalog-expansion/heritage/palacio-bellas-artes-source.json',
            'perimeter_samples': [{'x': -66,
                                   'z': -59,
                                   'lat': 19.434829721114205,
                                   'lon': -99.1420097301354,
                                   'ground_m': 2238.288818359375},
                                  {'x': 0,
                                   'z': -59,
                                   'lat': 19.434734396691855,
                                   'lon': -99.14138933438421,
                                   'ground_m': 2238.192626953125},
                                  {'x': 66,
                                   'z': -59,
                                   'lat': 19.43463907226951,
                                   'lon': -99.14076893863304,
                                   'ground_m': 2241.71240234375},
                                  {'x': 66,
                                   'z': 23,
                                   'lat': 19.43537030199947,
                                   'lon': -99.14064409712955,
                                   'ground_m': 2244.248779296875},
                                  {'x': 66,
                                   'z': 105,
                                   'lat': 19.43610153172943,
                                   'lon': -99.14051925562606,
                                   'ground_m': 2245.138671875},
                                  {'x': 0,
                                   'z': 105,
                                   'lat': 19.43619685615178,
                                   'lon': -99.14113965137723,
                                   'ground_m': 2238.574951171875},
                                  {'x': -66,
                                   'z': 105,
                                   'lat': 19.43629218057413,
                                   'lon': -99.14176004712841,
                                   'ground_m': 2238.749267578125},
                                  {'x': -66,
                                   'z': 23,
                                   'lat': 19.435560950844167,
                                   'lon': -99.1418848886319,
                                   'ground_m': 2240.152587890625}]}},
 {'name': 'florence-cathedral',
  'city': 'Florence',
  'lat': 43.7731189006,
  'lon': 11.2569506071,
  'heading': 91.36,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://duomo.firenze.it/en/discover/cathedral',
                 'https://duomo.firenze.it/en/40/dome',
                 'https://duomo.firenze.it/en/discover/giottos-bell-tower'],
  'description': 'A polychrome marble basilica, three radial tribunes, eight terracotta dome sails and '
                 'lantern, ornate west front and independent Giotto campanile.',
  'capture': {'distance': 310,
              'azimuth': 242,
              'elevation': 28,
              'target_height': 43,
              'time_utc': '2026-03-20T14:00:00Z'},
  'terrain_flatten': [{'center_xz': [0.0, 0.0],
                       'half_extent': [48.0, 48.0],
                       'feather_m': 2.0,
                       'sample_clearance_m': 0.0,
                       'fixed_base_geo': 67.7647},
                      {'center_xz': [0.0, -65.0],
                       'half_extent': [22.0, 43.0],
                       'feather_m': 2.0,
                       'sample_clearance_m': 0.0,
                       'fixed_base_geo': 67.7647},
                      {'center_xz': [27.7, -100.8],
                       'half_extent': [7.2, 7.4],
                       'feather_m': 2.0,
                       'sample_clearance_m': 0.0,
                       'fixed_base_geo': 67.7647}],
  'audit': {'source_anchor_latlon': [43.7731, 11.25685],
            'source_offset_east_north_m': [8.1, 2.1],
            'ground_center_m': 74.45357513427734,
            'grid_min_m': 60.285221099853516,
            'grid_max_m': 74.58064270019531,
            'perimeter_m': [62.53937911987305,
                            60.98955154418945,
                            63.67778778076172,
                            69.82095336914062,
                            69.22723388671875,
                            69.88423919677734,
                            64.03123474121094,
                            62.30561828613281],
            'perimeter_median_m': 63.85451126098633,
            'source_file': 'output/city-catalog-expansion/heritage/florence-cathedral-source.json',
            'fixed_grade_note': 'Connected cathedral requires one datum across three narrow pads; eleven '
                                'measured piazza points outside every sourcebuildingpolygon yield '
                                'median67.7647m, versus74.45m at the dome. Ordinary enclosing rectangle '
                                'would suppress adjacent blocks.',
            'clean_piazza_samples': [{'x': -53,
                                      'z': 0,
                                      'lat': 43.77359578038534,
                                      'lon': 11.256966231202181,
                                      'ground_m': 66.41595458984375,
                                      'source_building_seeds': []},
                                     {'x': 53,
                                      'z': 0,
                                      'lat': 43.77264202081465,
                                      'lon': 11.25693498299782,
                                      'ground_m': 69.82858276367188,
                                      'source_building_seeds': []},
                                     {'x': -28,
                                      'z': -70,
                                      'lat': 43.77338579011104,
                                      'lon': 11.256089661996779,
                                      'ground_m': 63.63046646118164,
                                      'source_building_seeds': []},
                                     {'x': 28,
                                      'z': -70,
                                      'lat': 43.772881917130306,
                                      'lon': 11.256073153511455,
                                      'ground_m': 70.18944549560547,
                                      'source_building_seeds': []},
                                     {'x': -26,
                                      'z': -106,
                                      'lat': 43.773375484772366,
                                      'lon': 11.255642055601562,
                                      'ground_m': 60.96056365966797,
                                      'source_building_seeds': []},
                                     {'x': 0,
                                      'z': -119,
                                      'lat': 43.77314432073515,
                                      'lon': 11.255472968211999,
                                      'ground_m': 61.93027114868164,
                                      'source_building_seeds': []},
                                     {'x': 0,
                                      'z': 55,
                                      'lat': 43.77310715179804,
                                      'lon': 11.257633549443195,
                                      'ground_m': 68.65576171875,
                                      'source_building_seeds': []},
                                     {'x': -30,
                                      'z': 42,
                                      'lat': 43.77337986074156,
                                      'lon': 11.257480970538953,
                                      'ground_m': 68.12024688720703,
                                      'source_building_seeds': []},
                                     {'x': 30,
                                      'z': 42,
                                      'lat': 43.77283999683362,
                                      'lon': 11.257463282876108,
                                      'ground_m': 67.76469421386719,
                                      'source_building_seeds': []},
                                     {'x': -27,
                                      'z': -36,
                                      'lat': 43.77336952948349,
                                      'lon': 11.256511549741827,
                                      'ground_m': 66.91290283203125,
                                      'source_building_seeds': []},
                                     {'x': 27,
                                      'z': -42,
                                      'lat': 43.772884933653835,
                                      'lon': 11.25642112804419,
                                      'ground_m': 74.18716430664062,
                                      'source_building_seeds': []}],
            'suppression_note': 'Crossing/tribunes, narrow nave, and independent campanile patches '
                                'preserve neighboring blocks and Baptistery.'}},
 {'name': 'hallgrimskirkja',
  'city': 'Reykjavik',
  'lat': 64.1419406683,
  'lon': -21.9269111376,
  'heading': 148.7,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.hallgrimskirkja.is/en-gb/husi%C3%B0-og-sagan',
                 'https://www.skyscrapercenter.com/building/hallgrimskirkja/30284'],
  'description': 'A stepped sweep of concrete basalt flutes, clock-and-bell tower, pointed entrance and '
                 'long nave terminating in a rounded sanctuary.',
  'capture': {'distance': 185,
              'azimuth': 312,
              'elevation': 18,
              'target_height': 33,
              'time_utc': '2026-03-20T16:00:00Z'},
  'terrain_flatten': [{'center_xz': [0, 24],
                       'half_extent': [34, 41],
                       'feather_m': 6,
                       'sample_clearance_m': 18,
                       'fixed_base_geo': None}],
  'audit': {'source_anchor_latlon': [64.14172, -21.92664],
            'source_offset_east_north_m': [-13.2, 24.6],
            'ground_center_m': 105.5347900390625,
            'grid_min_m': 101.06343841552734,
            'grid_max_m': 111.83184814453125,
            'perimeter_m': [102.64380645751953,
                            103.6494140625,
                            102.76441192626953,
                            102.76760864257812,
                            101.56416320800781,
                            101.40845489501953,
                            106.19183349609375,
                            102.74072265625],
            'perimeter_median_m': 102.75256729125977,
            'source_file': 'output/city-catalog-expansion/heritage/hallgrimskirkja-source.json',
            'perimeter_samples': [{'x': -58,
                                   'z': -41,
                                   'lat': 64.14252521432832,
                                   'lon': -21.926330686270532,
                                   'ground_m': 102.64380645751953},
                                  {'x': 0,
                                   'z': -41,
                                   'lat': 64.14225492159719,
                                   'lon': -21.92734866446574,
                                   'ground_m': 103.6494140625},
                                  {'x': 58,
                                   'z': -41,
                                   'lat': 64.14198462886607,
                                   'lon': -21.928366642660947,
                                   'ground_m': 102.76441192626953},
                                  {'x': 58,
                                   'z': 24,
                                   'lat': 64.14148642241929,
                                   'lon': -21.927673002507948,
                                   'ground_m': 102.76760864257812},
                                  {'x': 58,
                                   'z': 89,
                                   'lat': 64.14098821597251,
                                   'lon': -21.926979362354945,
                                   'ground_m': 101.56416320800781},
                                  {'x': 0,
                                   'z': 89,
                                   'lat': 64.14125850870364,
                                   'lon': -21.925961384159738,
                                   'ground_m': 101.40845489501953},
                                  {'x': -58,
                                   'z': 89,
                                   'lat': 64.14152880143477,
                                   'lon': -21.92494340596453,
                                   'ground_m': 106.19183349609375},
                                  {'x': -58,
                                   'z': 24,
                                   'lat': 64.14202700788154,
                                   'lon': -21.925637046117533,
                                   'ground_m': 102.74072265625}]}},
 {'name': 'dragon-tiger-pagodas',
  'city': 'Kaohsiung',
  'lat': 22.680474913,
  'lon': 120.2924792404,
  'heading': 109.3,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://eng.taiwan.net.tw/m1.aspx?id=r157&sno=0000208',
                 'https://www.openstreetmap.org/#map=19/22.6803/120.2922'],
  'attribution': 'Bridge centerlines: © OpenStreetMap contributors (ODbL), via Protomaps. Landmark geometry originally authored.',
  'road_exclusion_polygons': _pagoda_road_exclusions(),
  'activation_bounds_xz': [[-21,-78.4],[21,10]],
  'description': 'Paired seven-storey pagodas with twelve-corner orange eaves, open balconies, '
                 'sculptural dragon/tiger mouth entrances and a static zigzag bridge.',
  'capture': {'distance': 125,
              'azimuth': 285,
              'elevation': 22,
              'target_height': 10,
              'time_utc': '2026-03-20T07:00:00Z'},
  'terrain_flatten': [],
  'audit': {'source_anchor_latlon': [22.6808, 120.29285],
            'source_offset_east_north_m': [-38.1, -36],
            'ground_center_m': 13.750072479248047,
            'shore_ground_m': 9.477,
            'shore_inferred_water_m': 9.665,
            'shore_deck_top_m': 9.90,
            'water_audit_file': 'output/city-catalog-expansion/heritage/pagodas-water-clearance.json',
            'shore_survey_file': 'output/city-catalog-expansion/heritage/pagodas-shore-survey.json',
            'source_bridge_roads': [218,220,144],
            'grid_min_m': 9.158557891845703,
            'grid_max_m': 15.280964851379395,
            'perimeter_m': [14.224042892456055,
                            11.15357780456543,
                            9.158557891845703,
                            15.280964851379395,
                            13.685744285583496,
                            12.84914493560791,
                            12.280073165893555,
                            13.403145790100098],
            'perimeter_median_m': 13.126145362854004,
            'source_file': 'output/city-catalog-expansion/heritage/dragon-tiger-pagodas-source.json',
            'grounding_note': 'Between-tower anchor13.751m; twelve-sided foundations extend7m below the '
                              'model datum. Surveyed OSM zigzag deck descends to shore9.5m; piers reach localy−7. No DEM flattening of the '
                              'surrounding lake.'}}]
