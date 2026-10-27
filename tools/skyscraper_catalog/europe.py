"""Eight individually authored European skyscrapers, in metric model space."""
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.regional import *
from heritage_catalog.geometry import arch_bay,turn

GROUP='europe'
BASES={'tour-montparnasse':55.3,'torre-glories':11.8,'commerzbank-tower':114.8,'messeturm':102.0,
       'palace-of-culture-and-science':114.8,'20-fenchurch-street':14.9,'leadenhall-building':13.2,'unicredit-tower':123.1}

def foundation(m,name,color=(125,124,116)):
    footings(m,GROUP,name,BASES[name],color)


def _montparnasse(lod):
    name='tour-montparnasse';m=Mesh();m.facade_material=105;p=ccw(main_parts(GROUP,name)[0]['outer'])
    foundation(m,name,(78,73,66))
    # The current dark anodized envelope is retained during the 2026 renovation.
    vertical(m,p,.2,196,lod,(66,73,73),(55,49,42),3.32,1.5,1.18)
    c=p.mean(0);roof=c+(p-c)*.975
    vertical(m,roof,196,206,lod,(63,78,85),(91,90,81),3.3,1.6,.6)
    plan_solid(m,roof,206,207.4,(103,104,95),SCULPTURE_METAL)
    for a,b in zip(roof,np.roll(roof,-1,axis=0)):strip(m,a,b,207.4,.35,1.35,(172,177,171))
    plan_solid(m,rect(24,17),207.4,210,(99,103,101),SCULPTURE_METAL)
    if lod<2:
        for x in np.arange(-10,12,3.5):m.box((x,209,0),(1.8,1.6,8),(86,93,91),SCULPTURE_METAL)
    return m


def _glories(lod):
    name='torre-glories';m=Mesh();m.facade_material=103;foundation(m,name,(119,112,106))
    n=(80,56,40)[lod];height=144.4
    def shape(y):
        # The concrete shaft bows slightly before the elliptic glazed crown.
        r=1-.04*(max(0,y)/105)**2 if y<105 else .96*math.sqrt(max(.000025,1-((y-105)/(height-105))**2))
        return ellipse(19.89*r,18.43*r,n)
    def glass(f,i,k):
        t=min(1,f*3.9/128);palette=[(128,76,73),(118,89,106),(80,117,153),(80,139,157)]
        a=palette[min(3,int(t*4))];delta=((i*17+f*13)%9-4)*4
        return tuple(max(0,min(255,v+delta))for v in a)
    levels=list(np.arange(.2,140,3.9))+[142,143.5,144.4]
    loft(m,shape,levels,lod,glass,(148,151,147),bay=1.45,spandrel=.2,mullion=.025)
    # Native facade filtering carries the fine louver rhythm without sparse
    # metallic proxy rings that alias against the curved outer skin.
    return m


def _commerzbank(lod):
    name='commerzbank-tower';m=Mesh();m.facade_material=104;foundation(m,name,(172,170,155))
    for q in read(GROUP,name,'envelope')['layers']:
        lo,hi=q['bottom'],q['top'];p=ccw(q['outer'])
        loft(m,lambda y:p,[lo,hi],lod,(99,138,147),(175,180,169),2.3,.82,cap_top=False)
        for tri in q['triangles']:face(m,[(x,hi,z)for x,z in tri],(120,133,130),FLAT_ROOF)
        # Courtyard rings are not closed by a full triangular parent prism.
        for ring in q['holes']:
            ring=ccw(ring)[::-1]
            for a,b in zip(ring,np.roll(ring,-1,axis=0)):
                curtain_wall(m,a,b,lo,hi,4,2.3,(99,138,147),(175,180,169),lod)
        for tri in q['triangles']:
            face(m,[(x,lo,z)for x,z in tri],(151,160,151),FLAT_ROOF,(0,-1,0))
        if hi in [60,70,80,100,110,120,140,150]:
            for a,b in zip(p,np.roll(p,-1,axis=0)):
                if np.linalg.norm(b-a)>10:strip(m,a,b,hi+.04,.48,.55,(121,134,110),STONE_WALL)
    # Slender, occupied corner cores rise beyond the garden petals; mast is separate.
    lathe(m,(-11.05,-7.0),[(259,2.5),(265,2.5),(265,.6),(283,.48),(300.0,.12)],(193,196,184),SCULPTURE_METAL,(16,12,8)[lod])
    return m


