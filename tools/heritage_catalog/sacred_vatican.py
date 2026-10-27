"""St Peter's Basilica and Bernini's piazza, in surveyed east/up/north metres."""
import json
import math
from pathlib import Path
import numpy as np
from heritage_catalog.geometry import Mesh, arch, arch_bay, balustrade, column, face, lathe, plan_solid, plan_triangles, ring, tube, turn, window
from heritage_catalog.materials import WEATHERED_LIMESTONE, VEINED_MARBLE, METAL_ROOF, TILE_ROOF

REF=Path(__file__).with_name('references')/'sacred_vatican'
SURVEY=json.loads((REF/'st-peters-basilica-survey.json').read_text())
STONE=(211,204,184); TRIM=(230,224,205); LEAD=(107,116,117); TILE=(144,97,74); DARK=(43,47,45); GOLD=(195,158,71)
BASE=24.5035
FLOOR=3.3
CX,CZ=-46.7,-.8


def part(i):return next(b for b in SURVEY['buildings']if b['id']==i)
def poly(i):return np.asarray(part(i)['rings'][0],float)
def center(i):
    p=poly(i);return (p.min(0)+p.max(0))/2

def grade(x,z):return (300-x)*.008+z*.001


def clip(p,z,positive):
    result=[]
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        ia=(a[1]>=z) if positive else (a[1]<=z)
        ib=(b[1]>=z) if positive else (b[1]<=z)
        if ia:result.append(a)
        if ia!=ib:result.append(a+(b-a)*(z-a[1])/(b[1]-a[1]))
    return np.asarray(result)


BASILICA=[clip(poly(432),0,True)[::-1],clip(poly(1267),0,False)[::-1]]


def courses(m,a,b,lo,hi,lod):
    a,b=np.asarray(a,float),np.asarray(b,float);v=b-a;length=np.linalg.norm(v)
    if length<2:return
    v/=length;out=np.array([v[1],-v[0]])
    step=(1.9,3.8,8)[lod];dy=(.85,1.7,3.4)[lod]
    for row,y in enumerate(np.arange(lo,hi,dy)):
        for col,u in enumerate(np.arange(-step*.5*(row%2),length,step)):
            u0=max(0,u);u1=min(length,u+step)
            if u1-u0<.04:continue
            p=a+v*(u0+.015)+out*.016;q=a+v*(u1-.015)+out*.016
            color=tuple(c+(-3,1,3,-1)[(row+col*3)%4]-int(max(0,4-(y-lo))) for c in STONE)
            face(m,[(p[0],y+.015,p[1]),(q[0],y+.015,q[1]),(q[0],min(hi,y+dy)-.015,q[1]),(p[0],min(hi,y+dy)-.015,p[1])],color,WEATHERED_LIMESTONE,(out[0],0,out[1]))


def slab(m,p,lo,hi,color=STONE,material=WEATHERED_LIMESTONE):plan_solid(m,p,lo,hi,color,material)

def cross(m,x,y,z,h,w,lod):
    m.box((x,y+h/2,z),(.18*h,h,.18*h),GOLD,95)
    m.box((x,y+h*.69,z),(w,.18*h,.18*h),GOLD,95)


def statue(m,x,y,z,h,lod,pose=0):
    """Draped silhouettes retain human anatomy and varied attributes at modest cost."""
    n=(10,8,4)[lod]
    m.box((x,y+.06*h,z),(.55*h,.12*h,.46*h),TRIM,17)
    base=y+.12*h;body=h
    lathe(m,(x,z),[(base,.22*body),(base+.12*body,.24*body),(base+.55*body,.16*body),(base+.72*body,.22*body),(base+.80*body,.10*body)],TRIM,17,n)
    m.ellipsoid((x,base+.88*body,z),(.105*body,.12*body,.11*body),TRIM,17,n,(5,4,3)[lod])
    for side in [-1,1]:
        shoulder=np.array([x+side*.18*body,base+.69*body,z]);elbow=np.array([x+side*.29*body,base+(.50 if (pose+side)%3 else .78)*body,z-.06*body]);hand=elbow+np.array([side*.05*body,.12*body,-.10*body])
        tube(m,shoulder,elbow,.07*body,TRIM,17,4 if lod==2 else 6);tube(m,elbow,hand,.055*body,TRIM,17,4 if lod==2 else 6)
    if lod<2:
        for i in range(6):
            a=i*math.tau/6; tube(m,(x+.22*body*math.cos(a),base+.05*body,z+.22*body*math.sin(a)),(x+.15*body*math.cos(a+.2),base+.53*body,z+.15*body*math.sin(a+.2)),.018*body,(199,193,176),17,4)
    if pose%4==0:tube(m,(x+.31*body,base+.15*body,z-.12*body),(x+.31*body,base+1.05*body,z-.12*body),.025*body,TRIM,17,5)


