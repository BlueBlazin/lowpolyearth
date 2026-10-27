"""Four individually articulated American limestone and terracotta towers."""
from skyscraper_catalog.americas_common import *
from heritage_catalog.geometry import turn

SPEC={
 'tribune-tower':('Chicago',141.7,'Limestone Gothic shaft with recessed windows, octagonal crown, flying buttresses and pinnacle clusters above the retained wings.',['https://tribunetower.com/history/','https://www.skyscrapercenter.com/building/tribune-tower/9017','https://scb.com/project/tribune-tower-conversion/']),
 'wrigley-building':('Chicago',133.5,'Two glazed terracotta buildings with recessed bay grids, connecting bridges and the four-faced clock tower beneath its lantern.',['https://zeller.us/thewrigleybuilding/wp-content/uploads/sites/21/2021/09/Wrigley-BRO-09-2021-1.pdf','https://www.skyscrapercenter.com/chicago/wrigley-building/9018']),
 'woolworth-building':('New York City',241.4,'Gothic terracotta base and slender stepped shaft with tracery, corner pinnacles and a green copper pyramidal crown.',['https://www.skyscrapercenter.com/building/woolworth-building/969','https://www.nyc.gov/assets/lpc/downloads/pdf/presentation-materials/20250805/2-Park-Place.pdf']),
 '30-rockefeller-plaza':('New York City',259.08,'Indiana buff limestone Art Deco slab with precisely mapped setbacks, aluminum window spandrels, vertical piers and rooftop terraces.',['https://www.rockefellercenter.com/leasing/30-rockefeller-plaza','https://www.rockefellercenter.com/documents/TopoftheRockStudentGuide.pdf'])}
OWN={n:[ccw(p['polygon_xz'])for p in fit(n)['parts']]for n in SPEC}
ENTRIES=[measured_entry(n,*s,owned=OWN[n])for n,s in SPEC.items()];E={e['name']:e for e in ENTRIES}


def cornice(m,p,y,color=STONE,depth=.32,height=.55):
    c=np.mean(p,axis=0);q=c+(p-c)*(1+depth/max(1,np.linalg.norm(p-c,axis=1).mean()))
    plan_solid(m,q,y,y+height,color,WEATHERED_LIMESTONE)


def ribs(m,p,bottom,top,lod,spacing=3.5,color=STONE):
    p=ccw(p)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        v=b-a;width=np.linalg.norm(v);normal=np.array([v[1],-v[0]])/width*.15
        count=max(1,round(width/spacing))
        for k in range(count+1):
            q=a+v*k/count+normal;m.beam((q[0],bottom,q[1]),(q[0],top,q[1]),.15,color,WEATHERED_LIMESTONE,4)


def tribune(lod):
    n='tribune-tower';m=Mesh();e=E[n];shaft=part(n,1);wing=part(n,10);stone=(202,194,170)
    foundation(m,e,OWN[n],stone,top=.1)
    facade(m,wing,wing,0,20,4.0,3.0,lod,True,metal=stone,glass=(69,83,79),spandrel=1.0,mullion=.48)
    cornice(m,wing,19.8,stone)
    facade(m,shaft,shaft,20,111,3.65,3.1,lod,True,metal=stone,glass=(76,88,82),spandrel=.8,mullion=.60)
    ribs(m,shaft,9,114,lod,3.1,(210,201,179));cornice(m,shaft,109.8,stone,height=1.0)
    ang=np.arange(8)*math.tau/8+math.pi/8;outer=np.c_[np.cos(ang),np.sin(ang)]*14.8;inner=outer*.68
    facade(m,inner,inner,111,138,5.0,4,lod,True,metal=stone,glass=(52,62,58),spandrel=.6,mullion=.8)
    for i,(q,r)in enumerate(zip(outer,inner)):
        pin(m,q[0],q[1],110,24,1.15,(212,204,183),lod)
        m.beam((q[0],118,q[1]),(r[0],133,r[1]),.68,stone,WEATHERED_LIMESTONE,6)
        m.beam((q[0],123,q[1]),(r[0],136,r[1]),.39,stone,WEATHERED_LIMESTONE,6)
        pin(m,r[0],r[1],135,6.7,.7,(211,203,181),lod)
        if lod<2:
            for t in [.25,.5,.75]:
                v=q*(1-t)+r*t;pin(m,v[0],v[1],118+15*t,2,.2,stone,lod)
    roof(m,inner,138,(137,145,127));cornice(m,inner,137.7,stone,height=.6)
    lathe(m,(0,0),[(138,.22),(145.8,.05)],(157,153,130),SCULPTURE_METAL,6)
    return m