def _messeturm(lod):
    name='messeturm';m=Mesh();m.facade_material=106;foundation(m,name,(159,118,106));stone=(177,134,118)
    p=ccw(main_parts(GROUP,name)[0]['outer'])
    lobby=next(q for q in parts(GROUP,name)if q['seed']=='13432480008112848836')
    m.facade_material=104
    vertical(m,ccw(lobby['outer']),.2,17,lod,(137,157,165),(165,170,169),17,6,.1)
    m.facade_material=106
    vertical(m,p*.97,17,30,lod,(83,108,119),stone,6.5,5,1.2,WEATHERED_LIMESTONE)
    for x in [-17.6,17.6]:
        for z in [-17.6,17.6]:m.box((x,15,z),(3.8,30,3.8),stone,WEATHERED_LIMESTONE)
    for q in read(GROUP,name,'envelope')['layers']:
        vertical(m,ccw(q['outer']),q['bottom'],q['top'],lod,(95,119,126),stone,3.68,2.0,1.4,WEATHERED_LIMESTONE)
    # The circular crown and pyramidal copper roof are separate structural forms.
    vertical(m,ellipse(16.2,16.2,(48,32,24)[lod]),210,220,lod,(96,126,134),(191,159,139),5,2,.6)
    top=ccw(next(q for q in parts(GROUP,name)if q['seed']=='11515491196798397075')['outer']);apex=(.1,256.5,-.1)
    for a,b in zip(top,np.roll(top,-1,axis=0)):
        n=(b[1]-a[1],15,a[0]-b[0]);face(m,[(a[0],220,a[1]),(b[0],220,b[1]),apex],(170,128,111),SCULPTURE_METAL,n)
        if lod<2:
            for t in np.linspace(.12,.9,8):
                aa=np.array([a[0],220,a[1]])*(1-t)+np.array(apex)*t;bb=np.array([b[0],220,b[1]])*(1-t)+np.array(apex)*t
                m.beam(aa,bb,.07,(131,112,103),SCULPTURE_METAL,4)
    return m


def _palace(lod):
    name='palace-of-culture-and-science';m=Mesh();m.facade_material=106;foundation(m,name,(162,149,128));stone=(201,188,162);trim=(219,207,181)
    qs=parts(GROUP,name);low=next(q for q in qs if q['seed']=='17286896172022932443')
    prism_part(m,low,.2,18,stone,WEATHERED_LIMESTONE,sides=False)
    def masonry(q,lo,hi,pitch=4.6,bay=3.7):
        p=ccw(q['outer']);vertical(m,p,lo,hi,lod,(77,88,84),stone,pitch,bay,1.35,WEATHERED_LIMESTONE)
        for y in ([lo+.6,hi-1.0,hi] if lod==0 else [hi-1.0,hi] if lod==1 else [hi]):
            for a,b in zip(p,np.roll(p,-1,axis=0)):strip(m,a,b,y,.45,.35,trim,WEATHERED_LIMESTONE)
        return p
    for ring in [low['outer']]+low['holes']:
        p=ccw(ring)
        if ring is not low['outer']:p=p[::-1]
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            if np.linalg.norm(b-a)>2:curtain_wall(m,a,b,.2,18,5.1,3.8,(72,87,85),stone,min(lod,1),1.7,.9,105)
    for q in qs:
        if q['min_height']==18 and q['height']<=30:masonry(q,18,q['height'])
    shoulder=next(q for q in qs if q['seed']=='5593721448005934833');masonry(shoulder,18,67,4.6,3.8)
    shaft=next(q for q in qs if q['seed']=='10976408832712539429');p=masonry(shaft,67,114,4.3,3.25)
    center=np.array([.85,16.7]);terrace=center+(p-center)*1.035
    plan_solid(m,terrace,114,115.5,trim,WEATHERED_LIMESTONE)
    for a,b in zip(terrace,np.roll(terrace,-1,axis=0)):strip(m,a,b,115.5,.6,1.6,stone,WEATHERED_LIMESTONE)
    # Setback clock tower is narrower than the office shaft; clocks are 6.3m wide.
    for w,d,lo,hi in [(32,32,115.5,132),(24,25,132,146),(17.2,17.2,146,168),(12.3,12.9,168,180)]:
        q={'outer':rect(w,d,center)};masonry(q,lo,hi,4.6,3.3)
    for side in range(4):
        angle=side*math.pi/2;normal=np.array([math.sin(angle),0,math.cos(angle)]);tangent=np.array([normal[2],0,-normal[0]])
        c=np.array([center[0],157,center[1]])+normal*8.67
        ring=[]
        for t in np.linspace(0,2*math.pi,(48,32,20)[lod],endpoint=False):ring.append(c+tangent*math.cos(t)*3.15+np.array([0,math.sin(t)*3.15,0]))
        for a,b in zip(ring,np.roll(ring,-1,axis=0)):face(m,[c,a,b],(225,213,174),17,normal)
        m.beam(c+normal*.06,c+tangent*1.77+np.array([0,3.065,0])+normal*.06,.07,(44,48,45),95,4)
        m.beam(c+normal*.08,c+tangent*-2.34+np.array([0,1.35,0])+normal*.08,.1,(44,48,45),95,4)
        for k in range(12):
            t=k*math.pi/6;m.beam(c+tangent*math.sin(t)*2.65+np.array([0,math.cos(t)*2.65,0])+normal*.04,c+tangent*math.sin(t)*2.95+np.array([0,math.cos(t)*2.95,0])+normal*.04,.07,(63,64,57),95,4)
    lathe(m,tuple(center),[(180,7.5),(182,7.5),(184,5.1),(187.68,4.8),(194,2.5),(217,.55),(230.68,.12),(237,.07)],(149,158,146),SCULPTURE_METAL,(32,24,16)[lod])
    # Corner Renaissance attics crown each terrace, not arbitrary rooftop spikes.
    for x in [-14.3,14.3]:
        for z in [-14.3,14.3]:
            lathe(m,tuple(center+[x,z]),[(132,1.4),(135,1.4),(138,.8),(141,.15)],stone,WEATHERED_LIMESTONE,(12,8,6)[lod])
    return m