def pediment(m,x,y,z,w,h,lod):
    face(m,[(x-w/2,y,z),(x+w/2,y,z),(x,y+h,z)],(198,192,174),WEATHERED_LIMESTONE,(0,0,-1))
    for a,b in [((x-w/2,y,z-.12),(x,y+h,z-.12)),((x,y+h,z-.12),(x+w/2,y,z-.12))]:tube(m,a,b,.26 if w<10 else .55,TRIM,17,(8,6,4)[lod])
    m.box((x,y-.15,z-.03),(w+.5,.45,.6),TRIM,17)


def elevation(m,a,b,y,h,lod,spacing=9):
    a,b=np.asarray(a,float),np.asarray(b,float);delta=b-a;length=np.linalg.norm(delta)
    if length<6:return
    angle=math.degrees(math.atan2(-delta[1],delta[0]));q=Mesh()
    count=max(1,int(length/spacing))
    for x in np.linspace(-length/2+length/count/2,length/2-length/count/2,count):
        window(q,x,y+4,-.1,3.4,min(9,h-5),lod,TRIM,False)
        if h>25:window(q,x,y+23,-.12,3.8,8,lod,TRIM,False)
        for side in (-1,1):q.box((x+side*3.4,y+h/2,0),(1.2,h,1.0),TRIM,17)
    for level in [y+2,y+h-2,y+h]:q.box((0,level,0),(length,.65,1.2),TRIM,17)
    m.merge(turn(q,angle,*(np.r_[((a+b)/2)[0],0,((a+b)/2)[1]])))


def main_dome(m,lod):
    n=(96,64,40)[lod];steps=(28,18,12)[lod]
    lathe(m,(CX,CZ),[(48,31.8),(53,31.8),(54,29.45),(72,27.5),(73,28.1)],STONE,WEATHERED_LIMESTONE,n)
    for i in range(16):
        a=i*math.tau/16
        for side in [-1,1]:
            aa=a+side*.040;column(m,CX+28.5*math.sin(aa),CZ+28.5*math.cos(aa),54,17.2,.86,TRIM,lod)
        q=Mesh();window(q,0,56,-27.95,3.5,9,lod,TRIM,False);pediment(q,0,66,-28,5.2,1.5,lod)
        m.merge(turn(q,math.degrees(a)+180,CX,0,CZ))
    for yy,r in [(53,30),(54,29.7),(71,28.2),(73,28.6)]:lathe(m,(CX,CZ),[(yy,r),(yy+.55,r)],TRIM,17,n)
    profile=[]
    for j in range(steps+1):
        t=j/steps;profile.append((73+36.8*t,24.8*(1-t*t)+4.4*t*t))
    def tint(i,j):return tuple(c+(-2,0,2,1)[j%4] for c in LEAD)
    lathe(m,(CX,CZ),profile,tint,METAL_ROOF,n)
    for i in range(16):
        a=i*math.tau/16
        path=[(CX+(r+.1)*math.sin(a),y+.05,CZ+(r+.1)*math.cos(a))for y,r in profile]
        for p,q in zip(path,path[1:]):tube(m,p,q,.42,TRIM,17,(8,6,4)[lod])
        if lod<2:
            for t in [.24,.52,.75]:
                y=73+36.8*t;r=24.8*(1-t*t)+4.4*t*t
                q=Mesh();window(q,0,y,-r-.15,1.6,2.0,lod,TRIM,False);m.merge(turn(q,math.degrees(a)+180,CX,0,CZ))
    lathe(m,(CX,CZ),[(109.8,5.1),(111,5.1),(111,4.1),(121,4.1),(122,4.7)],TRIM,17,(32,24,16)[lod])
    for i in range(16):
        a=i*math.tau/16;column(m,CX+4.5*math.cos(a),CZ+4.5*math.sin(a),111,10,.27,TRIM,lod)
    lathe(m,(CX,CZ),[(122,4.7),(124,3.6),(126.4,1.1)],LEAD,METAL_ROOF,(32,24,16)[lod])
    m.ellipsoid((CX,128.72,CZ),(1.25,)*3,GOLD,95,(24,16,10)[lod],(12,8,5)[lod])
    cross(m,CX,129.97,CZ,3.33,2.4,lod)


