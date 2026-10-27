"""Seven American modern towers with their distinct structural envelopes."""
from skyscraper_catalog.americas_common import *

NAMES=['john-hancock-center','marina-city','aqua-tower','432-park-avenue','transamerica-pyramid','salesforce-tower','torre-reforma']
# A curved balcony edge is not the structural grid: Aqua follows its podium's
# straight north/south street edge, about one degree west of true north.
aqua_fit=fit('aqua-tower')

SPEC={
'john-hancock-center':('Chicago',343.7,'Tapered black aluminum tower with five tiers of exposed X bracing, mechanical crown and twin antennae.',['https://www.som.com/projects/875-north-michigan-avenue-formerly-john-hancock-center/','https://www.skyscrapercenter.com/building/john-hancock-center/345']),
'marina-city':('Chicago',179.2,'Twin concrete towers with open spiral parking, scalloped balconies, radial piers and central rooftop plant.',['https://www.skyscrapercenter.com/complex/155','https://www.architecture.org/online-resources/buildings-of-chicago/marina-city']),
'aqua-tower':('Chicago',261.8,'Blue glass residential slab wrapped in changing white concrete balcony contours over the mapped landscaped podium.',['https://studiogang.com/projects/aqua-tower/','https://www.studiogang.com/wp-content/uploads/reveal-aqua-tower-lr.pdf']),
'432-park-avenue':('New York City',425.5,'Slender exposed concrete grid with six square windows per face, open double-height wind floors and adjoining retail.',['https://vinoly.com/works/432-park-avenue/']),
'transamerica-pyramid':('San Francisco',260,'Quartz aggregate pyramid with open diagonal ground supports, narrow windows, projecting service wings and a long pyramidal spire.',['https://pcad.lib.washington.edu/building/2499/','https://www.fosterandpartners.com/projects/transamerica-pyramid-center']),
'salesforce-tower':('San Francisco',326,'Rounded tapering glass tower with a screened, curved crown.',['https://pcparch.com/work/salesforce-tower']),
'torre-reforma':('Mexico City',246,'Triangular open-book concrete tower with a sloping glazed facade and the preserved historic house at its base.',['https://www.lbrarquitectos.com/en/?proyectos=torre-reforma-3'])}

OWN={n:[ccw(p['polygon_xz'])for p in fit(n)['parts']]for n in NAMES}
# Marina's footprint includes the balcony overhang; each tower owns only its
# own circular contact area, leaving the public gap between towers untouched.
ENTRIES=[measured_entry(n,*SPEC[n],owned=OWN[n])for n in NAMES]
E={e['name']:e for e in ENTRIES}


def hancock(lod):
    n='john-hancock-center';m=Mesh();e=E[n];p=part(n,3);lo,hi=p.min(0),p.max(0);w,d=hi-lo;bottom=rect(w,d);top=rect(w*.59,d*.58)
    foundation(m,e,OWN[n],(69,68,61),top=.1)
    facade(m,bottom,top,0,337,3.37,1.9 if lod==0 else 2.8,lod,glass=(85,86,76),metal=(113,114,105),spandrel=.8,native_material=105)
    # Five 20-story structural Xs on every facade taper with the shell.
    def ring(y):return bottom+(top-bottom)*y/337
    for y0,y1 in zip([0,67.4,134.8,202.2,269.6],[67.4,134.8,202.2,269.6,337]):
        a,b=ring(y0),ring(y1)
        for i in range(4):
            j=(i+1)%4;v=a[j]-a[i];normal=np.array([v[1],-v[0]])/np.linalg.norm(v)*.30
            def pos(p,y):return(p[0]+normal[0],y,p[1]+normal[1])
            m.beam(pos(a[i],y0),pos(b[j],y1),.67,(95,98,91),PANEL_WALL,4)
            m.beam(pos(a[j],y0),pos(b[i],y1),.67,(95,98,91),PANEL_WALL,4)
            m.beam(pos(a[i],y0),pos(a[j],y0),.42,(103,106,99),PANEL_WALL,4)
    for k in range(4):m.beam((bottom[k,0],0,bottom[k,1]),(top[k,0],337,top[k,1]),.63,(99,103,97),PANEL_WALL,4)
    loft(m,top,top*.93,337,343.7,(64,64,54),SCULPTURE_METAL)
    for x,tip in [(-15.5,456.9),(15.5,456.9)]:
        lathe(m,(x,0),[(343.7,1.5),(347,1.3),(411,.65),(tip,.13)],(180,181,167),SCULPTURE_METAL,(12,10,6)[lod])
        for y in range(360,453,8):lathe(m,(x,0),[(y,.93),(y+.6,.93)],(125,126,117),SCULPTURE_METAL,8)
    for index in [21,23]:
        q=part(n,index);hi=next(v['height_m']for v in fit(n)['parts']if v['source_index']==index)
        facade(m,q,q,0,hi,4,3,lod,metal=(122,126,117),native_material=105)
    return m