def _fenchurch(lod):
    name='20-fenchurch-street';m=Mesh();m.facade_material=104
    # Model +x follows the long facade; measured outer roof is wider than lobby.
    def shape(y):
        t=max(0,min(1,(y-7.7)/137));w=45.5+20.8*(t*t*.7+t*.3);d=39.5+15*t
        p=rect(w,d);p[:,0]+=2.5*(1-t);return p
    plan_solid(m,shape(0),-4,.2,(146,150,139),STONE_WALL)
    vertical(m,shape(7.7),.2,7.7,lod,(97,142,152),(187,192,184),7.5,3.4,.25)
    loft(m,shape,floor_levels(7.7,144,4.25,lod),lod,(105,151,165),(205,210,200),2.2,.75,.18)
    # Broad white vertical fins follow the curved, outward-growing elevations.
    for edge in range(4):
        count=(9,7,5)[lod] if edge%2==0 else (7,5,4)[lod]
        for u in np.linspace(0,1,count):
            for lo,hi in zip(np.linspace(7.7,144,(10,7,5)[lod]),np.linspace(7.7,144,(10,7,5)[lod])[1:]):
                a=shape(lo);b=shape(hi);v=a[edge]*(1-u)+a[(edge+1)%4]*u;w=b[edge]*(1-u)+b[(edge+1)%4]*u
                m.beam((v[0],lo,v[1]),(w[0],hi,w[1]),.21,(208,214,206),PANEL_WALL,4)
    # Three-level Sky Garden glazing wraps a shallow arched roof with real ribs.
    p=shape(144);vertical(m,p,144,153,lod,(139,174,174),(183,198,193),4.5,2.2,.18)
    xs=np.linspace(-33.15,33.15,(24,16,12)[lod]);zs=(-27.25,27.25)
    def roof(x):return 153+7*math.sqrt(max(0,1-(x/33.15)**2))
    for a,b in zip(xs,xs[1:]):face(m,[(a,roof(a),zs[0]),(b,roof(b),zs[0]),(b,roof(b),zs[1]),(a,roof(a),zs[1])],(145,178,181),WINDOW,(0,1,0))
    for z in np.linspace(*zs,(15,11,7)[lod]):
        for a,b in zip(xs,xs[1:]):m.beam((a,roof(a)+.05,z),(b,roof(b)+.05,z),.11,(202,210,205),95,4)
    return m