def small_dome(m,x,z,lod):
    n=(40,28,16)[lod]
    lathe(m,(x,z),[(FLOOR+31,10.0),(FLOOR+38,9.7),(FLOOR+40,10.4)],STONE,WEATHERED_LIMESTONE,n)
    profile=[(FLOOR+40+12*t,9.5*(1-t*t)+1.3*t*t)for t in np.linspace(0,1,(14,10,6)[lod])]
    lathe(m,(x,z),profile,LEAD,METAL_ROOF,n)
    for i in range(8):
        a=i*math.tau/8;q=Mesh();window(q,0,FLOOR+32,-9.9,2,4.5,lod);m.merge(turn(q,math.degrees(a)+180,x,0,z))
        for (yy,r),(yy1,r1) in zip(profile,profile[1:]):tube(m,(x+r*math.sin(a),yy,z+r*math.cos(a)),(x+r1*math.sin(a),yy1,z+r1*math.cos(a)),.19,TRIM,17,4)
    lathe(m,(x,z),[(FLOOR+52,1.65),(FLOOR+56,1.4),(FLOOR+57,0.35)],TRIM,17,16)
    cross(m,x,FLOOR+57,z,1.9,1.1,lod)


def basilica(m,lod):
    for p in BASILICA:
        slab(m,p,-3,FLOOR+31)
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            if abs(a[1])<1e-6 and abs(b[1])<1e-6:continue
            courses(m,a,b,FLOOR,FLOOR+31,lod);elevation(m,a,b,FLOOR,30,lod)
    # Raised Latin-cross nave and curved tribunes remain legible over lower chapels.
    m.box((-15,FLOOR+40,0),(178,18,36),STONE,WEATHERED_LIMESTONE)
    m.box((CX,FLOOR+40,0),(36,18,127),STONE,WEATHERED_LIMESTONE)
    for x,z,r in [(-106,0,19),(CX,59,18),(CX,-59,18)]:
        lathe(m,(x,z),[(FLOOR+31,r),(FLOOR+49,r)],STONE,WEATHERED_LIMESTONE,(48,32,20)[lod])
        lathe(m,(x,z),[(FLOOR+49,r+.15),(FLOOR+50,r*.78),(FLOOR+53,1)],LEAD,METAL_ROOF,(48,32,20)[lod])
        for i in range(12):
            a=i*math.tau/12;q=Mesh();window(q,0,FLOOR+34,-r-.12,3.5,8,lod);m.merge(turn(q,math.degrees(a),x,0,z))
    for side in [-1,1]:
        a,b=([-105,side*18],[71,side*18])if side<0 else([71,side*18],[-105,side*18])
        elevation(m,a,b,FLOOR+31,18,lod,12)
        face(m,[(-111,FLOOR+49,side*19),(72,FLOOR+49,side*19),(72,FLOOR+54,0),(-111,FLOOR+54,0)],TILE,TILE_ROOF,(0,1,side))
        face(m,[(CX+side*19,FLOOR+49,-67),(CX+side*19,FLOOR+49,67),(CX,FLOOR+54,67),(CX,FLOOR+54,-67)],TILE,TILE_ROOF,(side,1,0))
    for ident in [738,770,1547,1683]:small_dome(m,*center(ident),lod)
    main_dome(m,lod)
    # East front: five portals, five loggias, eight giant columns, attic and clocks.
    q=Mesh();q.box((0,FLOOR+22.75,0),(116,45.5,15.8),STONE,WEATHERED_LIMESTONE)
    for x in [-36,-18,0,18,36]:
        arch(q,(x,FLOOR+1,-8.04),7.2,13,.5,TRIM,False,(28,18,10)[lod],DARK,17)
        window(q,x,FLOOR+21,-8.12,6.0,9,lod,TRIM,False)
        pediment(q,x,FLOOR+31,-8.2,9.0,2.4,lod)
        balustrade(q,(x-3.8,-9.0),(x+3.8,-9.0),FLOOR+21,1.4,lod)
    for x in [-43,-29,-25,-8,8,25,29,43]:
        column(q,x,-8.5,FLOOR+4,27.48,1.385,TRIM,lod)
        if lod<2:
            for a in np.arange(0,math.tau,math.tau/8):q.ellipsoid((x+1.6*math.cos(a),FLOOR+30.5,-8.5+1.6*math.sin(a)),(.28,.6,.28),TRIM,17,6,4)
    for x in [-53,-47,47,53]:q.box((x,FLOOR+17.7,-8.15),(2.4,27.5,.8),TRIM,17)
    for y,t in [(3,1),(32.5,1.3),(35.2,1.0),(44.5,1.0),(45.5,.7)]:q.box((0,FLOOR+y,-.2),(117,t,17.0),TRIM,17)
    pediment(q,0,FLOOR+35.4,-8.85,30,7.5,lod)
    for x in [-46,-37,-27,-18,-9,0,9,18,27,37,46]:
        if abs(x)>16:q.box((x,FLOOR+39.7,-8.12),(3.2,3.8,.1),(129,123,106),15)
    for j,x in enumerate(np.linspace(-46,46,13)):statue(q,x,FLOOR+45.9,-6.8,6 if j==6 else 5.65,lod,j)
    for x in [-53,53]:
        q.box((x,FLOOR+47,-3),(8.0,4,5),TRIM,17)
        q.ellipsoid((x,FLOOR+50.1,-5.9),(3.5,3.9,.7),TRIM,17,(32,20,12)[lod],(12,8,6)[lod])
        # Clock dial faces the square; numerical markers preserve the two clocks.
        ring(q,(x,FLOOR+50,-6.7),2.0,.15,GOLD,(40,28,16)[lod],95,'xy')
        face(q,[(x+2*math.cos(i*math.tau/40),FLOOR+50+2*math.sin(i*math.tau/40),-6.61)for i in range(40)],(226,222,209),17,(0,0,-1))
        for i in range(12):
            a=i*math.tau/12;tube(q,(x+1.7*math.sin(a),FLOOR+50+1.7*math.cos(a),-6.8),(x+1.9*math.sin(a),FLOOR+50+1.9*math.cos(a),-6.8),.055,DARK,16,4)
        tube(q,(x,FLOOR+50,-6.85),(x+.9,FLOOR+51.1,-6.85),.075,DARK,16,5)
        if x<0:tube(q,(x,FLOOR+50,-6.85),(x-1.1,FLOOR+49.7,-6.85),.06,DARK,16,5)
        for side in [-1,1]:statue(q,x+side*3.0,FLOOR+47.5,-5,3.1,lod,side+3)
    m.merge(turn(q,-89.3,81.0,0,-.8))
    # Broad stepped threshold avoids an unsupported thin facade platform.
    m.box((90.55,(FLOOR-2)/2,0),(2.2,FLOOR+2,114),STONE,WEATHERED_LIMESTONE)
    for i in range(12):
        x=92+i*.72;top=FLOOR+(grade(100.3,0)-FLOOR)*(i+.5)/12
        m.box((x,(top-2)/2,0),(.73,top+2,114),STONE,WEATHERED_LIMESTONE)


