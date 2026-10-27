"""Eight separately authored Japanese and Southeast Asian tower compositions."""
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.pacific_geometry import *

GROUP='japan_southeast_asia'
NAMES=('abeno-harukas','tokyo-metropolitan-government-building','mode-gakuen-cocoon-tower','umeda-sky-building','mahanakhon','baiyoke-tower-ii','the-exchange-106','capitaspring')
SITES={n:site(GROUP,n)for n in NAMES}
BASES={'abeno-harukas':26.8,'tokyo-metropolitan-government-building':39.0,'mode-gakuen-cocoon-tower':38.3,'umeda-sky-building':4.0,'mahanakhon':7.2,'baiyoke-tower-ii':6.4,'the-exchange-106':43.0,'capitaspring':18.1}


def _tokyo(lod):
    name=NAMES[1];d=SITES[name];m=Mesh();base=BASES[name]
    stone=(177,173,164);glass=(100,120,129)
    # These overlapping mapped parts are vertical tiers, not separate 243m prisms.
    tiers=[(89877471,0,133),(568387704,133,176),(568387705,176,207),(568387706,207,243.4),
           (568387707,133,176),(568387708,176,207),(568387709,207,243.4),
           (89877420,0,73),(568465171,73,115),(568465175,73,138),(568465180,73,163)]
    for id,lo,hi in tiers:
        p=part(d,id)
        if lo==0:support(m,d,p,base)
        envelope(m,p,lo,hi,lod,glass,stone,material=106)
        # Projecting stone corners express Tange's articulated octagonal towers.
        if lod<2:ribs(m,p,lo,hi,8,.40,(183,181,175),lod)
        if hi>170:slab(m,p,hi,.55,stone)
    # The Assembly ring wraps an open civic court; its concave plan is retained.
    p=part(d,89878322);ground=halfplane(p,[1,-.2],55)
    support(m,d,ground,base);envelope(m,ground,0,15,lod,glass,stone,material=106)
    envelope(m,p,15,25,lod,glass,stone,material=106)
    for id,lo,hi in [(245962672,25,41),(245962673,15,25),(245962674,15,25),(245962675,15,25),(572510171,15,25),(572510172,15,25)]:
        p=part(d,id);envelope(m,p,lo,hi,lod,glass,stone,material=106)
        for tri in plan_triangles(p):face(m,[(x,lo,z)for x,z in tri],stone,PANEL_WALL,(0,-1,0))
    for id in (568387706,568387709):
        c=part(d,id).mean(0);lathe(m,c,[(243.4,.65),(246.2,.28)],(143,148,145),SCULPTURE_METAL,8)
    return m


def _pixel_plan(p,tolerance):
    p=list(clean(p))
    while len(p)>3:
        for i,b in enumerate(p):
            a,c=p[i-1],p[(i+1)%len(p)];span=c-a
            if np.dot(b-a,c-b)<0 or np.linalg.norm(span)<1e-8:continue
            distance=abs((b-a)[0]*span[1]-(b-a)[1]*span[0])/np.linalg.norm(span)
            if distance<tolerance:
                p.pop(i);break
        else:break
    return clean(p)


def _mahanakhon(lod):
    d=SITES['mahanakhon'];m=Mesh();base=BASES[d['name']]
    def inside(point,polygon):
        x,z=point;hit=False
        for a,b in zip(polygon,np.roll(polygon,-1,axis=0)):
            if (a[1]>z)!=(b[1]>z) and x<(b[0]-a[0])*(z-a[1])/(b[1]-a[1])+a[0]:hit=not hit
        return hit
    volumes=[(float(w['tags'].get('min_height',0)),float(w['tags']['height']),clean(w['polygon_xz_m']))for w in d['osm_parts']]
    for w in d['osm_parts']:
        tags=w['tags'];lo=float(tags.get('min_height',0));hi=float(tags['height']);p=clean(w['polygon_xz_m'])
        if hi<=lo:continue
        if lo>0:p=_pixel_plan(p,(.001,.06,.35)[lod])
        if lo==0:support(m,d,p,base)
        envelope(m,p,lo,hi,lod,(104,135,148),(142,153,154),material=103)
        # Each mapped pixel setback retains a structural slab and actual terrace.
        # The envelope roof already forms this terrace slab; no coplanar overlay.
        if hi<305 and lod==0:
            above=[ring for low,high,ring in volumes if low<=hi+.45<high]
            for a,b in zip(p,np.roll(p,-1,axis=0)):
                length=np.linalg.norm(b-a)
                if not 4<length<12:continue
                tangent=(b-a)/length;inward=np.array([-tangent[1],tangent[0]])
                # Repeated floor edges are usually covered by the next storey.
                # Rails belong only to an exposed roof terrace, never to glass.
                checks=[a+(b-a)*t+inward*.2 for t in (.05,.25,.5,.75,.95)]
                if any(inside(q,ring)for q in checks for ring in above):continue
                m.beam((a[0],hi+.9,a[1]),(b[0],hi+.9,b[1]),.055,(164,173,171),SCULPTURE_METAL,4)
                for t in np.linspace(0,1,max(2,math.ceil(length/3)+1)):
                    q=a+(b-a)*t
                    m.beam((q[0],hi-.05,q[1]),(q[0],hi+.9,q[1]),.045,(164,173,171),SCULPTURE_METAL,4)
    top=part(d,444651187);c=top.mean(0)
    # The roof viewing platform follows the highest mapped plan, with a glass edge.
    for a,b in zip(top,np.roll(top,-1,axis=0)):
        face(m,[(a[0],313,a[1]),(b[0],313,b[1]),(b[0],314.2,b[1]),(a[0],314.2,a[1])],(151,174,177),WINDOW,(b[1]-a[1],0,a[0]-b[0]))
    return m