def clock_face(m,side,y,r,lod):
    # Clock geometry is authored in a vertical plane, then turned to each
    # facade. Hands and hour markers remain distinct from a painted square.
    s=Mesh();z=-6.32;center=np.array([0,y,z]);num=(48,32,24)[lod]
    points=[center+np.array([r*math.sin(k*math.tau/num),r*math.cos(k*math.tau/num),0])for k in range(num)]
    for a,b in zip(points,points[1:]+points[:1]):s.triangle(center,a,b,(219,218,198),PANEL_WALL)
    for k in range(12):
        a=k*math.tau/12;p=(r*.81*math.sin(a),y+r*.81*math.cos(a),z-.04);q=(r*.94*math.sin(a),y+r*.94*math.cos(a),z-.04);s.beam(p,q,.10,(53,59,53),SCULPTURE_METAL,4)
    for angle,length in [(math.pi/3,r*.65),(math.pi,r*.8)]:s.beam((0,y,z-.08),(length*math.sin(angle),y+length*math.cos(angle),z-.08),.12,(39,48,44),SCULPTURE_METAL,4)
    m.merge(turn(s,side*90))


def wrigley(lod):
    n='wrigley-building';m=Mesh();e=E[n];south=part(n,4);north=part(n,2);white=(222,218,200)
    foundation(m,e,[south,north],white,top=.1)
    for p,top in [(south,56),(north,83)]:
        # Six subtle terracotta shades rise toward the crown.
        for tier in range(6):
            color=tuple(min(255,c+tier*2)for c in white);y0=tier*top/6;y1=(tier+1)*top/6
            facade(m,p,p,y0,y1,3.8,3.0,lod,True,metal=color,glass=(74,94,91),spandrel=1.1,mullion=.55)
        for y in [8,top-5,top-.4]:cornice(m,p,y,white)
    # Both historic skybridges span the open pedestrian passage. Endpoints
    # are inside the measured building rings, keeping the passage open below.
    for y in [10.8,50.8]:
        a=np.array([-8.65356268,6.45421357]);b=np.array([-20.47291604,17.32332652])
        direction=(b-a)/np.linalg.norm(b-a);normal=np.array([direction[1],-direction[0]])
        a-=direction*2;b+=direction*2
        bridge=ccw([a-normal*2.5,b-normal*2.5,b+normal*2.5,a+normal*2.5])
        facade(m,bridge,bridge,y,y+3.6,3.6,2.2,lod,True,metal=white,glass=(81,107,100),spandrel=.6,mullion=.35)
    tower=rect(12,12.3)
    facade(m,tower,tower,56,109,4.1,3.0,lod,True,metal=(230,227,214),glass=(82,100,94),spandrel=.9,mullion=.65)
    cornice(m,tower,107.8,(230,227,214),depth=.7,height=1.2)
    plan_solid(m,tower,109,121,(231,229,214),WEATHERED_LIMESTONE)
    for side in range(4):clock_face(m,side,115,3.8,lod)
    cornice(m,tower,120.5,white,depth=.8,height=.8)
    lantern=rect(7.5,7.5);facade(m,lantern,lantern,121.3,128,6.7,2.5,lod,True,metal=white,glass=(54,72,65),spandrel=.6,mullion=.50)
    loft(m,lantern,rect(2.3,2.3),128,131.5,(122,143,123),SCULPTURE_METAL)
    pin(m,0,0,131.5,2,.4,(202,186,131),lod)
    return m


