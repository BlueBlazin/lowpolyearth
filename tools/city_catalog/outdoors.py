"""Original, static outdoor landmarks in metres; references and placement in ENTRIES."""
import math
import numpy as np
from common import Mesh

# IDs follow vearth_gen::material. Metal RGB is specular reflectance; matte
# pigments use the physical atlas families for relief and roughness.
METAL, MASONRY, PLASTER, TIMBER, PAINT = 95, 82, 83, 84, 87
ASPHALT, PAVING, OXIDE = 7, 8, 11
STEEL = (178, 190, 200)
DARK = (28, 38, 45)
WHITE = (190, 183, 166)
STONE = (160, 132, 88)
RUST = (125, 57, 28)
LIGHT = (245, 202, 132)


def _quad(m, p, color, material=METAL, normal=None):
    p = np.asarray(p, dtype=float)
    if normal is not None and np.dot(np.cross(p[1]-p[0], p[2]-p[0]), normal) < 0:
        p = p[::-1]
    m.quad(*p, color, material)


def _tube(m, points, radius, color, material=METAL, sides=6):
    # Connected rings omit hidden caps at every segment of a curved member.
    p = np.asarray(points, dtype=float)
    rings = []
    for i, c in enumerate(p):
        axis = p[min(i+1,len(p)-1)]-p[max(i-1,0)]
        axis /= np.linalg.norm(axis)
        side = np.cross(axis, [0,1,0] if abs(axis[1]) < .9 else [1,0,0])
        side /= np.linalg.norm(side)
        up = np.cross(axis, side)
        rings.append([c+radius*(math.cos(j*math.tau/sides)*side+math.sin(j*math.tau/sides)*up) for j in range(sides)])
    for a,b in zip(rings,rings[1:]):
        for j in range(sides):
            k=(j+1)%sides
            m.quad(a[j],a[k],b[k],b[j],color,material)
    for j in range(sides):
        k=(j+1)%sides
        m.triangle(p[0],rings[0][k],rings[0][j],color,material)
        m.triangle(p[-1],rings[-1][j],rings[-1][k],color,material)


def _loft(m, profile, color, material=METAL, n=32, panels=False):
    """Closed horizontal ellipse rings: (height, center_x, center_z, rx, rz)."""
    rings = [[np.array([x+rx*math.cos(j*math.tau/n),y,z+rz*math.sin(j*math.tau/n)])
              for j in range(n)] for y,x,z,rx,rz in profile]
    for i,(a,b) in enumerate(zip(rings,rings[1:])):
        for j in range(n):
            k=(j+1)%n
            shade=tuple(int(np.clip(v+((i*17+j*7)%11-5)*1.6,0,255)) for v in color) if panels else color
            m.quad(a[j],b[j],b[k],a[k],shade,material)
    for j in range(n):
        k=(j+1)%n
        m.triangle([profile[0][1],profile[0][0],profile[0][2]],rings[0][j],rings[0][k],color,material)
        m.triangle([profile[-1][1],profile[-1][0],profile[-1][2]],rings[-1][k],rings[-1][j],color,material)


def _disc(m, center, radius, depth, color, material=MASONRY, n=64):
    x,y,z=center
    _loft(m,[(y-depth,x,z,radius,radius),(y,x,z,radius,radius)],color,material,n)


def _transform(m, x=0,y=0,z=0, heading=0):
    a=math.radians(heading);c,s=math.cos(a),math.sin(a)
    return m.transform([[c,0,s,x],[0,1,0,y],[-s,0,c,z],[0,0,0,1]])


def _angel(lod):
    m=Mesh();n=(32,20,12)[lod]
    # The ribbed human figure carries two shallow, forward-swept steel wings.
    for sx in [-1,1]:
        _loft(m,[(-1,sx*.65,0,.58,.68),(1,sx*.65,.1,.55,.6),
                 (5,sx*.65,0,.67,.67),(9,sx*.65,0,.77,.72)],RUST,OXIDE,n=n)
    _loft(m,[(8,0,0,1.3,.72),(10.5,0,0,1.15,.65),(13,0,0,1.6,.88),
             (15.1,0,0,1.45,.85),(16,0,0,.7,.68)],RUST,OXIDE,n=n)
    _loft(m,[(15.5,0,0,.65,.6),(17,0,0,.65,.65),(18,0,.06,.9,.9),
             (19.5,0,.08,.8,.82),(20,0,.05,.45,.55)],RUST,OXIDE,n=n)
    for side in [-1,1]:
        wing=Mesh();wing.box((side*14.1,12.65,0),(25.8,5.7,.5),RUST,OXIDE)
        for x in np.linspace(1.5,27,(34,22,14)[lod]):
            wing.box((side*x,12.65,.42),(.14,5.8,.35),(161,76,33),OXIDE)
            if lod<2:wing.box((side*x,12.65,-.42),(.14,5.8,.35),(90,36,18),OXIDE)
        m.merge(_transform(wing,heading=-side*3.5))
    for x in np.linspace(-1.05,1.05,(9,7,5)[lod]):
        m.beam((x,8.8,.76),(x,14.8,.9),.07,(168,83,39),OXIDE,4)
    return m