def _baiyoke(lod):
    d=SITES['baiyoke-tower-ii'];m=Mesh();base=BASES[d['name']]
    podium=production_part(d,3206850546398957023);support(m,d,podium,base)
    envelope(m,podium,0,60,lod,(118,133,137),(173,168,149),material=110)
    for y,col in [(8,(169,163,148)),(22,(152,161,156)),(40,(174,168,149)),(59.6,(181,176,159))]:slab(m,podium,y,.6,col)
    p=part(d,443491486);envelope(m,p,60,284,lod,(104,125,137),(181,178,161),material=108)
    ribs(m,p,60,283,7,.48,(181,180,164),lod)
    for w in d['osm_parts']:
        if w['id']not in (443491485,443491486,443491487):
            p=clean(w['polygon_xz_m']);hi=float(w['tags']['height'])
            envelope(m,p,60,hi,lod,(118,135,137),(177,173,154),material=110)
    for id,lo,hi in [(443491485,284,295),(443491487,295,304)]:
        p=part(d,id);envelope(m,p,lo,hi,lod,(114,137,142),(168,176,173),material=107)
        for y in np.arange(lo,hi,3):slab(m,p,y,.55,(192,190,174))
    c=part(d,443491487).mean(0)
    lathe(m,c,[(304,2.2),(307,1.25),(312,1),(328.4,.18)],(189,179,166),SCULPTURE_METAL,(12,10,8)[lod])
    for y in np.arange(308,327,4):lathe(m,c,[(y,.8),(y+1.8,.75)],(158,98,88),SCULPTURE_METAL,8)
    return m


def _umeda(lod):
    d=SITES['umeda-sky-building'];m=Mesh();base=BASES[d['name']];silver=(165,170,169);glass=(137,157,165)
    # The thin central pieces are bridge deck only; never fill the space below.
    for id,hi in [(252708903,155.5),(252708901,160),(252485324,164.2),(252708902,160),(252708904,155.5)]:
        p=part(d,id);support(m,d,p,base);envelope(m,p,0,hi,lod,glass,silver,material=104)
        if lod<2:ribs(m,p,0,hi,7,.32,(168,175,173),lod)
    for id in (588689730,649098944):
        p=part(d,id);envelope(m,p,164.2,173,lod,glass,silver,material=104)
        for tri in plan_triangles(p):face(m,[(x,164.2,z)for x,z in tri],silver,SCULPTURE_METAL,(0,-1,0))
    # West inner support strip completes the forty-storey tower below the bridge.
    p=rect(-10,1,10,55,-3.7);support(m,d,p,base);envelope(m,p,0,164.2,lod,glass,silver,material=104)
    # Glass elevator cylinders are external, terminating at the transfer floor.
    for x,z in [(-9,-28),(39,-31)]:
        lathe(m,(x,z),[(0,2.7),(143,2.7),(146,2.1)],(143,166,171),104,(20,14,10)[lod])
        if lod<2:
            for y in np.arange(3,145,4 if lod==0 else 8):lathe(m,(x,z),[(y,2.77),(y+.15,2.77)],(164,176,175),SCULPTURE_METAL,16)
    # Two 45m enclosed escalators rise from floor35 toward opposite rim points.
    for a,b in [((-8,145,-24),(29,165,-10)),((39,145,-27),(2,165,-12))]:
        m.beam(a,b,1.65,(155,176,180),WINDOW,(12,10,8)[lod])
        m.beam((a[0],a[1]-1.1,a[2]),(b[0],b[1]-1.1,b[2]),.5,(147,157,157),SCULPTURE_METAL,6)
        if lod==0:
            for t in np.linspace(0,1,20):
                q=np.array(a)*(1-t)+np.array(b)*t;m.beam(q+[-1.8,0,0],q+[1.8,0,0],.06,(176,184,181),SCULPTURE_METAL,4)
    return m


