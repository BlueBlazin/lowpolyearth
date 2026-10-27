"""Six Middle Eastern tower complexes; measured plans and original facades."""
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.regional import *

GROUP='middle_east'
BASES={'kingdom-centre':633.9,'al-faisaliah-centre':621.0,'bahrain-world-trade-center':3.6,
       'al-hamra-firdous-tower':6.7,'cayan-tower':-2.7,'emirates-towers':11.6}

def foundation(m,name,color=(159,157,141)):
    footings(m,GROUP,name,BASES[name],color)


def _kingdom(lod):
    name='kingdom-centre';m=Mesh();m.facade_material=104;foundation(m,name,(170,169,154))
    p=ccw(main_parts(GROUP,name)[0]['outer'])
    vertical(m,p,.2,205,lod,(110,162,178),(171,195,204),4.3,2,.65)
    # The curved opening passes through the crown: no opaque painted void.
    xs=np.linspace(-36.5,36.5,(48,32,20)[lod]);depth=8.6
    def sill(x):return 211+78*(abs(x)/36.5)**2.15
    for a,b in zip(xs,xs[1:]):
        for sign in [-1,1]:
            pts=[(a,205,sign*depth),(b,205,sign*depth),(b,sill(b),sign*depth),(a,sill(a),sign*depth)]
            face(m,pts,(148,162,167),104,(0,0,sign))
        face(m,[(a,sill(a),-depth),(b,sill(b),-depth),(b,sill(b),depth),(a,sill(a),depth)],(205,217,215),95,(0,1,0))
        if lod<2:
            for sign in [-1,1]:m.beam((a,sill(a),sign*depth),(b,sill(b),sign*depth),.30,(228,230,214),95,6)
    for q in parts(GROUP,name):
        if q['seed']in ['10815289335792689903','17861318586900343671']:
            vertical(m,ccw(q['outer']),205,302,lod,(143,182,190),(208,216,207),4.2,1.6,.35)
    # The skybridge follows the mapped bowed front/back ring at the crown.
    q=next(q for q in parts(GROUP,name)if q['seed']=='7569050876976495771')
    prism_part(m,q,294.5,302,(148,162,167),104)
    return m


def _faisaliah(lod):
    name='al-faisaliah-centre';m=Mesh();m.facade_material=104;foundation(m,name,(180,173,153))
    # Four tapering corner columns carry the square office floors and open crown.
    def size(y):return 41.3-(41.3-13)*min(y,220)/220
    def shape(y):return rect(size(y),size(y)*.96)
    vertical(m,shape(0),.2,22,lod,(112,151,154),(203,198,180),5.5,3.1,.8)
    loft(m,shape,floor_levels(22,184,5.0,lod),lod,(118,158,165),(196,198,184),2.0,.7)
    for sx in [-1,1]:
        for sz in [-1,1]:
            for lo,hi in [(0,90),(90,155),(155,220),(220,249)]:
                a=size(lo)/2 if lo<220 else 6.5;b=size(hi)/2 if hi<220 else max(1.5,6.5-(hi-220)*.17)
                m.beam((sx*a,lo,sz*a*.96),(sx*b,hi,sz*b*.96),.82 if lo<184 else .47,(219,217,192),95,(8,6,4)[lod])
    for y in [82,141,181]:
        p=shape(y)
        for i,(a,b) in enumerate(zip(p,np.roll(p,-1,axis=0))):
            strip(m,a,b,y,.7,2,(214,213,191))
            mid=(a+b)/2
            m.beam((shape(y-17)[i,0],y-17,shape(y-17)[i,1]),(mid[0],y,mid[1]),.45,(224,224,205),95,6)
            m.beam((shape(y-17)[(i+1)%4,0],y-17,shape(y-17)[(i+1)%4,1]),(mid[0],y,mid[1]),.45,(224,224,205),95,6)
    # Gold glazing is actual spherical geometry within the uncovered upper frame.
    r=11.4;center=200;profile=[(center+r*math.sin(t),max(.035,r*math.cos(t)))for t in np.linspace(-math.pi/2,math.pi/2,(32,24,16)[lod])]
    lathe(m,(0,0),profile,(177,142,59),95,(48,32,24)[lod])
    for y in [190.4,195,200,205,209.6]:
        radius=math.sqrt(max(0,r*r-(y-center)**2))+.05
        lathe(m,(0,0),[(y,radius),(y+.12,radius)],(209,192,127),95,(48,32,24)[lod])
    lathe(m,(0,0),[(230,3.2),(242,2.5),(249,1.6),(259,.4),(267,.08)],(205,211,200),95,(16,12,8)[lod])
    return m