def _horse(lod, up):
    m=Mesh();n=(36,24,16)[lod];rows=(22,15,10)[lod]
    # The neck is a continuous plated shell; head, jaw and ears retain their
    # equine proportions in every LOD instead of dissolving into a thin frame.
    keys=([(0,0,0,4.2,5.5),(9,0,-1,4.0,4.8),(18,0,-2.8,3.5,4.5),
           (24,0,-1.8,2.9,3.8),(27,0,.5,2.4,2.5)] if up else
          [(0,0,0,4.2,5.1),(8,0,-1,3.7,4.5),(16,0,-1.3,3.3,4.1),
           (22,0,.0,3,3.8),(25.8,0,2.5,2.3,2.4)])
    k=np.asarray(keys)
    profile=[tuple(np.interp(y,k[:,0],k[:,j]) if j else y for j in range(5)) for y in np.linspace(-1,k[-1,0],rows)]
    _loft(m,profile,STEEL,n=n,panels=True)
    head=Mesh()
    _loft(head,[(0,0,0,2.5,3.2),(3,0,.4,2.75,3.6),(7,0,1.3,2.05,2.5),
                (10,0,1.9,1.75,1.9),(11.7,0,1.8,1.9,1.9),(12.3,0,1.8,1.2,1.2),(12.6,0,1.8,.2,.2)],STEEL,n=n,panels=True)
    # A head authored upright is tilted forward about its transverse axis.
    angle=math.radians(38 if up else 124);c,s=math.cos(angle),math.sin(angle)
    head.transform([[1,0,0,0],[0,c,-s,22.5 if up else 25.7],[0,s,c,0],[0,0,0,1]])
    m.merge(head)
    for x in [-1.65,1.65]:
        _loft(m,[(24.8 if up else 25.0,x,-1,1.05,1.0),
                 (29.8,x,-1.8,.25,.35)],(155,170,181),n=(16,12,8)[lod],panels=True)
        m.ellipsoid((x*1.6,24.3,3.0 if up else 4.3),(.38,.5,.65),DARK,16,12,6)
    # Plate courses and the ridged mane remain fixed metallic features.
    for row in profile[1:-1:1 if lod==0 else 2]:
        y,x,z,rx,rz=row
        pts=[(x+rx*1.005*math.cos(t),y,z+rz*1.005*math.sin(t))for t in np.linspace(0,math.tau,n+1)]
        _tube(m,pts,.045,DARK,16,4)
    mane=[(0,y,z-rz-.1)for y,x,z,rx,rz in profile]
    _tube(m,mane,.33,(61,78,91),METAL,6)
    m.transform([[1.25,0,0,0],[0,.94,0,0],[0,0,1.20,0],[0,0,0,1]])
    return m


def _kelpies(lod):
    m=Mesh()
    for up,x,z in [(False,-6.2,-15.2),(True,6.2,15.2)]:
        m.merge(_transform(_horse(lod,up),x=x,z=z,heading=90))
        _loft(m,[(-1.65,x,z,7.1,5.8),(.05,x,z,7.1,5.8)],(104,110,104),MASONRY,(48,32,20)[lod])
        if lod<2:
            for t in np.linspace(0,math.tau,12,endpoint=False):
                m.box((x+6.1*math.cos(t),.14,z+6.1*math.sin(t)),(.23,.12,.23),LIGHT,94)
    return m


def _cloud(lod):
    m=Mesh();nx=(96,64,40)[lod];nt=(72,48,32)[lod]
    def ring(t):
        x=10.05*math.sin(t);root=math.cos(t)
        top=5.0+5.06*root
        bottom=max(.12,5-5*root+3.68*math.exp(-(x/3.7)**4)-1.2*math.exp(-((abs(x)-6.2)/2)**2))
        cy=(top+bottom)/2;ry=(top-bottom)/2
        return [np.array([x,cy+ry*math.cos(a),6.4*root*math.sin(a)])for a in np.linspace(0,math.tau,nt,endpoint=False)]
    rings=[ring(t)for t in np.linspace(-math.pi/2,math.pi/2,nx+1)]
    for a,b in zip(rings,rings[1:]):
        for j in range(nt):
            k=(j+1)%nt
            # Neutral silver midtones retain the direct-light specular contrast.
            m.quad(a[k],b[k],b[j],a[j],(205,213,219),METAL)
    m.box((0,-1.95,0),(23,4.1,16),(144,141,128),PAVING)
    return m


def _gold_sun_face(m,lod):
    # Radial metal plates form a shallow dished mask. The central folded ridge,
    # bevelled eye wells and forked crown carry the silhouette at every LOD.
    cy,cz,r=58.8,3.75,5.3
    n,rows=(96,64,40)[lod],(14,10,7)[lod]
    gold=(205,180,109)
    def p(t,rho,back=False):
        return np.array([r*rho*math.cos(t),cy+r*rho*math.sin(t),
                         cz+(-.32 if back else .18+.40*(1-rho*rho))])
    for j in range(n):
        ta,tb=j*math.tau/n,(j+1)*math.tau/n
        for i in range(rows):
            ra,rb=i/rows,(i+1)/rows
            d=((j*13+i*7)%9-4)*2
            col=tuple(int(v+d)for v in gold)
            _quad(m,[p(ta,ra),p(ta,rb),p(tb,rb),p(tb,ra)],col,METAL,(0,0,1))
        _quad(m,[p(ta,1),p(ta,1,True),p(tb,1,True),p(tb,1)],(123,105,62),METAL)
        m.triangle((0,cy,cz-.32),p(tb,1,True),p(ta,1,True),(160,139,84),METAL)
    def q(x,y,z):return (x,cy+y,cz+z)
    # Folded asymmetric planes produce the long blade-like nose, not a decal.
    for side in [-1,1]:
        col=(221,195,120)if side<0 else(167,137,66)
        _quad(m,[q(0,5.03,.20),q(side*.88,1.0,.45),q(side*.68,-1.35,.55),q(0,-1.50,2.45)],col,METAL,(0,0,1))
        _quad(m,[q(0,-1.50,2.45),q(side*.68,-1.35,.55),q(side*.47,-3.10,.42),q(0,-4.98,.20)],col,METAL,(0,0,1))
    _tube(m,[q(0,4.9,.26),q(0,2.5,1.22),q(0,-1.5,2.49)],.055,(64,58,36),16,4)
    _quad(m,[q(-.67,-1.35,.58),q(0,-1.50,2.45),q(.67,-1.35,.58),q(0,-1.72,.57)],(54,48,29),16,(0,-1,0))
    for ex in [-2.2,2.2]:
        ey,ez=cy-.18,cz+.55
        # A dark blue well sits below a silver bevel and segmented instrument
        # lens. These are nested relief surfaces, not black painted eye dots.
        for j in range((48,32,20)[lod]):
            ta,tb=j*math.tau/(48,32,20)[lod],(j+1)*math.tau/(48,32,20)[lod]
            circle=lambda t,rr,z:(ex+rr*math.cos(t),ey+rr*math.sin(t),z)
            _quad(m,[circle(ta,1.00,ez+.18),circle(ta,.82,ez+.02),circle(tb,.82,ez+.02),circle(tb,1.00,ez+.18)],(106,127,145),METAL,(0,0,1))
            m.triangle((ex,ey,ez+.01),circle(ta,.83,ez+.01),circle(tb,.83,ez+.01),(20,42,65),PAINT)
        for angle in [35,145,225,315]:
            t=math.radians(angle)
            pts=[(ex+.76*math.cos(t+da),ey+.76*math.sin(t+da),ez+.08)for da in [-.30,.30]]
            m.triangle((ex+.32*math.cos(t),ey+.32*math.sin(t)-.20,ez+.16),*pts,(188,200,203),METAL)
        m.ellipsoid((ex,ey-.28,ez+.13),(.34,.37,.12),(72,91,106),METAL,(24,16,12)[lod],8)
        _tube(m,[(ex+.39*math.cos(t),ey-.28+.41*math.sin(t),ez+.18)for t in np.linspace(0,math.tau,(33,25,17)[lod])],.055,(174,184,185),METAL,4)
    crown=(53,63,67)
    m.beam((0,64.0,cz),(0,70.0,cz),.075,crown,16,6)
    _disc(m,(0,65.4,cz),.63,.22,crown,16,(32,24,16)[lod])
    for side in [-1,1]:
        _tube(m,[(side*x,y,cz)for x,y in [(0,66.8),(.55,66.95),(1.03,67.55),(1.38,68.85)]],.065,crown,16,5)
        _tube(m,[(side*x,y,cz-.12)for x,y in [(0,66.8),(.30,67.2),(.70,67.8),(.95,69.1)]],.05,crown,16,4)