def promenade(m,p,lod,sloping=False):
    """Mapped colonnade roof has actual void below it and a graded foundation."""
    q=Mesh();slab(q,p,13,16.77)
    # Datum is baked per vertex, so the long arms descend toward the oval.
    q.triangles=[(np.array([[x,y+grade(x,z),z]for x,y,z in points]),c,mat)for points,c,mat in q.triangles]
    m.merge(q)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        v=b-a;length=np.linalg.norm(v)
        if length<.25:continue
        angle=math.degrees(math.atan2(-v[1],v[0]));q=Mesh()
        for h in [13,14,16.77]:q.box((0,h,0),(length+.03,.32,.5),TRIM,17)
        for t in np.arange(0,length,(1.4,2.1,3.4)[lod]):
            q.box((t-length/2,17.5,0),(.26,1.2,.30),TRIM,17)
        q.box((0,18.2,0),(length,.32,.55),TRIM,17)
        turn(q,angle,(a[0]+b[0])/2,0,(a[1]+b[1])/2)
        q.triangles=[(np.array([[x,y+grade(x,z),z]for x,y,z in points]),c,mat)for points,c,mat in q.triangles]
        m.merge(q)


def colonnades(m,lod):
    for ident in [869,929]:promenade(m,poly(ident),lod)
    supports=[b for b in SURVEY['buildings']if b['height']==15 and b['id'] not in [432,1267]]
    # Survey provides 305 small support centres: circular shafts and terminal
    # piers are not distinguished by provider tags. Keep all mapped centres.
    for b in supports:
        p=np.asarray(b['rings'][0]);x,z=(p.min(0)+p.max(0))/2;y=grade(x,z)
        if (x<205 or x>346):
            m.box((x,y+6.5,z),(1.35,13,1.35),STONE,WEATHERED_LIMESTONE)
            m.box((x,y+12.6,z),(1.7,.8,1.7),TRIM,17)
        elif lod==2:lathe(m,(x,z),[(y,.86),(y+.5,.69),(y+12.2,.58),(y+12.5,.88),(y+13,.88)],TRIM,17,6)
        else:column(m,x,z,y,13,.69,TRIM,lod)
        m.box((x,y-.6,z),(1.9,1.2,1.9),STONE,WEATHERED_LIMESTONE)
    for ident in [451,1284]:
        p=poly(ident);q=Mesh();slab(q,p,-3,16.77)
        q.triangles=[(np.array([[x,y+grade(x,z),z]for x,y,z in points]),c,mat)for points,c,mat in q.triangles];m.merge(q)
        promenade(m,p,lod,True)
        # Trapezoid arms: paired giant pilasters, shallow recesses and a roof walk.
        a,b=(p[1],p[0])if ident==451 else(p[3],p[4])
        v=b-a;length=np.linalg.norm(v);q=Mesh()
        for x in np.linspace(-length/2+4,length/2-4,max(2,int(length/7))):
            window(q,x,3,-.05,2.4,7.5,lod,TRIM,False)
            q.box((x+2.7,7,0),(.9,13.5,.65),TRIM,17)
        c=(a+b)/2;turn(q,math.degrees(math.atan2(-v[1],v[0])),float(c[0]),0,float(c[1]))
        q.triangles=[(np.array([[x,y+grade(x,z),z]for x,y,z in points]),c,mat)for points,c,mat in q.triangles];m.merge(q)
    # 70 saints on each side, including straight arms. Individual iconographic
    # identities are deliberately not asserted by these small original sculptures.
    for side in [-1,1]:
        for j in range(14):
            x=94+(j+.5)*98/14;z=(57-(x-94)*.017)if side==1 else(-58-(x-94)*.14)
            statue(m,x,grade(x,z)+18.25,z,3.10,lod,j)
        p=poly(869 if side==1 else 929)
        for j,a in enumerate(np.linspace(math.radians(145 if side==1 else 148),math.radians(37 if side==1 else 31),56)):
            direction=np.array([math.cos(a),side*math.sin(a)]);origin=np.array([275.,7.]);hits=[]
            for u,v in zip(p,np.roll(p,-1,axis=0)):
                matrix=np.column_stack([direction,u-v])
                if abs(np.linalg.det(matrix))<1e-8:continue
                t,k=np.linalg.solve(matrix,u-origin)
                if t>0 and 0<=k<=1:hits.append(t)
            if not hits:raise ValueError('saint ray misses mapped colonnade')
            x,z=origin+direction*(max(hits)-.2)
            statue(m,x,grade(x,z)+18.25,z,3.10,lod,j)