def _bahrain(lod):
    name='bahrain-world-trade-center';m=Mesh();m.facade_material=103;foundation(m,name,(183,180,159));qs=main_parts(GROUP,name)
    centers=[np.array([0.,0.]),np.array([59.55,38.93])];axis=(centers[1]-centers[0]);axis/=np.linalg.norm(axis)
    wind=np.array([axis[1],-axis[0]])
    for j,q in enumerate(qs):
        p=ccw(q['outer']);c=centers[j];outward=axis*(-1 if j==0 else 1)
        def shape(y,p=p,c=c,outward=outward):
            t=y/240;s=max(.014,(1-t)**.62)
            return c+(p-c)*s+outward*(17*t)
        loft(m,shape,floor_levels(.2,239.8,4.8,lod)+[240],lod,(97,165,186),(193,210,205),2.5,.75)
        # White convex sail spines retain their continuous curve at all LODs.
        for sign in [-1,1]:
            for lo,hi in zip(np.linspace(0,240,(33,19,11)[lod]),np.linspace(0,240,(33,19,11)[lod])[1:]):
                def rib(y):
                    t=y/240;return c+outward*(17*t)+wind*sign*19*(1-t)**.62
                a,b=rib(lo),rib(hi);m.beam((a[0],lo,a[1]),(b[0],hi,b[1]),.40,(226,229,213),95,6)
    bridge_parts=[q for q in parts(GROUP,name)if q['seed'] in ['2143212552683684504','9189241806025722662','16235271060337160268']]
    for qbridge in bridge_parts:
        y=qbridge['height'];c=np.mean(qbridge['outer'],axis=0)
        ends=[]
        for j,q in enumerate(qs):
            t=y/240;s=max(.014,(1-t)**.62);outward=axis*(-1 if j==0 else 1)
            polygon=ccw(q['outer']);polygon=centers[j]+(polygon-centers[j])*s+outward*(17*t)
            hits=[]
            for u,v in zip(polygon,np.roll(polygon,-1,axis=0)):
                du,dv=np.dot(u-c,wind),np.dot(v-c,wind)
                if du*dv<=0 and abs(du-dv)>1e-9:
                    point=u+(v-u)*du/(du-dv);hits.append(np.dot(point-c,axis))
            if not hits:raise ValueError('turbine bridge must meet both sail sections')
            distance=max(hits) if j==0 else min(hits)
            ends.append(c+axis*(distance+(-.6 if j==0 else .6)))
        a,b=ends
        m.beam((a[0],y,a[1]),(b[0],y,b[1]),1.0,(194,211,205),95,(8,6,4)[lod])
        hub=np.array([c[0],y+1.7,c[1]])+np.array([wind[0],0,wind[1]])*2
        vaxis=np.array([axis[0],0,axis[1]]);normal=np.array([wind[0],0,wind[1]])
        m.beam(hub-normal*2.2,hub+normal*2,.7,(213,220,207),95,(12,8,6)[lod])
        for j in range(3):
            angle=j*math.tau/3+.24;radial=vaxis*math.cos(angle)+np.array([0,math.sin(angle),0]);tangent=-vaxis*math.sin(angle)+np.array([0,math.cos(angle),0])
            blade=[hub+radial*1.2-tangent*.6,hub+radial*4+tangent*1.5,hub+radial*14.5+tangent*.3,hub+radial*14.5-tangent*.15]
            face(m,blade,(226,234,225),95,normal);face(m,[p-normal*.18 for p in blade],(191,207,198),95,-normal)
    return m