def _abeno(lod):
    d=SITES['abeno-harukas'];m=Mesh();base=BASES[d['name']]
    whole=part(d,187296989);support(m,d,whole,base)
    envelope(m,whole,0,29,lod,(122,145,153),(162,168,165),material=107)
    low=part(d,488467176);envelope(m,low,29,83,lod,(137,157,165),(165,170,169),material=104)
    middle=rect(-2,-9,75,61,-16.8);upper=rect(-9,-1,61,41,-16.8)
    envelope(m,middle,87,187,lod,(123,149,158),(153,164,165),material=104)
    envelope(m,upper,191,287,lod,(121,147,158),(160,170,170),material=104)
    # The three-storey observatory surrounds an open-air central garden.
    c=upper.mean(0);r=(upper-c)*.72+c
    for i in range(4):
        q=clean([upper[i],upper[(i+1)%4],r[(i+1)%4],r[i]])
        envelope(m,q,287,300,lod,(145,166,173),(162,174,175),material=104)
    roof(m,r,288,(151,148,132))
    for y,p in [(83,low),(187,middle),(287,upper)]:
        if lod<2:
            q=p.mean(0)
            for i,a in enumerate(p):
                v=a*.9+q*.1;vegetation(m,*[v[0],y+.75,v[1]],1.3,.75,1.3,lod,i)
    # Double-height transfer/mechanical bands are quieter than the glazed rooms.
    for p,y in [(middle,83),(upper,187)]:
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            face(m,[(a[0],y,a[1]),(b[0],y,b[1]),(b[0],y+4,b[1]),(a[0],y+4,a[1])],(122,137,139),PANEL_WALL,(b[1]-a[1],0,a[0]-b[0]))
    return m


def _cocoon(lod):
    d=SITES['mode-gakuen-cocoon-tower'];m=Mesh();base=BASES[d['name']]
    footprint=part(d,145495557);support(m,d,footprint,base)
    center=np.array([3.9,10.4]);angle=math.radians(3.2)
    rotation=np.array([[math.cos(angle),-math.sin(angle)],[math.sin(angle),math.cos(angle)]])
    profile=np.array([(0,.74),(20,.91),(70,1),(130,.93),(170,.77),(198,.53),(204,.36)])
    def point(y,t,relief=0):
        scale=np.interp(y,profile[:,0],profile[:,1]);q=np.array([(27*scale+relief)*math.cos(t),(24*scale+relief)*math.sin(t)])@rotation.T+center
        return np.array([q[0],y,q[1]])
    segments=(48,32,20)[lod];ys=np.unique(np.r_[np.linspace(0,204,(35,23,15)[lod]),profile[:,0]])
    for y0,y1 in zip(ys[:-1],ys[1:]):
        for i in range(segments):
            t0=2*math.pi*i/segments;t1=2*math.pi*(i+1)/segments
            tint=(130,150,157)if abs(math.sin((t0+.25)*1.5))>.13 else(117,134,141)
            face(m,[point(y0,t0),point(y0,t1),point(y1,t1),point(y1,t0)],tint,104,(math.cos(t0),0,math.sin(t0)))
    for y,normal in [(0,(0,-1,0)),(204,(0,1,0))]:face(m,[point(y,2*math.pi*i/segments)for i in range(segments)],(160,173,176),PANEL_WALL,normal)
    # Fine diagonal spandrel ribs follow the curved shell rather than straight
    # lines crossing through the building; three structural seams remain visible.
    for sign in (-1,1):
        for t in np.linspace(0,2*math.pi,(20,14,10)[lod],endpoint=False):
            points=[point(y,t+sign*y/43,.11)for y in np.linspace(0,204,(39,27,17)[lod])]
            for a,b in zip(points[:-1],points[1:]):m.beam(a,b,.105,(185,190,188),PANEL_WALL,4)
    for t in [0,2*math.pi/3,4*math.pi/3]:
        points=[point(y,t,.2)for y in ys]
        for a,b in zip(points[:-1],points[1:]):m.beam(a,b,.32,(189,193,190),PANEL_WALL,6)
    return m