def fountain(m,x,z,lod):
    y=grade(x,z);n=(40,28,16)[lod]
    lathe(m,(x,z),[(y-.3,6),(y+.3,6),(y+.65,5.9)],TRIM,17,n)
    lathe(m,(x,z),[(y+.65,5.4),(y+.67,5.4)],(70,122,131),15,n)
    lathe(m,(x,z),[(y+.6,2.5),(y+1.2,2.5),(y+1.3,1.5),(y+4.4,1.2),(y+4.6,3.1),(y+5.1,3.4),(y+5.4,.65),(y+7.1,.65),(y+7.2,2.55),(y+7.5,2.5),(y+8,.16)],STONE,WEATHERED_LIMESTONE,n)
    if lod<2:
        for i in range(24):
            a=i*math.tau/24;points=[(x+(2.5+.7*t)*math.cos(a),y+7.4-2.4*t*t,z+(2.5+.7*t)*math.sin(a))for t in np.linspace(0,1,5)]
            for p,q in zip(points,points[1:]):tube(m,p,q,.045,(169,196,195),15,4)


def piazza(m,lod):
    # Convex decomposition of the deliberately authored paving is also its
    # lower-only DSM correction; it never expands into the surrounding streets.
    for p in PLAZA:
        paving=Mesh()
        plan_solid(paving,p,-.4,-.04,(119,122,118),82)
        paving.triangles=[(np.array([[x,y+grade(x,z),z]for x,y,z in points]),c,mat)
                          for points,c,mat in paving.triangles]
        m.merge(paving)
    x,z=center(1668);y=grade(x,z)
    m.box((x,y+2.6,z),(6.8,5.2,6.8),TRIM,17)
    m.box((x,y+5.55,z),(5.8,.7,5.8),TRIM,17)
    m.box((x,y+7.1,z),(4.1,2.8,4.1),TRIM,17)
    # Plain red-granite shaft, correctly without hieroglyphics.
    low=[(x-1.65,y+8.25,z-1.65),(x+1.65,y+8.25,z-1.65),(x+1.65,y+8.25,z+1.65),(x-1.65,y+8.25,z+1.65)]
    high=[(x-1.08,y+31.8,z-1.08),(x+1.08,y+31.8,z-1.08),(x+1.08,y+31.8,z+1.08),(x-1.08,y+31.8,z+1.08)]
    for i in range(4):
        face(m,[low[i],low[(i+1)%4],high[(i+1)%4],high[i]],(162,135,116),82)
        face(m,[high[i],high[(i+1)%4],(x,y+33.61,z)],(166,141,121),82)
    cross(m,x,y+33.61,z,2.3,1.45,lod)
    for dx in [-2.2,2.2]:
        for dz in [-2.2,2.2]:
            m.ellipsoid((x+dx,y+7.2,z+dz),(.8,.45,.45),(96,104,88),95,(10,8,6)[lod],(5,4,3)[lod])
    for a in np.arange(0,math.tau,math.tau/16):
        p=(x+10*math.cos(a),y+.01,z+10*math.sin(a));q=(x+50*math.cos(a),grade(x+50*math.cos(a),z+50*math.sin(a)),z+50*math.sin(a));tube(m,p,q,.13,(209,210,197),17,4)
    fountain(m,272,65,lod);fountain(m,278,-50,lod)
    for z in [-43,43]:statue(m,103,grade(103,z),z,9.0,lod,1)