def _hamra(lod):
    name='al-hamra-firdous-tower';m=Mesh();m.facade_material=103;foundation(m,name,(183,178,155))
    def shape(y):
        t=max(0,min(1,y/412));left=-29.95+29.9*t;right=left+29.95
        return np.array([[-30,-29.3],[left,-29.3],[left,-.3],[right,-.3],[right,-29.3],[30,-29.3],[30,29.3],[-30,29.3]])
    levels=floor_levels(.2,372,4.8,lod)+[382,392,402,412]
    m.facade_omit_edges={0,1,2,3,4}
    loft(m,shape,levels,lod,(117,161,167),(184,192,183),2.4,.65)
    # Jura limestone walls and twisting ribbons replace glazing on the cut south side.
    for lo,hi in zip(levels,levels[1:]):
        p,q=shape(lo),shape(hi)
        for i in range(5):
            a,b,c,d=p[i],p[i+1],q[i+1],q[i];edge=b-a
            normal=np.array([edge[1],0,-edge[0]]);normal/=max(.001,np.linalg.norm(normal))
            points=np.array([(a[0],lo,a[1]),(b[0],lo,b[1]),(c[0],hi,c[1]),(d[0],hi,d[1])])+normal*.07
            color=(216,207,178)if i%2==0 else(201,192,166)
            face(m,points,color,WEATHERED_LIMESTONE,normal)
    # Tall north lobby's inward columns remain visible through the recessed glass.
    for x in np.linspace(-25,25,7):m.beam((x,.2,29.5),(x*.77,24.4,27.8),.65,(223,217,195),95,(8,6,4)[lod])
    for y in [372,412]:
        p=shape(y)
        for a,b in zip(p,np.roll(p,-1,axis=0)):strip(m,a,b,y-.4,.7,.4,(213,216,199))
    return m


def _cayan(lod):
    name='cayan-tower';m=Mesh();m.facade_material=109;foundation(m,name,(151,151,140))
    def shape(y):
        angle=math.pi/2*max(0,min(1,(y-24)/283));c,s=math.cos(angle),math.sin(angle)
        return rect(39.9,37.0)@np.array([[c,s],[-s,c]]).T
    vertical(m,shape(0),.2,24,lod,(107,147,156),(206,204,187),4,3.2,.8)
    loft(m,shape,floor_levels(24,303.2,3.94,lod)+[307],lod,(107,150,165),(206,211,198),2.1,.95,.19)
    # Native recessed-residential shading provides floor/sill rhythm. Separate
    # proxy rings diverge from the adaptive twisted skin between its sections.
    return m


def _emirates(lod):
    name='emirates-towers';m=Mesh();m.facade_material=104;foundation(m,name,(187,183,163))
    outlines=[np.array([[-26.5,15.5],[26.9,15.8],[.2,-31.2]]),np.array([[-154.2,-18.2],[-125,27.1],[-100.2,-20.7]])]
    for index,p in enumerate(outlines):
        first_triangle=len(m.triangles)
        p=ccw(p);hi,roof_hi,tip=(274,312,354.6)if index==0 else(230,280,309)
        vertical(m,p,.2,hi,lod,(127,164,169),(205,210,195),4.7 if index==0 else 4.1,2.1,.8)
        # Sloped triangular crowns rise to one corner, with the mast on that apex.
        apex_index=int(np.argmin(p[:,1]))if index==0 else int(np.argmax(p[:,0]))
        yy=[hi+4]*3;yy[apex_index]=roof_hi
        for i,(a,b)in enumerate(zip(p,np.roll(p,-1,axis=0))):
            points=[(a[0],hi,a[1]),(b[0],hi,b[1]),(b[0],yy[(i+1)%3],b[1]),(a[0],yy[i],a[1])]
            edge=b-a;face(m,points,(176,190,185),95,(edge[1],0,-edge[0]))
            m.beam(points[-1],points[-2],.36,(226,229,212),95,6)
            for y in np.arange(hi+3,roof_hi,3.6):
                if yy[i]>=y and yy[(i+1)%3]>=y:strip(m,a,b,y,.25,.15,(139,161,157))
        face(m,[(a[0],y,a[1])for a,y in zip(p,yy)],(194,205,198),95)
        apex=p[apex_index];lathe(m,tuple(apex),[(roof_hi,1.4),(roof_hi+7,1.0),(tip-10,.55),(tip,.09)],(211,220,213),95,(16,12,8)[lod])
        if index==0:
            # The office approach is5.5m above the separate hotel control.
            for i in range(first_triangle,len(m.triangles)):
                points,color,material=m.triangles[i]
                m.triangles[i]=(points+np.array([0,5.5,0]),color,material)
            office=next(q for q in main_parts(GROUP,name)if q['seed']=='11874907196484683909')
            prism_part(m,office,-.5,5.7,(187,183,163),STONE_WALL)
    return m