def _white_sun_face(m,lod):
    cy,cz,r=24.8,6.45,6.1
    n,rows=(96,64,40)[lod],(28,18,12)[lod]
    stone=(170,167,151)
    def relief(x,y):
        value=.25-.65*(x*x+y*y)/(r*r)
        for side in [-1,1]:
            # Eye sockets, bowed brow ridges and cheeks are continuous relief.
            eye=math.exp(-((x-side*2.5)/1.7)**2-((y-.7)/1.05)**2)
            brow_y=1.4+1.0*math.exp(-((x-side*2.7)/1.8)**2)
            brow=math.exp(-((x-side*2.7)/2.0)**4-((y-brow_y)/.45)**2)
            cheek=math.exp(-((x-side*3.2)/1.9)**2-((y+1.2)/1.4)**2)
            value+=-.55*eye+.70*brow+.40*cheek
        value+=1.05*math.exp(-((x+.15)/.70)**2-((y-.1)/1.9)**2)
        return value
    def p(t,rho):
        x,y=r*rho*math.cos(t),r*rho*math.sin(t)
        return (x,cy+y,cz+relief(x,y))
    for j in range(n):
        ta,tb=j*math.tau/n,(j+1)*math.tau/n
        for i in range(rows):
            ra,rb=i/rows,(i+1)/rows
            _quad(m,[p(ta,ra),p(ta,rb),p(tb,rb),p(tb,ra)],stone,PLASTER,(0,0,1))
        pa,pb=p(ta,1),p(tb,1)
        _quad(m,[pa,(pa[0],pa[1],cz-2.0),(pb[0],pb[1],cz-2.0),pb],stone,PLASTER)
    # The forehead crest and the tight, asymmetric lips replace the smiling
    # cartoon face. Small tonal differences reinforce sculpted recesses.
    for side in [-1,1]:
        _quad(m,[(0,cy+6.55,cz-.2),(side*1.2,cy+2.25,cz+.8),
                 (side*.68,cy+.65,cz+1.35),(-.15,cy+1.5,cz+1.8)],(179,175,158),PLASTER,(0,0,1))
        _tube(m,[(side*x,cy+y,cz+relief(side*x,y)+.025)for x,y in [(1.0,.1),(1.7,-.1),(2.7,-.2),(3.7,.1)]],.075,(98,98,88),PLASTER,5)
    m.ellipsoid((-.28,cy-.38,cz+1.22),(1.08,.57,.66),(151,149,132),PLASTER,(32,24,16)[lod],12)
    m.ellipsoid((.39,cy-.67,cz+1.56),(.42,.16,.12),(74,77,67),PLASTER,16,6)
    _tube(m,[(-.70,cy-1.65,cz+.76),(-.15,cy-1.47,cz+.99),(.48,cy-1.67,cz+.83)],.23,(157,153,134),PLASTER,8)
    _tube(m,[(-.61,cy-1.92,cz+.72),(-.12,cy-1.85,cz+.89),(.46,cy-1.94,cz+.76)],.07,(80,83,73),PLASTER,5)
    _tube(m,[(-.35,cy-2.30,cz+.62),(.10,cy-2.20,cz+.85),(.49,cy-2.50,cz+.45)],.25,(161,157,138),PLASTER,8)