def _leadenhall(lod):
    name='leadenhall-building';m=Mesh();m.facade_material=103;foundation(m,name,(142,145,138))
    qs=parts(GROUP,name)
    for q in qs:
        if q['seed']in ['7207863093169503675','9899206779041563347','3960270491468644442','11006299752801360093']:
            vertical(m,ccw(q['outer']),.2,q['height'],lod,(117,152,155),(199,180,63),4,2,.75)
    def shape(y):
        south=27.7-(y-28.5)/(224.5-28.5)*36.5
        return rect(south+9.7,48.4,((south-9.7)/2,0))
    loft(m,shape,floor_levels(28.5,224.5,4.08,lod),lod,(114,155,170),(173,193,195),2.1,.5)
    for q in qs:
        if q['seed']in ['4197548780318393228','11243578037866138234','18289607286673347320']:
            prism_part(m,q,q['min_height'],q['height'],(153,161,157),SCULPTURE_METAL)
    # The south groundcroft is open; external megaframe carries the overhang.
    ys=[.2,28.5,57,85,113,141,169,197,224.5]
    for z in [-24.4,24.4]:
        for j,(lo,hi)in enumerate(zip(ys,ys[1:])):
            sx=lambda y:27.7-max(0,y-28.5)/(224.5-28.5)*36.5
            m.beam((sx(lo),lo,z),(sx(hi),hi,z),.57,(211,222,220),95,(8,6,4)[lod])
            m.beam((-9.8,lo,z),(-9.8,hi,z),.5,(206,217,215),95,(8,6,4)[lod])
            m.beam((-9.8,lo,z),(sx(hi),hi,z),.32,(200,213,212),95,(8,6,4)[lod])
            if j%2==0:m.beam((sx(lo),lo,z),(-9.8,hi,z),.32,(200,213,212),95,(8,6,4)[lod])
            m.beam((-9.8,hi,z),(sx(hi),hi,z),.33,(206,220,218),95,(8,6,4)[lod])
    for z in [-16,0,16]:m.beam((27.7,.2,z),(27.7,28.5,z),.65,(212,219,208),95,8)
    return m


def _unicredit(lod):
    name='unicredit-tower';m=Mesh();m.facade_material=107;foundation(m,name,(165,165,151))
    qs=parts(GROUP,name)
    selected={'18119073773535275737':146,'12101046335778266713':100,'15427730080555090861':50}
    for q in qs:
        if q['seed'] in selected:
            p=ccw(q['outer']);hi=selected[q['seed']]
            vertical(m,p,.2,hi,lod,(90,146,167),(153,181,188),4.5,1.8,.45)
            # Native horizontal-office shading supplies the dense sunshade rhythm.

    # Tower A's bright spiral blade tapers around the surveyed mast anchor.
    cx,cz=12.18,-26.73;n=(40,28,20)[lod]
    for lo,hi in zip(np.linspace(146,231,n),np.linspace(146,231,n)[1:]):
        def ring(y):
            t=(y-146)/85;radius=4.5*(1-t)+.08;ang=t*math.pi*2.1
            return np.array([(cx+math.cos(ang+a)*radius,cz+math.sin(ang+a)*radius)for a in np.linspace(-.2,math.pi*1.15,10)])
        p,q=ring(lo),ring(hi)
        for a,b,c,d in zip(p,p[1:],q[1:],q):
            points=[(a[0],lo,a[1]),(b[0],lo,b[1]),(c[0],hi,c[1]),(d[0],hi,d[1])]
            normal=(a[0]-cx,0,a[1]-cz);face(m,points,(222,225,216),95,normal);face(m,points,(189,202,201),95,tuple(-v for v in normal))
    lathe(m,(cx,cz),[(146,1.05),(197,.7),(226,.3),(231,.08)],(202,212,210),95,(12,8,6)[lod])
    return m