# The interior of the oval and the trapezoid are paving, not a rectangular site pad.
PLAZA=[np.asarray([[99,-57],[192,-44],[208,-41],[210,56],[198,55],[99,57]],float)]
PLAZA.append(np.asarray([(275+88*math.cos(a),7+99*math.sin(a))for a in np.linspace(0,math.tau,81)[:-1]]))


def build(name,lod):
    if name!='st-peters-basilica' or lod not in (0,1,2):raise ValueError('unknown Vatican model or LOD')
    m=Mesh();basilica(m,lod);colonnades(m,lod);piazza(m,lod)
    m.triangles=[(p,c,WEATHERED_LIMESTONE if mat in (11,17)else mat)for p,c,mat in m.triangles]
    return m


def ownership():
    pp=[]
    threshold=np.asarray([[89.4,-57],[100.4,-57],[100.4,57],[89.4,57]])
    for p in BASILICA+[poly(i)for i in [451,1284,869,929]]+PLAZA+[threshold]:
        pp.extend(tri.tolist()for tri in plan_triangles(p))
    return pp


POLYGONS=ownership()
REFERENCES=[
 'https://www.vatican.va/content/vatican/en/ra/facciata-basilica.html',
 'https://www.vatican.va/content/vatican/en/ra/cupolone-lanterna.html',
 'https://www.vatican.va/content/vatican/en/ra/colonnato-statue-santi.html',
 'https://www.vatican.va/content/vatican/en/ra/orologio-alla-italiana.html',
 'https://www.vatican.va/content/vatican/en/ra/obelisco.html',
 'https://www.vatican.va/content/vatican/en/ra/fontana-destra.html',
 'https://www.vatican.va/content/vatican/en/ra/fontana-sinistra.html',
 'https://www.basilicasanpietro.va/en/san-pietro/the-square',
 'https://www.basilicasanpietro.va/en/san-pietro/the-dome',
 'https://www.openstreetmap.org/copyright']