def marina(lod):
    n='marina-city';m=Mesh();e=E[n];count=(96,64,48)[lod];angles=np.arange(count)*math.tau/count
    for source in [6,10]:
        p=part(n,source);c=(p.min(0)+p.max(0))/2
        def circle(r):return c+np.c_[np.cos(angles),np.sin(angles)]*np.asarray(r).reshape(-1,1)
        base=circle(np.full(count,18.5));foundation(m,e,[base],(170,167,150),top=.2)
        lathe(m,c,[(0,7.1),(174,7.1),(179.2,5.2)],(194,189,170),PANEL_WALL,(32,24,16)[lod])
        # Nineteen genuinely open parking levels; the helicoidal slab climbs
        # one floor over a full turn and is carried by sixteen radial columns.
        for k in range(19):
            y=2+k*2.62;r=18.5
            for i in range(count):
                j=(i+1)%count;a=angles[i];b=(i+1)*math.tau/count
                yy=y+2.62*i/count;zz=y+2.62*(i+1)/count
                pts=[(c[0]+q*math.cos(t),h,c[1]+q*math.sin(t))for q,t,h in [(7.2,a,yy),(r,a,yy),(r,b,zz),(7.2,b,zz)]]
                face(m,pts,(192,187,167),PANEL_WALL)
                face(m,[(pts[1][0],yy-.30,pts[1][2]),(pts[2][0],zz-.30,pts[2][2]),pts[2],pts[1]],(184,180,161),PANEL_WALL,(math.cos(a),0,math.sin(a)))
        for k in range(16):
            a=k*math.tau/16;x,z=c+14*np.array([math.cos(a),math.sin(a)])
            m.beam((x,-.3,z),(x,169,z),.39,(193,189,173),PANEL_WALL,6)
        for level in range(40):
            y=53+level*2.9
            # Broad scallops reproduce the petal balconies, not a stack of
            # round discs. The glazing sits well behind their concrete edge.
            radius=17.9+1.8*np.cos(16*angles)
            poly=circle(radius);plan_solid(m,poly,y,y+.30,(204,199,180),PANEL_WALL)
            if lod<2 or level%2==0:
                inner=circle(np.full(count,14.3));loft(m,inner,inner,y+.31,y+2.72,(99,126,124),WINDOW,False)
                if lod==0:
                    rail=circle(radius-.10)[::2]
                    for a,b in zip(rail,np.roll(rail,-1,axis=0)):
                        m.beam((a[0],y+1.22,a[1]),(b[0],y+1.22,b[1]),.045,(85,89,83),SCULPTURE_METAL,4)
        roof(m,circle(np.full(count,16.6)),169.4,(168,169,153))
    return m