SPECS=[
('tour-montparnasse','Paris',210,210,'Measured curved rectangular dark aluminum tower with 59 floor bands, observation terrace and roof plant.',['https://www.tourmontparnasse56.com/fr/la-tour-montparnasse-en-chiffres/','https://www.paris.fr/pages/la-tour-montparnasse-fete-ses-50-ans-24034'],'Owner 210m and 59 floors; current dark facade retained. Observatory closed 31 March 2026 for renovation; no future proposed facade modeled. Roof plant and secondary bands are photograph-based estimates.'),
('torre-glories','Barcelona',144.4,144.4,'Ellipsoidal colored shaft, glass-louver outer skin and continuous domed crown.',['https://www.jeannouvel.com/en/projects/tour-agbar/','https://www.miradortorreglories.com/ca/sobre-nosaltres/'],'Architect program142m differs from operator144m; mapped144.4m envelope retained with0.4m precision treated as an estimate. Crown spring105m, facade module and color distribution are photograph-based approximations, not panel-by-panel survey.'),
('commerzbank-tower','Frankfurt',259,300,'Triangular three-core tower with nine deep winter gardens, stepped crown and separate antenna.',['https://www.arup.com/projects/commerzbank-tower/','https://www.commerzbank.de/konzern/newsroom/pressemitteilungen/buerogebaeude.html','https://www.permasteelisagroup.com/historic-project/commerzbank-tower/','https://dev.fosterandpartners.com/projects/commerzbank-headquarters'],'Owner259m, facade contractor300m tip; source plans give rounded garden levels, which remain estimates. Source solid envelope is omitted to preserve real sky-garden voids.'),
('messeturm','Frankfurt',256.5,256.5,'Rose granite stepped office shaft, renovated curved-glass lobby, cylindrical crown and copper pyramid.',['https://www.skyscrapercenter.com/building/building/796','https://jahn.studio/work/messeturm/','https://messeturm.com/'],'CTBUH256.5m; mapped188/194/200/210/220m shoulders and drum levels rounded. Jahn2022 curved-glass lobby17m retained; granite panel subdivisions estimated. Jahn project summary251m differs from CTBUH256.5m and owner rounded257m; CTBUH tip used.'),
('palace-of-culture-and-science','Warsaw',230.68,237,'Courtyard-preserving stone complex with stepped office shaft,114m terrace, four6.3m clocks and spire.',['https://pkin.pl/en/trivia/','https://pkin.pl/informacje-o-pkin/'],'Owner187.68m without spire,230.68m original tip,237m with1994 antenna,114m terrace. OSM upper-envelope heights conflict; upper setbacks and clock center157m are proportion-based estimates constrained to owner heights.'),
('20-fenchurch-street','London',160,160,'Outward-growing curved glass tower with white fins, double-height lobby and three-level arched Sky Garden.',['https://vinoly.com/works/20-fenchurch-street/'],'Architect160m,32 office levels and3 Sky Garden levels. Roof envelope mapped; curvature, fin pitch and intermediate levels estimated. Southern annex and neighboring pocket park retained as native sources.'),
('leadenhall-building','London',224.5,224.5,'South-tapering glass wedge carried over an open groundcroft by steel megaframes, with yellow north service core.',['https://rshp.com/projects/office/the-leadenhall-building/','https://rshp.com/assets/uploads/3510_TheLeadenhallBuilding_JS_en_1.pdf'],'Architect224.5m. Mapped28.5m office underside and service-core footprints; slope and structural bay sizes proportionally reconstructed. Groundcroft remains open except actual cores and external columns.'),
('unicredit-tower','Milan',231,231,'Three mapped curved glazed office wings with horizontal sunshades and Tower A spiral metal spire.',['https://pcparch.com/work/porta-nuova-garibaldi-and-unicredit-headquarters','https://www.unicreditgroup.eu/it/press-media/press-releases/2013/unicredit-tower--nuova-direzione-generale.html','https://www.holcim.it/torre-unicredit','https://www.skyscrapercenter.com/milan/unicredit-tower/5292'],'Owner231m and supplier146m body+85m spire used. CTBUH currently lists217.7m, an unresolved published datum discrepancy; not silently merged. B/C100/50m are mapped estimates. Piazza remains unowned and unfilled.')]
ENTRIES=[entry(GROUP,n,city,BASES[n],h,t,desc,urls,est)for n,city,h,t,desc,urls,est in SPECS]
BUILDERS=dict(zip([s[0]for s in SPECS],[_montparnasse,_glories,_commerzbank,_messeturm,_palace,_fenchurch,_leadenhall,_unicredit]))
DEPENDENCIES=[Path(__file__),ROOT/'regional.py',ROOT.parent.parent/'crates/vearth-gen/src/highrise.rs',*sorted((ROOT/'references'/GROUP).glob('*.json'))]

def build(name,lod):
    if lod not in (0,1,2):raise ValueError('expected LOD0,1,2')
    return BUILDERS[name](lod)