SPECS=[
('kingdom-centre','Riyadh',302,302,'Elliptical blue-glass shaft with open parabolic crown, slim end piers and mapped bowed skybridge.',['https://www.arup.com/projects/kingdom-centre/'],'Engineer302m. Source205m lower shaft and mapped end-pier/skybridge plans retained; curved opening sill211–289m and bridge underside294.5m are proportion-based estimates.'),
('al-faisaliah-centre','Riyadh',267,267,'Tapering square office tower with four corner columns, K-braced belts, gold glazed sphere and open lantern.',['https://www.fosterandpartners.com/projects/al-faisaliah-center/','https://saudipedia.com/en/al-faisaliah-tower'],'Architect267m and gold sphere at200m; mapped41.3×40m plan. Sphere11.4m radius, office/body184m and brace levels estimated from architectural proportions.'),
('bahrain-world-trade-center','Manama',240,240,'Two asymmetric sail towers on mapped plans with three29m wind turbines carried on connecting bridges.',['https://www.otis.com/en/au/our-company/global-projects/project-showcase/bahrain-world-trade-center','https://www.skyscrapercenter.com/building/bahrain-world-trade-center-1/998'],'Primary240m tower/29m rotor diameter. Mapped twin-axis determines turbine orientation;70/100/130m bridge levels are source estimates. Sail taper and blade chord estimated.'),
('al-hamra-firdous-tower','Kuwait City',412,412,'Sculpted glass tower with migrating south cutout and two continuous Jura limestone ribbons.',['https://www.som.com/projects/al-hamra-tower/'],'Architect412m,74 stories, quarter-floor south cut migrates west to east,24.4m north lobby and39.6m upper hall. Mapped60×58.7m plan; cutout path and panel subdivision proportional approximations.'),
('cayan-tower','Dubai',307,307,'Ninety-degree twisting residential shaft with native recessed glazing and six-level podium.',['https://www.som.com/projects/cayan-tower/','https://www.cayan.net/projects/cayantower','https://www.cayan.net/live/wp-content/uploads/2023/10/Cayan-Corporate-Profile-Eng-2023.pdf'],'Architect307m and90 degree twist; owner podium six levels. Native terrain datum−2.7m follows the measured perimeter upper quartile; no inferred sea-level raising. Negative provider datum is not a surveyed absolute elevation. Floor pitch is estimated; native109 supplies facade floor rhythm.'),
('emirates-towers','Dubai',354.6,354.6,'Two equilateral triangular silver towers with unequal heights, diagonal crowns and apex masts.',['https://www.multiplex.global/projects/emirates-towers/','https://www.besix.com/en/projects/emirates-hotel-tower','https://www.skyscrapercentre.com/building/emirates-tower-one/311'],'Contractor354m office/309m hotel; CTBUH354.6m office tip. Source hotel parent incorrectly355m corrected to309m. Exact separate triangular plans; crown274–312/230–280m levels proportion estimates. Boulevard and parking retained native.')]
ENTRIES=[entry(GROUP,n,city,BASES[n],h,t,desc,urls,est)for n,city,h,t,desc,urls,est in SPECS]
for e in ENTRIES:
    if e['name']=='emirates-towers':
        control=read(GROUP,e['name'],'contact-calibration')
        hotel=next(q for q in main_parts(GROUP,e['name'])if q['seed']=='4828877939102876477')
        e['terrain_cuts']=[{'polygon_xz':t,'plane':[0,0,-.15],'feather_m':1.2}for t in hotel['triangles']]
        e['terrain_cuts'].append({'polygon_xz':control['office_approach_ring_xz'],'plane':[0,0,5.35],'feather_m':10})
        e['audit']['office_ground_datum_m']=17.1
    if e['name']=='bahrain-world-trade-center':
        bridges=[q for q in parts(GROUP,e['name'])if q['seed'] in ['2143212552683684504','9189241806025722662','16235271060337160268']]
        e['source_exclusion_polygons'].extend(t for q in bridges for t in q['triangles'])
        e['capture']['azimuth']=150
    if e['name']=='kingdom-centre':e['capture']['azimuth']=155
BUILDERS=dict(zip([s[0]for s in SPECS],[_kingdom,_faisaliah,_bahrain,_hamra,_cayan,_emirates]))
DEPENDENCIES=[Path(__file__),ROOT/'regional.py',ROOT.parent.parent/'crates/vearth-gen/src/highrise.rs',*sorted((ROOT/'references'/GROUP).glob('*.json'))]

def build(name,lod):
    if lod not in (0,1,2):raise ValueError('expected LOD0,1,2')
    return BUILDERS[name](lod)