def aqua(lod):
    n='aqua-tower';m=Mesh();e=E[n];pod=part(n,7);foundation(m,e,[pod],(165,171,163),top=.1)
    facade(m,pod,pod,0,12.1,4.0,2.8,lod,glass=(82,120,127),metal=(172,179,177))
    roof(m,pod,12.12,(153,159,147))
    # Structural glass envelope follows the site grid; balcony variation is
    # continuous vertically, with the architect's maximum 3.66m cantilever.
    h=math.radians(-9.4);rot=np.array([[math.cos(h),-math.sin(h)],[math.sin(h),math.cos(h)]])
    core=rect(49.5,24.0)@rot.T
    facade(m,core,core,12.1,261.8,3.05,2.35,lod,glass=(75,115,137),metal=(116,143,148),spandrel=.32)
    per=(20,14,10)[lod];outline=[]
    for a,b in zip(rect(49.5,24),np.roll(rect(49.5,24),-1,axis=0)):
        for t in np.arange(per)/per:outline.append(a+(b-a)*t)
    outline=np.array(outline);normal=outline/np.maximum(1,np.linalg.norm(outline,axis=1))[:,None]
    for level in range(80):
        y=14+level*3.05
        if y>260:break
        t=np.arange(len(outline))*math.tau/len(outline)
        ext=np.clip(1.45+1.1*np.sin(t*3+level*.14)+.9*np.cos(t*5-level*.09),.25,3.66)
        p=(outline+normal*ext[:,None])@rot.T
        plan_solid(m,p,y,y+.26,(211,215,208),PANEL_WALL)
        if lod==0:
            for a,b in zip(p,np.roll(p,-1,axis=0)):
                m.beam((a[0],y+1.0,a[1]),(b[0],y+1.0,b[1]),.035,(101,125,127),SCULPTURE_METAL,4)
    # Small planted roof beds stay inside the south portion of the mapped roof.
    for x,z,w,d in [(20,-40,22,15),(50,-25,17,13)]:
        p=rect(w,d,x,z);plan_solid(m,p,12.14,12.55,(169,174,155),PANEL_WALL);roof(m,p*.98+np.array([x,z])*.02,12.56,(91,123,63),3)
    return m


def park432(lod):
    n='432-park-avenue';m=Mesh();e=E[n];tower=part(n,1);p=rect(28.5,28.5)
    foundation(m,e,[tower],(210,210,197),top=.1)
    floor=425.5/96;white=(214,214,202);glass=(96,131,143)
    # Open mechanical levels expose a recessed core through the concrete
    # basket. No dark opaque sleeve fills these wind passages.
    openings={*range(12,14),*range(26,28),*range(40,42),*range(54,56),*range(68,70),*range(82,84)}
    for level in range(96):
        y=level*floor
        if level in openings:
            plan_solid(m,rect(9.1,9.1),y,y+floor,(160,164,160),PANEL_WALL)
            for a,b in zip(p,np.roll(p,-1,axis=0)):
                for k in range(7):
                    q=a+(b-a)*k/6;m.box((q[0],y+floor/2,q[1]),(.82,floor,.82),white,PANEL_WALL)
            plan_solid(m,p,y,y+.48,white,PANEL_WALL)
        else:
            facade(m,p,p,y,y+floor,floor,28.5/6,lod,stone=True,glass=glass,metal=white,spandrel=1.2,mullion=.8,wall_material=PANEL_WALL,native_material=110,explicit_grid=True)
    # The northern retail component occupies the residual source parcel.
    retail=rect(38,14,-4.5,21.5)
    foundation(m,e,[retail],(183,187,178),top=.1)
    facade(m,retail,retail,0,15,5,3,lod,glass=(116,149,152),metal=(178,188,183),spandrel=.3)
    return m