def _sun(lod):
    m=Mesh();n=(96,64,40)[lod]
    profile=np.array([(-2,9.0),(0,9.0),(8,8.5),(17,7.5),(26,6.5),
                      (34,5.5),(42,4.5),(50,3.8),(58.8,3.9)])
    radius=lambda y:float(np.interp(y,profile[:,0],profile[:,1]))
    ys=np.linspace(-2,58.8,(80,52,32)[lod])
    _loft(m,[(y,0,0,radius(y),radius(y)*.88)for y in ys],WHITE,PLASTER,n)
    for side in [-1,1]:
        arm=Mesh()
        # Swept tapered arms emerge level with the lower face, with tips
        # curving upward. Local vertical rings are rotated into the span.
        _loft(arm,[(0,0,0,4.9,4.5),(5,0,-.5,4.3,3.7),
                   (12,0,-2.2,3.0,2.6),(19,0,-4.9,1.45,1.35),
                   (23.8,0,-8.2,.07,.12)],WHITE,PLASTER,n//2)
        arm.transform([[0,side,0,side*4.2],[0,0,-1,25.1],[-side,0,0,0],[0,0,0,1]])
        m.merge(arm)
    markings=[(0,.995),(5,.94),(10,.90),(13,.60),(17,.90),(20,.94),
              (25,.94),(29,.90),(32,.62),(34,.91),(36,.95),(38,.55),(41,.995)]
    def inner(y):
        i=min(len(markings)-2,max(0,np.searchsorted([p[0]for p in markings],y)-1))
        a,b=markings[i],markings[i+1];t=(y-a[0])/(b[0]-a[0]);t=t*t*(3-2*t)
        return a[1]+(b[1]-a[1])*t
    def surf(side,y,fraction):
        rx=radius(y);x=side*rx*fraction
        return (x,y,rx*.88*math.sqrt(max(.001,1-fraction*fraction))+.045)
    for side in [-1,1]:
        for a,b in zip(np.linspace(.2,41,(181,121,81)[lod]),np.linspace(.2,41,(181,121,81)[lod])[1:]):
            # Multiple columns keep wide curves flush with the round trunk.
            for u,v in zip(np.linspace(0,1,7),np.linspace(0,1,7)[1:]):
                f=lambda y,t:inner(y)+(.995-inner(y))*t
                _quad(m,[surf(side,a,f(a,u)),surf(side,a,f(a,v)),surf(side,b,f(b,v)),surf(side,b,f(b,u))],(135,32,36),PLASTER,(0,0,1))
    _gold_sun_face(m,lod)
    _white_sun_face(m,lod)
    m.ellipsoid((0,24.8,-6.4),(4.2,4.2,.35),(21,24,20),PLASTER,n,16)
    return m


# Deliberately simplified, hand-drawn continental silhouettes in lon/lat.
# No external map or texture asset is sampled into the sculpture.
_LAND = [
 [(-168,67),(-145,70),(-126,58),(-113,49),(-83,50),(-60,52),(-53,46),(-70,40),(-81,24),(-97,18),(-106,25),(-120,34),(-130,52),(-164,57)],
 [(-81,12),(-66,10),(-50,0),(-35,-7),(-42,-23),(-52,-33),(-68,-55),(-76,-40),(-70,-17),(-81,0)],
 [(-18,35),(5,37),(22,32),(35,30),(43,12),(50,10),(42,-15),(32,-29),(19,-35),(10,-20),(8,0),(-15,6)],
 [(-10,36),(-10,58),(5,60),(20,71),(45,69),(70,74),(115,73),(155,63),(177,55),(152,45),(135,34),(121,22),(107,4),(96,8),(82,22),(73,8),(63,25),(45,30),(34,41),(19,40),(7,43)],
 [(113,-22),(126,-12),(141,-12),(153,-25),(145,-39),(127,-35),(115,-34)],
 [(-55,60),(-25,68),(-19,80),(-44,84),(-65,75)],
 [(47,-13),(50,-19),(45,-26),(43,-20)],[(130,31),(141,42),(145,44),(141,34)],
 [(166,-34),(179,-39),(169,-47),(166,-43)],
]


def _inside(x,y,polygon):
    inside=False
    for a,b in zip(polygon,polygon[1:]+polygon[:1]):
        if (a[1]>y)!=(b[1]>y) and x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:inside=not inside
    return inside


def _unisphere(lod):
    m=Mesh();n=(96,64,40)[lod];r=18.3
    # Tilt and orient the entire globe, including the raised continental plates.
    globe=Mesh()
    def point(lat,lon,radius=r):
        a,b=math.radians(lat),math.radians(lon)
        return np.array([radius*math.cos(a)*math.sin(b),radius*math.sin(a),radius*math.cos(a)*math.cos(b)])
    for lat in range(-75,76,15):
        _tube(globe,[point(lat,t)for t in np.linspace(-180,180,n+1)],.10,STEEL,METAL,(6,5,4)[lod])
    for lon in range(0,180,15):
        _tube(globe,[point(t,lon)for t in np.linspace(-180,180,n+1)],.11,STEEL,METAL,(6,5,4)[lod])
    step=(2,3,5)[lod]
    for lat in range(-60,85,step):
        for lon in range(-180,180,step):
            if any(_inside(lon+step/2,lat+step/2,poly)for poly in _LAND):
                p=[point(lat,lon,r+.14),point(lat+step,lon,r+.14),point(lat+step,lon+step,r+.14),point(lat,lon+step,r+.14)]
                _quad(globe,p,(174,187,196),METAL,sum(p))
                _quad(globe,p[::-1],(174,187,196),METAL)
    a=math.radians(23.5);c,s=math.cos(a),math.sin(a)
    globe.transform([[c,-s,0,0],[s,c,0,24.5],[0,0,1,0],[0,0,0,1]])
    m.merge(globe)
    for tilt,spin in [(28,0),(-35,50),(58,100)]:
        a,b=math.radians(tilt),math.radians(spin)
        pts=[]
        for t in np.linspace(0,math.tau,n+1):
            x,z=22*math.cos(t),22*math.sin(t)
            y=z*math.sin(a);z*=math.cos(a)
            pts.append((x*math.cos(b)+z*math.sin(b),24.5+y,-x*math.sin(b)+z*math.cos(b)))
        _tube(m,pts,.16,STEEL,METAL,(8,6,4)[lod])
    for x,z in [(-2,-1.3),(2,-1.3),(0,2.5)]:m.beam((x*1.6,0,z*1.6),(x,8,z),.65,DARK,16,8)
    _disc(m,(0,.25,0),4.8,2,STONE,MASONRY,n)
    return m


def _spire(lod):
    m=Mesh();n=(64,40,24)[lod]
    profile=[(-2,0,0,1.5,1.5),(0,0,0,1.5,1.5),(10,0,0,1.38,1.38),
             (60,0,0,.79,.79),(108,0,0,.218,.218),(120,0,0,.075,.075)]
    _loft(m,profile,STEEL,METAL,n)
    _loft(m,[(108,0,0,.221,.221),(120,0,0,.078,.078)],(220,237,247),94,n)
    _disc(m,(0,.04,0),2.7,.5,(126,99,60),16,n)
    if lod<2:
        for k in range(12):
            pts=[]
            for y in np.linspace(.1,9.8,24):
                a=k*math.tau/12+y*.20;r=1.503-y*.012
                pts.append((r*math.cos(a),y,r*math.sin(a)))
            _tube(m,pts,.028,(70,86,96),METAL,4)
    return m


def _bridge(lod):
    m=Mesh();iron=(63,78,83);side_count=(8,6,4)[lod]
    def beam(a,b,r=.22,col=iron):m.beam(a,b,r,col,16,side_count)
    # Both decks use surveyed approach datums; the main arch springs from
    # masonry on each bank. The lower deck remains open to the river below.
    low=lambda z: float(np.interp(z,[-89,89],[7.280597,5.626709]))
    upper=lambda z: float(np.interp(z,[-174,236],[51.438747,70.568466]))
    for za,zb in [(-174,236),(-89,89)]:
        grade=upper if zb>100 else low
        steps=(48,32,24)[lod]
        for a,b in zip(np.linspace(za,zb,steps+1),np.linspace(za,zb,steps+1)[1:]):
            ya,yb=grade(a),grade(b)
            for dy,col,finish in [(0,(49,55,52),ASPHALT),(-1.25,iron,METAL)]:
                _quad(m,[(-4,ya+dy,a),(4,ya+dy,a),(4,yb+dy,b),(-4,yb+dy,b)],col,finish,(0,1 if dy==0 else -1,0))
            for x in [-4,4]:
                _quad(m,[(x,ya-1.25,a),(x,ya,a),(x,yb,b),(x,yb-1.25,b)],iron,16,(x,0,0))
        for x in [-4.2,4.2]:
            beam((x,grade(za)+1.15,za),(x,grade(zb)+1.15,zb),.10)
            beam((x,grade(za)+.45,za),(x,grade(zb)+.45,zb),.07)
            for z in np.linspace(za,zb,(110,70,42)[lod]):beam((x,grade(z),z),(x,grade(z)+1.2,z),.065)
    for x in [-2.0,-.57,.57,2.0]:beam((x,upper(-174)+.09,-174),(x,upper(236)+.09,236),.07,(118,123,119))
    # The open lattice is two braced parabolic ribs, 172.5m between springings.
    zs=np.linspace(-86.25,86.25,(48,32,24)[lod])
    arch=lambda z: 4+49*(1-(z/86.25)**2)
    for x in [-3.2,3.2]:
        for offset in [-1.4,1.4]:_tube(m,[(x,arch(z)+offset,z)for z in zs],.42,iron,16,side_count)
        for i,(a,b)in enumerate(zip(zs,zs[1:])):
            beam((x,arch(a)-1.4,a),(x,arch(b)+1.4,b),.16)
            beam((x,arch(a)+1.4,a),(x,arch(b)-1.4,b),.16)
        for z in np.linspace(-80,80,(17,13,9)[lod]):
            beam((x,low(z)-1.1,z),(x,arch(z)-1.5,z),.18)
            if upper(z)-arch(z)>2:beam((x,arch(z)+1.4,z),(x,upper(z)-1.2,z),.20)
    for z in zs[::2]:beam((-3.2,arch(z),z),(3.2,arch(z),z),.20)
    for z in [-150,-112,113,154,196]:
        bottom=(7 if z<0 else 18)+(abs(z)-100)*.17
        for x in [-3.0,3.0]:
            beam((x*1.5,bottom,z),(x,upper(z)-1.2,z),.36)
            beam((-3,bottom,z),(3,upper(z)-1.2,z),.20)
            beam((3,bottom,z),(-3,upper(z)-1.2,z),.20)
    for z in [-89,89]:
        for x in [-3.4,3.4]:m.box((x,4,z),(4,16,7),STONE,MASONRY)
    return m


def _arcade(m, centers, spring, radius, thickness, depth, color, n, bottom, top):
    # Voussoirs form the intrados and outer wall separately, leaving real
    # openings with closed soffits rather than dark painted arches.
    for x in centers:
        for j in range(n):
            a,b=math.pi*j/n,math.pi*(j+1)/n
            ri,ro=radius,radius+thickness
            p=lambda r,t,z:(x+r*math.cos(t),spring+r*math.sin(t),z)
            shade=tuple(min(255,v+(j%3-1)*6)for v in color)
            for z,normal in [(-depth/2,(0,0,-1)),(depth/2,(0,0,1))]:
                _quad(m,[p(ri,a,z),p(ro,a,z),p(ro,b,z),p(ri,b,z)],shade,MASONRY,normal)
            _quad(m,[p(ri,a,-depth/2),p(ri,b,-depth/2),p(ri,b,depth/2),p(ri,a,depth/2)],shade,MASONRY,(-math.cos((a+b)/2),-math.sin((a+b)/2),0))
            # Spandrels fill to the level course over every opening.
            for z,normal in [(-depth/2,(0,0,-1)),(depth/2,(0,0,1))]:
                _quad(m,[p(ro,a,z),(x+ro*math.cos(a),top,z),(x+ro*math.cos(b),top,z),p(ro,b,z)],color,MASONRY,normal)
    pitch=centers[1]-centers[0] if len(centers)>1 else radius*2+thickness
    width=pitch-2*radius
    for x in [centers[0]-pitch/2]+[v+pitch/2 for v in centers]:
        m.box((x,(spring+bottom)/2,0),(width,spring-bottom,depth),color,MASONRY)
        m.box((x,(spring+top)/2,0),(max(.1,width-2*thickness),top-spring,depth),color,MASONRY)
        m.box((x,spring-.12,0),(width+.24,.35,depth+.2),color,MASONRY)
        if n>10:
            for y in np.arange(bottom+1,spring,1.4):
                for z in [-depth/2-.015,depth/2+.015]:m.box((x,y,z),(width,.032,.03),(102,81,51),MASONRY)
    m.box(((centers[0]+centers[-1])/2,top+.18,0),(pitch*len(centers)+width,.36,depth+.1),color,MASONRY)


def _pont(lod):
    m=Mesh();n=(24,16,10)[lod]
    # Surviving top gallery: 35 small openings; six and eleven larger bays
    # below. The extended side gallery rests on the natural valley slopes.
    _arcade(m,[-55,-33,-11,11,33,55],10.0,9.4,1.6,6.0,STONE,n,-8,21.2)
    _arcade(m,[v*22 for v in range(-5,6)],29.0,9.4,1.3,4.5,STONE,n,21.2,40.3)
    _arcade(m,[(v-17)*7.4 for v in range(35)],43.2,2.5,.7,3.1,STONE,max(6,n//2),40.3,47.5)
    m.box((0,48.3,0),(261,1.2,3.6),STONE,MASONRY)
    for z in [-1.7,1.7]:m.box((0,49,z),(261,.6,.28),(176,147,101),MASONRY)
    # The retained pedestrian bridge is alongside the lowest Roman tier.
    m.box((0,21.0,4.6),(142,.7,3.0),(156,131,90),MASONRY)
    for z in [3.1,6.1]:m.box((0,21.65,z),(142,.8,.3),STONE,MASONRY)
    return m


_PIER_OUTLINE = [[-172.273, 71.457], [-190.009, 89.988], [-190.142, 93.36], [-258.559, 164.184], [-251.316, 170.632], [-242.886, 162.318], [-242.564, 165.422], [-243.673, 165.672], [-243.285, 168.268], [-242.298, 168.027], [-242.01, 170.78], [-235.654, 176.175], [-227.346, 175.713], [-209.931, 190.438], [-208.067, 188.387], [-202.643, 188.064], [-198.872, 191.842], [-198.739, 196.11], [-182.777, 195.417], [-170.165, 207.278], [-168.08, 207.176], [-166.305, 205.643], [-166.117, 203.749], [-170.742, 198.511], [-159.018, 186.576], [-154.448, 190.872], [-152.041, 190.78], [-150.488, 188.821], [-150.676, 186.77], [37.669, -9.704], [-19.988, -65.795], [28.063, -115.484], [32.423, -111.235], [46.421, -125.83], [44.38, -127.945], [48.384, -131.714], [46.31, -133.987], [53.686, -141.469], [55.45, -139.77], [60.907, -145.312], [58.922, -147.197], [61.362, -149.672], [70.191, -141.072], [104.122, -175.99], [92.331, -187.168], [92.143, -189.496], [95.326, -193.117], [69.626, -217.753], [46.809, -194.669], [53.598, -187.685], [39.056, -172.748], [54.13, -158.051], [38.202, -141.737], [34.253, -145.552], [20.332, -131.539], [22.318, -129.322], [17.914, -124.888], [20.243, -122.745], [8.707, -111.142], [7.01, -112.897], [1.364, -107.142], [3.183, -105.507], [-146.106, 47.615], [-146.595, 47.227], [-151.919, 53.573], [-164.686, 41.805], [-177.52, 55.633], [-175.002, 58.091], [-179.061, 62.211], [-174.569, 67.06], [-175.601, 68.076]]


def _pier(m,lod):
    # Ear clipping preserves the concave, mapped deck outline; it does not
    # bridge the open-water notch beside the amusement platform.
    p=np.asarray(_PIER_OUTLINE);area=np.sum(p[:,0]*np.roll(p[:,1],-1)-p[:,1]*np.roll(p[:,0],-1))
    if area<0:p=p[::-1]
    def cross(a,b):return a[0]*b[1]-a[1]*b[0]
    ids=list(range(len(p)))
    while len(ids)>2:
        for j in range(len(ids)):
            a,b,c=[p[ids[k%len(ids)]]for k in [j-1,j,j+1]]
            if cross(b-a,c-b)<=1e-7:continue
            others=[p[k]for k in ids if k not in [ids[j-1],ids[j],ids[(j+1)%len(ids)]]]
            if any(cross(b-a,q-a)>=-1e-8 and cross(c-b,q-b)>=-1e-8 and cross(a-c,q-c)>=-1e-8 for q in others):continue
            xz=lambda q,y:(q[0],y,q[1])
            m.triangle(xz(c,0),xz(b,0),xz(a,0),(112,91,65),TIMBER)
            m.triangle(xz(a,-1),xz(b,-1),xz(c,-1),(67,54,38),TIMBER)
            del ids[j];break
        else:raise ValueError('pier outline cannot be triangulated')
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        _quad(m,[(a[0],-1,a[1]),(a[0],0,a[1]),(b[0],0,b[1]),(b[0],-1,b[1])],(84,65,43),TIMBER)
        count=max(1,math.ceil(np.linalg.norm(b-a)/(10,16,24)[lod]))
        for t in np.linspace(0,1,count,endpoint=False):
            q=a*(1-t)+b*t
            m.beam((q[0],-9,q[1]),(q[0],0,q[1]),.35,(58,44,28),TIMBER,(8,6,4)[lod])
    lo,hi=p.min(axis=0),p.max(axis=0)
    for x in np.arange(lo[0]+10,hi[0],20):
        for z in np.arange(lo[1]+10,hi[1],20):
            if _inside(x,z,p.tolist()):m.beam((x,-9,z),(x,-.3,z),.35,(58,44,28),TIMBER,(8,6,4)[lod])


def _wheel(lod):
    m=Mesh();_pier(m,lod);n=(120,80,48)[lod];r=12.95;cy=15.7
    palette=[(184,25,45),(16,127,177),(224,147,20),(90,43,160),(20,145,89)]
    lights=[(249,93,116),(66,193,225),(244,193,62),(149,121,226),(90,207,155)]
    for z in [-1.3,1.3]:
        for rr,thick in [(r,.22),(r-.55,.10)]:
            _tube(m,[(rr*math.cos(t),cy+rr*math.sin(t),z)for t in np.linspace(0,math.tau,n+1)],thick,WHITE,PAINT,(8,6,4)[lod])
        for j in range(20):
            t=math.tau*j/20;x,y=r*math.cos(t),cy+r*math.sin(t)
            m.beam((0,cy,z),(x,y,z),.11,WHITE,PAINT,6)
            m.beam((0,cy,z*1.06),(x,y,z*1.06),.045,lights[j%5],94,4)
            if j%2==0:m.beam((0,cy,z),(x,y,-z),.07,WHITE,PAINT,4)
    for j in range(20):
        t=math.tau*j/20;x,y=r*math.cos(t),cy+r*math.sin(t)
        m.beam((x,y,-1.4),(x,y,1.4),.10,WHITE,PAINT,6)
        m.box((x,y-1.05,0),(1.7,1.05,2.35),palette[j%5],PAINT)
        m.box((x,y+.55,0),(1.95,.15,2.6),(181,177,156),PAINT)
        for xx in [-.78,.78]:
            for zz in [-1.1,1.1]:m.beam((x+xx,y-.55,zz),(x+xx,y+.5,zz),.045,WHITE,PAINT,4)
    for z in [-2.2,2.2]:
        for x in [-6,6]:m.beam((x,.2,z*1.8),(0,cy,z),.55,(193,185,163),PAINT,(12,8,6)[lod])
    m.beam((0,cy,-2.7),(0,cy,2.7),.7,(165,111,35),METAL,16)
    m.box((0,-.1,0),(15,.7,10),(98,73,44),TIMBER)
    for x in [-6,0,6]:
        for z in [-4,4]:m.beam((x,-9,z),(x,0,z),.35,(64,47,28),TIMBER,6)
    return m


_COPA_SURVEY = [[92.07161, 13.01384, 138.58254], [83.52994, 12.83424, 127.09859], [63.70875, 10.64243, 97.46403], [27.28617, 9.0, 43.00112], [0.0, 11.43708, 0.0], [-0.90236, 11.57991, -1.42857], [-3.46589, 12.07334, -6.00222], [-21.69769, 13.93889, -37.24253], [-24.28172, 13.67894, -41.5947], [-38.63747, 10.9306, -69.55705], [-53.92634, 8.45269, -99.34665], [-61.24777, 8.0, -114.13071]]


_COPA_BASE = 11.50849104
_COPA_PATH = [[p[0]*math.cos(math.radians(34))-p[2]*math.sin(math.radians(34)),
               0.0,p[0]*math.sin(math.radians(34))+p[2]*math.cos(math.radians(34))]
              for p in _COPA_SURVEY]


def _copa_at(s, offset=0):
    p=np.asarray(_COPA_PATH);d=np.r_[0,np.cumsum(np.linalg.norm(np.diff(p[:,[0,2]],axis=0),axis=1))]
    i=min(len(p)-2,max(0,int(np.searchsorted(d,s,side='right')-1)))
    t=(s-d[i])/(d[i+1]-d[i]);c=p[i]*(1-t)+p[i+1]*t
    delta=p[i+1]-p[i];normal=np.array([-delta[2],0,delta[0]]);normal/=np.linalg.norm(normal)
    return c+offset*normal


def _copa_length():
    return float(np.linalg.norm(np.diff(np.asarray(_COPA_PATH)[:,[0,2]],axis=0),axis=1).sum())


def _copa(lod):
    m=Mesh();length=_copa_length();steps=math.ceil(length/(.4,.7,1.1)[lod])
    for a,b in zip(np.linspace(0,length,steps+1),np.linspace(0,length,steps+1)[1:]):
        pa,pb=_copa_at(a),_copa_at(b)
        def pos(s,o,dy=.22):return _copa_at(s,o)+[0,dy,0]
        _quad(m,[pos(a,-4.5),pos(a,4.5),pos(b,4.5),pos(b,-4.5)],(196,190,169),PAVING,(0,1,0))
        for center in [-4,0,4]:
            off=lambda s: center+1.15*math.sin(s*math.tau/10)
            lo=lambda s:max(-4.5,min(4.5,off(s)-.7))
            hi=lambda s:max(-4.5,min(4.5,off(s)+.7))
            _quad(m,[pos(a,lo(a),.235),pos(a,hi(a),.235),pos(b,hi(b),.235),pos(b,lo(b),.235)],(20,26,23),PAVING,(0,1,0))
        for o in [-4.5,4.5]:
            _quad(m,[pos(a,o,-2),pos(a,o),pos(b,o),pos(b,o,-2)],(132,129,112),MASONRY)
    return m


def _patch(half,base,center=(0,0),feather=1):
    return dict(center_xz=list(center),half_extent=list(half),feather_m=feather,
                sample_clearance_m=0,fixed_base_geo=base)


def _copa_patches():
    # Short bounded pads follow the surveyed walk; the adjacent carriageways
    # and beach retain their source terrain outside the narrow paving strip.
    return [_patch((4.6,5.5),_COPA_BASE,(float(p[0]),float(p[2])),feather=1.0)
            for s in np.linspace(0,_copa_length(),math.ceil(_copa_length()/9)+1)
            for p in [_copa_at(s)]]


def _copa_exclusions():
    p=np.array(_COPA_PATH)
    polygons=[]
    for a,b in zip(p,p[1:]):
        d=b[[0,2]]-a[[0,2]];d/=np.linalg.norm(d);n=np.array([-d[1],d[0]])*4.4
        # Small end overlap removes the source footway's segment joins.
        aa=a[[0,2]]-d*.4;bb=b[[0,2]]+d*.4
        polygons.append([list(aa-n),list(aa+n),list(bb+n),list(bb-n)])
    return polygons


ENTRIES = [
    dict(name='angel-of-the-north',city='Gateshead',lat=54.9141164,lon=-1.58952455,heading=152.64,base=87.49531555,
         suppress_buildings=False,suppress_roads=False,suppress_trees=True,
         references=['https://www.ice.org.uk/what-is-civil-engineering/infrastructure-projects/the-angel-of-the-north','https://www.openstreetmap.org/way/922533965'],
         description='A weathering-steel human figure with long ribbed wings swept slightly forward; 20m tall with a 54m span.',
         source_audit=dict(center_ground_m=87.49531555,foot_ground_range_m=[87.10557,87.63136],placement='OSM sculpture outline principal axis; narrow feet descend one metre into the retained grassy mound.'),
         capture=dict(distance=91,azimuth=155,elevation=17,target_height=9,time_utc='2026-03-20T15:30:00Z')),
    dict(name='the-kelpies',city='Falkirk',lat=56.0192179,lon=-3.7556864,heading=0,base=5.82931757,
         suppress_buildings=True,suppress_roads=False,suppress_trees=True,suppress_bbox=[[-15,-23],[19,23]],
         references=['https://www.thehelix.co.uk/venues/the-helix','https://www.openstreetmap.org/way/277313677','https://www.openstreetmap.org/way/277313678'],
         description='Two plated silver horse heads, one raised and one bowed, with pointed ears, ridged manes and steady low base lights.',
         source_audit=dict(center_ground_m=5.82931757,perimeter_ground_range_m=[5.46787,5.9669],placement='The two mapped sculpture footprints are 30.4m apart north-south and 12.4m east-west. Existing canal and moats retain their terrain.'),
         capture=dict(distance=106,azimuth=137,elevation=19,target_height=12,time_utc='2026-03-20T15:00:00Z')),
    dict(name='cloud-gate',city='Chicago',lat=41.88268615,lon=-87.62334065,heading=86.05,base=178.87258911,
         suppress_buildings=True,suppress_roads=True,suppress_trees=True,suppress_bbox=[[-12,-8],[12,8]],
         terrain_flatten=[_patch((12,8),178.87258911)],
         references=['https://www.choosechicago.com/articles/tours-and-attractions/the-bean-chicago/','https://www.openstreetmap.org/way/137060274'],
         description='A continuous silver bean with a walk-under central arch on a small stone pad; 20.1m long, 12.8m wide and 10.06m high.',
         source_audit=dict(perimeter_median_m=178.87258911,placement='Mapped long axis bearing 356 degrees; four corner source elevations define the local corrected pad.'),
         capture=dict(distance=44,azimuth=115,elevation=13,target_height=4.3,time_utc='2026-03-20T16:00:00Z')),
    dict(name='unisphere',city='Queens, New York',lat=40.746396,lon=-73.8450737,heading=0,base=5.9727993,
         suppress_buildings=False,suppress_roads=False,suppress_trees=True,
         references=['https://www.nycgovparks.org/parks/flushing-meadows-corona-park/monuments/1597','https://s-media.nyc.gov/agencies/lpc/lp/1925.pdf','https://www.openstreetmap.org/way/596273458'],
         description='An open 36.6m steel globe with a tilted axis, raised continental silhouettes and three orbit rings, on a three-legged support.',
         source_audit=dict(center_ground_m=5.9727993,foot_ground_range_m=[5.93596,5.96791],placement='Mapped globe center; support and foundation remain inside the existing pool without flattening it.'),
         capture=dict(distance=109,azimuth=35,elevation=17,target_height=22,time_utc='2026-03-20T17:00:00Z')),
    dict(name='tower-of-the-sun',city='Suita, Osaka',lat=34.8095167,lon=135.5324005,heading=180,base=39.56106758,
         suppress_buildings=True,suppress_roads=False,suppress_trees=True,suppress_bbox=[[-31,-11],[31,11]],
         terrain_flatten=[_patch((11,11),39.56106758,feather=4)],
         references=['https://taiyounotou-expo70.jp/en/about/','https://www.openstreetmap.org/way/1368703227'],
         description='A 70m ivory sculptural trunk with upswept arms, a radially plated gold mask with folded nose, recessed eye instruments and forked crown, a sculpted lower face, a black rear sun and curved red markings.',
         source_audit=dict(center_ground_m=39.01441956,perimeter_median_m=39.56106758,placement='Mapped sculpture center; the corrected trunk footprint removes local DEM variation while retaining the park slope outside it.'),
         capture=dict(distance=161,azimuth=190,elevation=16,target_height=32,time_utc='2026-03-20T05:00:00Z')),
    dict(name='dublin-spire',city='Dublin',lat=53.34981,lon=-6.26026,heading=0,base=10.97794533,
         suppress_buildings=True,suppress_roads=False,suppress_trees=True,
         references=['https://www.ritchie.studio/projects/dublin/'],
         description='A slender 120m steel needle, tapering from a 3m base to a 15cm tip, with a bronze ground disc and steady tip illumination.',
         source_audit=dict(center_ground_m=10.97794533,foot_ground_range_m=[10.91882,11.03603],placement='OConnell Street median; replaces the surveyed 121.2m generic source tower (seed 110781263095970683) within its 2.2m footprint while preserving surrounding streets.'),
         capture=dict(distance=234,azimuth=175,elevation=14,target_height=52,time_utc='2026-03-20T17:50:00Z')),
    dict(name='dom-luis-i-bridge',city='Porto',lat=41.1398745,lon=-8.6094167,heading=346.7,base=0,
         suppress_buildings=False,suppress_roads=True,suppress_trees=False,activation_pad=2,
         road_exclusion_polygons=[[[-5.6,-175],[5.6,-175],[5.6,237],[-5.6,237]]],
         references=['https://www.infraestruturasdeportugal.pt/pt-pt/reabertura-do-tabuleiro-inferior-da-ponte-luiz-i','https://www.openstreetmap.org/way/165660759','https://www.openstreetmap.org/way/31572985'],
         description='Two iron decks above the Douro, supported by a 172.5m open lattice arch with 49m rise, suspended lower-deck rods and braced approach piers.',
         source_audit=dict(upper_end_ground_m=[46.438747,65.568466],lower_end_ground_m=[2.280597,.626709],placement='Mapped bridge axis; authored deck endpoints use source ground plus the existing five-metre bridge clearance. Bounded corridor removes duplicate source decks.'),
         capture=dict(distance=404,azimuth=255,elevation=17,target_height=32,time_utc='2026-03-20T15:30:00Z')),
    dict(name='pont-du-gard',city='Near Nimes',lat=43.947251,lon=4.535305,heading=42,base=17.64553642,
         suppress_buildings=True,suppress_roads=True,suppress_trees=True,suppress_bbox=[[-135,-4],[135,7]],
         road_exclusion_polygons=[[[-77,2.8],[77,2.8],[77,7.0],[-77,7.0]]],activation_pad=1,
         references=['https://pontdugard.fr/en/discover/monument','https://pontdugard.fr/sites/default/files/2023-06/PlanGestionUNESCO.pdf'],
         description='A complete bounded Roman bridge-aqueduct with six lower arches, eleven middle arches and a surviving small-arch gallery, in warm limestone.',
         source_audit=dict(river_ground_m=17.64553642,placement='Production aqueduct footprint principal axis bearing 132 degrees; its center is 10m west and 14m south of the initial river sample. Valleyside terrain remains unchanged.'),
         capture=dict(distance=330,azimuth=222,elevation=20,target_height=22,time_utc='2026-03-20T14:00:00Z')),
    dict(name='pacific-wheel',city='Santa Monica',lat=34.0082275,lon=-118.49783405,heading=90,base=5,
         suppress_buildings=True,suppress_roads=False,suppress_trees=False,suppress_bbox=[[-15,-5],[15,5]],
         references=['https://pacpark.com/santa-monica-amusement-park/ferris-wheel/','https://pacpark.com/the-pacific-wheel/','https://www.openstreetmap.org/way/742372514','https://www.openstreetmap.org/way/22728853'],
         description='A stationary 25.9m Ferris wheel with twenty colorful level gondolas, steady radial lights, twin rims and supporting legs on a timber pier deck with supporting piles.',
         source_audit=dict(center_ground_m=0,placement='Mapped wheel plane runs north-south. A bounded deck follows the mapped pier outline at a five-metre datum, filling the missing source platform while preserving the surrounding pier buildings; piles descend below water.'),
         capture=dict(distance=106,azimuth=235,elevation=18,target_height=13,time_utc='2026-03-21T01:45:00Z')),
    dict(name='copacabana-promenade',city='Rio de Janeiro',lat=-22.9751921,lon=-43.1870711,heading=34,base=_COPA_BASE,
         suppress_buildings=False,suppress_roads=True,suppress_trees=False,
         terrain_flatten=_copa_patches(),road_exclusion_polygons=_copa_exclusions(),activation_pad=2,
         references=['https://riotur.rio/que_fazer/copacabana-e-leme/','https://mam.rio/lugar-de-estar/avenida-atlantica-ev/','https://www.openstreetmap.org/way/42292908'],
         description='A nine-metre-wide, roughly 295m beachfront stretch of the famous black-and-white wave paving, following the mapped promenade and surveyed grade.',
         source_audit=dict(path_ground_samples_m=[p[1]for p in _COPA_SURVEY],paving_datum_m=_COPA_BASE,placement='Twelve mapped walk vertices define the curve. A common median source datum anchors the connected paving and narrow terrain pads; nearby kiosks, carriageways and beach remain outside the replacement corridor.'),
         capture=dict(distance=270,azimuth=127,elevation=55,target_height=1,time_utc='2026-03-20T17:30:00Z')),
]


_BUILDERS = dict(zip([e['name'] for e in ENTRIES],[_angel,_kelpies,_cloud,_unisphere,_sun,_spire,_bridge,_pont,_wheel,_copa]))


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('city models have exactly three LODs')
    return _BUILDERS[name](lod)