def _exchange(lod):
    d=SITES['the-exchange-106'];m=Mesh();base=BASES[d['name']];p=part(d,503451214);c=p.mean(0)
    support(m,d,p,base);envelope(m,p,0,405.4,lod,(116,143,155),(151,163,168),material=104)
    ribs(m,p,0,405.4,8,.26,(169,182,181),lod)
    # A 48.2m luminous faceted crown rises above the highest occupied floor.
    profiles=[(405.4,1),(421.4,1),(436,.87),(447,.68),(453.6,.58)]
    for (lo,s0),(hi,s1) in zip(profiles[:-1],profiles[1:]):
        a=(p-c)*s0+c;b=(p-c)*s1+c
        for i in range(len(p)):
            j=(i+1)%len(p);e=a[j]-a[i]
            face(m,[(a[i,0],lo,a[i,1]),(a[j,0],lo,a[j,1]),(b[j,0],hi,b[j,1]),(b[i,0],hi,b[i,1])],(159,177,178),104,(e[1],.2,-e[0]))
            if lod<2:
                count=max(1,round(np.linalg.norm(e)/(8 if lod==0 else 14)))
                for k in range(count):
                    u=k/count;v=(k+1)/count;q0=a[i]*(1-u)+a[j]*u;q1=b[i]*(1-v)+b[j]*v
                    m.beam((q0[0],lo,q0[1]),(q1[0],hi,q1[1]),.12,(183,193,187),PANEL_WALL,4)
                    q0=a[i]*(1-v)+a[j]*v;q1=b[i]*(1-u)+b[j]*u
                    m.beam((q0[0],lo,q0[1]),(q1[0],hi,q1[1]),.12,(183,193,187),PANEL_WALL,4)
        roof(m,b,hi)
    gallery=part(d,1115404234);support(m,d,gallery,base);envelope(m,gallery,0,13,lod,(139,159,161),(184,181,166),material=106)
    return m


def _capitaspring(lod):
    d=SITES['capitaspring'];m=Mesh();base=BASES[d['name']];p=production_part(d,18069680006654502533);c=p.mean(0)
    support(m,d,p,base)
    core=(p-c)*.42+c
    envelope(m,core,0,277,lod,(112,133,140),(152,163,165),material=104)
    envelope(m,p,18,100,lod,(134,153,162),(165,170,169),material=104)
    envelope(m,p,135,274,lod,(137,157,165),(165,170,169),material=104)
    # Four open-air garden decks span the owner's verified 100–135m interval.
    for k,y in enumerate([100,111,122,134,277]):
        q=(p-c)*(.93 if y<200 else .97)+c;slab(m,q,y,.5,(173,178,168))
        for i,a in enumerate(q):
            b=q[(i+1)%len(q)];length=np.linalg.norm(b-a);count=max(2,int(length/(6 if lod==0 else 10)))
            for j in range(count):
                v=a+(b-a)*(j+.5)/count;v=v*.92+c*.08
                vegetation(m,v[0],y+1.5+(j%3)*.25,v[1],1.7,1.6,1.7,lod,j+k)
        if y<200 and lod<2:
            # A sloping inner promenade links the planted terraces.
            a=q[k%len(q)]*.75+c*.25;b=q[(k+1)%len(q)]*.75+c*.25
            m.beam((a[0],y,a[1]),(b[0],min(y+11,134),b[1]),.65,(174,173,156),PANEL_WALL,4)
    # Pinstripe fins split and bow at the city room and the mid-air garden.
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        edge=b-a;length=np.linalg.norm(edge);normal=np.array([edge[1],-edge[0]])/length
        count=max(2,round(length/(4 if lod==0 else 7 if lod==1 else 12)))
        for i in range(count+1):
            u=i/count;q=a+edge*u
            for lo,hi in [(0,18),(100,135),(274,280)]:
                samples=(10,7,4)[lod];points=[]
                for t in np.linspace(0,1,samples):
                    shift=math.sin(math.pi*t)*(5 if hi<200 else 1.3)
                    v=q+normal*shift+edge/length*(u-.5)*shift*.8;points.append((v[0],lo+(hi-lo)*t,v[1]))
                for p0,p1 in zip(points[:-1],points[1:]):m.beam(p0,p1,.18,(185,190,184),PANEL_WALL,4)
            if lod<2:
                for lo,hi in [(18,100),(135,274)]:
                    tangent=edge/length
                    fin=[q-tangent*.12-normal*.08,q+tangent*.12-normal*.08,q+tangent*.12+normal*.45,q-tangent*.12+normal*.45]
                    plan_solid(m,fin,lo,hi,(182,187,182),PANEL_WALL)
    return m