def transamerica(lod):
    n='transamerica-pyramid';m=Mesh();e=E[n];p=part(n,0);foundation(m,e,[p],(211,209,194),top=.15)
    shell0=rect(53,53);shell1=rect(5.6,5.6);stone=(211,211,197)
    # Open ground colonnade bears the sloping facade at its four faces.
    facade(m,rect(31,31),rect(31,31),0,15,5,3,lod,glass=(92,127,129),metal=(141,149,143))
    for a,b in zip(shell0,np.roll(shell0,-1,axis=0)):
        for k in range(5):
            q=a+(b-a)*(k+.5)/5;r=a+(b-a)*k/5;s=a+(b-a)*(k+1)/5
            m.beam((q[0],-.5,q[1]),(r[0],17,r[1]),.8,stone,PANEL_WALL,4);m.beam((q[0],-.5,q[1]),(s[0],17,s[1]),.8,stone,PANEL_WALL,4)
    facade(m,shell0,shell1,17,212,4.12,3.0,lod,stone=True,glass=(79,99,99),metal=stone,spandrel=1.0,mullion=.62,wall_material=PANEL_WALL)
    loft(m,shell1,rect(.15,.15),212,260,stone,PANEL_WALL)
    # Two narrow service wings extend beyond opposite upper faces.
    for sign in [-1,1]:
        side=rect(12,10,sign*6,0);loft(m,side,side,105,205,stone,PANEL_WALL)
        loft(m,side,rect(8,10,sign*4,0),205,215,stone,PANEL_WALL)
    return m


def salesforce(lod):
    n='salesforce-tower';m=Mesh();e=E[n];p=part(n,0);foundation(m,e,[p],(155,164,159),top=.05)
    # The mapped rounded-square plan tapers before the screened crown.
    shell=p if lod==0 else p[::2]
    count=len(shell)
    def ring(y):return shell*(1-.20*(min(y,285)/285)**1.7)
    levels=[0,45,90,135,180,225,260,283.65]
    for y,top in zip(levels,levels[1:]):
        a,b=ring(y),ring(top)
        loft(m,a,b,y,top,(132,153,160),104,False)
    roof(m,ring(283.65),283.65,(135,147,143))
    levels=np.linspace(283.65,326,(13,10,7)[lod])
    def crown(y):
        shrink=max(.015,math.cos((y-283.65)/(326-283.65)*math.pi/2))**.42
        return ring(283.65)*shrink
    # A continuous native finish represents the fine perforated screen at
    # game distances without subpixel rods producing dark flickering marks.
    for a,b in zip(levels,levels[1:]):
        loft(m,crown(a),crown(b),a,b,(142,160,165),104,False)
    roof(m,crown(326),326,(142,160,165),PANEL_WALL)
    return m


def reforma(lod):
    n='torre-reforma';m=Mesh();e=E[n];owned=OWN[n];foundation(m,e,owned,(155,160,152),top=.1)
    control=json.loads((REF/'torre-reforma-envelope.json').read_text())
    lower,upper=np.array(control['bottom_xz']),np.array(control['top_xz'])
    for i in range(len(lower)):
        j=(i+1)%len(lower);a,b=lower[i],lower[j];aa,bb=upper[i],upper[j]
        v=bb-aa;normal=np.array([v[1],-v[0]])
        glazed=normal[0]<0 and normal[1]<0
        face(m,[(a[0],0,a[1]),(b[0],0,b[1]),(bb[0],246,bb[1]),(aa[0],246,aa[1])],
             (129,150,154)if glazed else (166,170,160),103 if glazed else PANEL_WALL,(normal[0],0,normal[1]))
    roof(m,upper,246,(133,145,138))
    house=part(n,19);facade(m,house,house,0,8,4,2.5,lod,stone=True,metal=(189,177,151),glass=(58,81,77),spandrel=1.0,mullion=.45)
    roof(m,house,8.1,(163,157,137))
    for q in house[::2]:pin(m,q[0],q[1],8.1,1.2,.25,(205,195,169),lod)
    return m

BUILDERS=dict(zip(NAMES,[hancock,marina,aqua,park432,transamerica,salesforce,reforma]))
def build(name,lod):return BUILDERS[name](lod)
DEPENDENCIES=[Path(__file__).parent/'americas_common.py',Path(__file__).parent/'geometry.py',*REF.glob('*.json')]