ENTRIES=[{'name':'st-peters-basilica','city':'Vatican City','lat':SURVEY['lat'],'lon':SURVEY['lon'],
 'heading':0.,'base':BASE,'suppress_buildings':True,'suppress_roads':False,'suppress_trees':True,
 'source_exclusion_polygons':POLYGONS,'terrain_flatten':[],
 'terrain_cuts':[{'polygon_xz':p,'plane':([0,0,FLOOR-.2]if np.asarray(p)[:,0].max()<95 else[-.008,.001,2.3]),'feather_m':3}for p in POLYGONS],
 'references':REFERENCES,'description':'Surveyed Latin-cross basilica, apses, four subsidiary domes, sixteen-rib principal dome and lantern; Maderno facade with portals, loggias, giant order, clocks and thirteen statues; Bernini mapped open colonnades, trapezoid arms, 140 parapet saints, piazza paving, obelisk and fountains.',
 'attribution':'Plans © OpenStreetMap contributors, via Protomaps, ODbL 1.0. Original architectural geometry and sculpture silhouettes for Vearth.',
 'capture':{'distance':650,'azimuth':90,'elevation':26,'target_height':34,'time_utc':'2026-03-20T10:00:00Z'},
 'audit':{'survey_file':'tools/heritage_catalog/references/sacred_vatican/st-peters-basilica-survey.json',
 'dimensional_evidence':{'facade_width_m':116,'facade_height_m':45.5,'facade_column_m':[27.48,2.77],'pediment_m':[30,7.5],'dome_external_height_above_plaza_m':133.3,'dome_max_diameter_m':58.9,'columns_height_m':13,'colonnade_top_m':16.77,'colonnade_statues_height_m':3.1,'obelisk_shaft_m':25.36,'obelisk_base_m':8.25},
 'dimension_uncertainty':'Vatican RA explicitly gives 133.30m street-to-cross; model applies that above plaza datum without adding threshold rise. Rounded FAQ gives136m with no level basis. Roof tiers and individual sculpture poses are interpretations; 305 surveyed small support centres comprise shafts and piers, while primary total is 284 columns plus 88 piers. Source 61.38m center terrain is rejected as roof contamination. Threshold rise 3.3m and gentle plaza grade are estimates, not a conservation level survey.',
 'grounding':'Open plaza sample at [300,8] is 24.5035m; threshold 3.3m higher. Lower-only convex cuts correct demonstrable DSM roof contamination beneath modeled basilica, graded arms and paved plaza, preserving streets beyond authored paving.','orientation':'All mapped positions and east-facing facade rotation are baked in metric east/up/north.'}}]