def woolworth(lod):
    n='woolworth-building';m=Mesh();e=E[n];stone=(209,204,183)
    foundation(m,e,OWN[n],stone,top=.1)
    base=part(n,53)
    facade(m,base,base,0,105,3.7,2.7,lod,True,metal=stone,glass=(72,98,93),spandrel=.9,mullion=.5)
    ribs(m,base,7,107,lod,2.7);cornice(m,base,104,stone,height=1.0)
    for i in [45,46,47,48]:
        p=part(n,i);facade(m,p,p,105,175,3.7,2.6,lod,True,metal=stone,glass=(76,98,91),spandrel=.75,mullion=.6)
        cornice(m,p,174.4,stone)
        for q in p[::2]:pin(m,q[0],q[1],175,5,.5,stone,lod)
    shaft=part(n,50);facade(m,shaft,shaft,105,194,3.8,2.5,lod,True,metal=stone,glass=(75,95,90),spandrel=.8,mullion=.65)
    ribs(m,shaft,105,200,lod,2.6);cornice(m,shaft,192.8,stone,height=1.1)
    cap=rect(17,17);facade(m,cap,cap,194,215,5.2,3.4,lod,True,metal=(220,214,194),glass=(45,67,62),spandrel=.6,mullion=.8)
    for q in rect(20,20):
        pin(m,q[0],q[1],194,24,.9,stone,lod)
        r=q*.82;m.beam((q[0],200,q[1]),(r[0],213,r[1]),.45,stone,WEATHERED_LIMESTONE,6)
    cornice(m,cap,214.5,stone,height=.7)
    copper=(94,141,119);loft(m,cap,rect(.3,.3),215,241.4,copper,SCULPTURE_METAL)
    for q in cap:
        m.beam((q[0],215,q[1]),(q[0]*.018,241.4,q[1]*.018),.16,(153,170,135),SCULPTURE_METAL,4)
    if lod<2:
        for y in np.arange(218,239,3):
            r=17*(241.4-y)/26.4
            for a,b in zip(rect(r,r),np.roll(rect(r,r),-1,axis=0)):m.beam((a[0],y,a[1]),(b[0],y,b[1]),.045,(119,154,122),SCULPTURE_METAL,4)
    return m


def rockefeller(lod):
    n='30-rockefeller-plaza';m=Mesh();e=E[n];stone=(207,202,184)
    foundation(m,e,OWN[n],stone,top=.1)
    layers=json.loads((REF/'30-rockefeller-plaza-envelope.json').read_text())['layers']
    for layer in layers:
        bottom,top=layer['bottom_m'],layer['top_m']
        for ring in layer['polygons_xz']:
            p=ccw(ring)
            facade(m,p,p,bottom,top,3.5052,2.55,lod,True,metal=stone,glass=(77,102,105),spandrel=1.05,mullion=.45)
            ribs(m,p,bottom,top,lod,2.55 if lod==0 else 5.1,(217,211,193))
    p=part(n,0)
    for a,b in zip(p,np.roll(p,-1,axis=0)):m.beam((a[0],259.08,a[1]),(b[0],259.08,b[1]),.1,(181,194,191),SCULPTURE_METAL,4)
    return m

BUILDERS={'tribune-tower':tribune,'wrigley-building':wrigley,'woolworth-building':woolworth,'30-rockefeller-plaza':rockefeller}
def build(name,lod):return BUILDERS[name](lod)
DEPENDENCIES=[Path(__file__).parent/'americas_common.py',Path(__file__).parent/'geometry.py',*REF.glob('*.json')]