BUILDERS=dict(zip(NAMES,(_abeno,_tokyo,_cocoon,_umeda,_mahanakhon,_baiyoke,_exchange,_capitaspring)))

def _ownership(name):
    d=SITES[name]
    if name=='abeno-harukas':return [part(d,187296989)]
    if name=='tokyo-metropolitan-government-building':return [part(d,89877471),part(d,89877420),halfplane(part(d,89878322),[1,-.2],55)]
    if name=='mode-gakuen-cocoon-tower':return [part(d,145495557)]
    if name=='umeda-sky-building':return [part(d,id)for id in (252708903,252708901,252485324,252708902,252708904)]+[rect(-10,1,10,55,-3.7)]
    if name=='mahanakhon':return [clean(w['polygon_xz_m'])for w in d['osm_parts']if not w['tags'].get('min_height')]
    if name=='baiyoke-tower-ii':return [production_part(d,3206850546398957023)]
    if name=='the-exchange-106':return [part(d,503451214),part(d,1115404234)]
    if name=='capitaspring':return [production_part(d,18069680006654502533)]
    raise KeyError(name)

ENTRIES=[]
REF=Path(__file__).parent/'references'/GROUP
for name in NAMES:
    a=json.loads((REF/(name+'-architecture.json')).read_text())
    ENTRIES.append(make_entry(SITES[name],a['city'],BASES[name],_ownership(name),a['architectural_height_m'],a['primary_evidence'],a['references'],{
        'estimates':a['estimates'],'survey_file':f'tools/skyscraper_catalog/references/{GROUP}/{name}-site.json',
        'architecture_file':f'tools/skyscraper_catalog/references/{GROUP}/{name}-architecture.json',
        'contact_camera':a['contact_camera'],'ground_method':'Fixed approach datum, bounded lower-only cuts inside authored structure, perimeter supports sampled at every authored toe and buried 0.5m. No roof-driven datum; final engine contact review required.'}))

# Open circulation areas use bounded grades independently of the fixed tower datum.
for entry in ENTRIES:
    if entry['name']=='abeno-harukas':
        setting=json.loads((REF/'abeno-setting.json').read_text())
        entry['terrain_grades']=[{'polygon_xz':setting['grade_polygon_xz'],'plane':setting['plane_relative_to_floor'],'feather_m':setting['feather_m']}]
    elif entry['name']=='baiyoke-tower-ii':
        setting=json.loads((REF/'baiyoke-setting.json').read_text())
        entry['terrain_grades']=[{'polygon_xz':setting['grade_polygon_xz'],'plane':setting['plane_relative_to_floor'],'feather_m':setting['feather_m']}]
    elif entry['name']=='mode-gakuen-cocoon-tower':
        setting=json.loads((REF/'cocoon-setting.json').read_text())
        entry['terrain_grades']=[{'polygon_xz':setting['grade_polygon_xz'],'plane':setting['plane_relative_to_floor'],'feather_m':setting['feather_m']}]
    elif entry['name']=='capitaspring':
        entry['max_source_building_span_m']=json.loads((REF/'capitaspring-source-replacement.json').read_text())['max_source_building_span_m']
        setting=json.loads((REF/'capitaspring-setting.json').read_text())
        entry['terrain_grades']=[{'polygon_xz':setting['grade_polygon_xz'],'plane':setting['plane_relative_to_floor'],'feather_m':setting['feather_m']}]
    elif entry['name']=='tokyo-metropolitan-government-building':
        setting=json.loads((REF/'tokyo-setting.json').read_text())
        entry['max_source_building_span_m']=setting['max_source_building_span_m']
        entry['terrain_grades']=[{'polygon_xz':setting[key+'_polygon_xz'],'plane':setting['plane_relative_to_floor'],'feather_m':setting[key+'_feather_m']}for key in ('plaza','street','forecourt')]

DEPENDENCIES=[Path(__file__).parent/'pacific_geometry.py',Path(__file__).parent/'geometry.py',Path(__file__).parent/'inventory.py',
              Path(__file__).resolve().parents[2]/'crates/vearth-gen/src/highrise.rs',*REF.glob('*.json')]

def build(name,lod):
    if lod not in (0,1,2):raise ValueError('LOD must be 0, 1 or 2')
    return BUILDERS[name](lod)
