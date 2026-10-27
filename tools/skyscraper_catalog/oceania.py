"""Eureka's faceted residential tower and One Central Park's planted pair."""
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.pacific_geometry import *

GROUP='oceania'
SITES={n:site(GROUP,n)for n in ('eureka-tower','one-central-park')}
BASES={'eureka-tower':14.2,'one-central-park':23.1}


def _eureka(lod):
    d=SITES['eureka-tower'];m=Mesh();base=BASES[d['name']]
    podium=part(d,13307317);support(m,d,podium,base)
    envelope(m,podium,0,20,lod,(133,146,151),(177,174,164),material=110)
    carpark=part(d,270552645);support(m,d,carpark,base)
    envelope(m,carpark,0,20,lod,(132,143,146),(167,170,165),material=107)
    tower=part(d,272730141)
    envelope(m,tower,20,261,lod,(96,128,149),(169,179,179),material=108)
    # The mapped flanking apartment wings end well below the gold crown.
    for id,hi in [(796349959,207),(796349960,177)]:
        p=part(d,id);envelope(m,p,20,hi,lod,(103,134,152),(178,182,177),material=108)
        if lod<2:
            for y in np.arange(23,hi,6.4 if lod==0 else 12.8):slab(m,p,y,.2,(181,187,181))
    # The top ten levels have gold-coated glazing; red is a narrow crown accent.
    crown=part(d,272731350);envelope(m,crown,261,297.3,lod,(166,145,94),(181,159,107),material=105)
    ribs(m,tower,20,261,12,.40,(182,187,181),lod)
    i=min(range(len(crown)),key=lambda i:((crown[i]+crown[(i+1)%len(crown)])/2)[0]);a=crown[i];b=crown[(i+1)%len(crown)]
    q=(a+b)/2;edge=(b-a)/np.linalg.norm(b-a);n=np.array([edge[1],-edge[0]])
    a=q-edge*1.6+n*.08;b=q+edge*1.6+n*.08
    face(m,[(a[0],261,a[1]),(b[0],261,b[1]),(b[0],297.3,b[1]),(a[0],297.3,a[1])],(154,84,75),PANEL_WALL,(n[0],0,n[1]))
    # The visitor's Edge is a 3m glass cube projecting beyond the Skydeck.
    i=max(range(len(tower)),key=lambda i:((tower[i]+tower[(i+1)%len(tower)])/2)[1]);a=tower[i];b=tower[(i+1)%len(tower)];q=(a+b)/2
    edge=(b-a)/np.linalg.norm(b-a);n=np.array([edge[1],-edge[0]]);p=clean([q-edge*1.5,q+edge*1.5,q+edge*1.5+n*3,q-edge*1.5+n*3])
    envelope(m,p,282,285,lod,(142,167,178),(165,184,185),material=103)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        m.beam((a[0],282,a[1]),(b[0],282,b[1]),.065,(166,182,183),SCULPTURE_METAL,4)
    return m


def _planted_tower(m,p,lo,hi,lod,phase):
    c=p.mean(0)
    envelope(m,p,lo,hi,lod,(111,133,140),(175,178,166),material=108)
    # Long planters and asymmetric living-wall panels preserve daylight openings.
    step=(3.2,6.4,12.8)[lod]
    for floor,y in enumerate(np.arange(lo+2.8,hi-1,step)):
        for side,(a,b) in enumerate(zip(p,np.roll(p,-1,axis=0))):
            edge=b-a;length=np.linalg.norm(edge);direction=edge/length;n=np.array([direction[1],-direction[0]])
            count=max(1,int(length/(6 if lod==0 else 10)))
            for j in range(count):
                if (j*3+floor+side+phase)%5==0:continue
                t0=(j+.08)/count;t1=(j+.93)/count;q0=a+edge*t0;q1=a+edge*t1
                planter=clean([q0,q1,q1+n*1.1,q0+n*1.1]);slab(m,planter,y,.28,(162,164,149))
                if lod<2:
                    q=(q0+q1)/2+n*.65
                    vegetation(m,q[0],y+.55,q[1],max(.5,np.linalg.norm(q1-q0)/2),.65,.7,lod,j+floor)
            # Vertical planted tapes vary by facade, reflecting the different
            # species/solar exposure rather than repeated solid green boxes.
            if side in (0,2) and floor%3==phase%3:
                for u in (.18,.65):
                    q=a+edge*u+n*.16;w=2.3
                    aa=q-direction*w/2;bb=q+direction*w/2
                    face(m,[(aa[0],y,aa[1]),(bb[0],y,bb[1]),(bb[0],min(hi,y+step*2),bb[1]),(aa[0],min(hi,y+step*2),aa[1])],(86,108,70),14,(n[0],0,n[1]))


def _central_park(lod):
    d=SITES['one-central-park'];m=Mesh();base=BASES[d['name']]
    podium=part(d,245274989);support(m,d,podium,base)
    envelope(m,podium,0,17,lod,(126,145,149),(167,175,166),material=103)
    east=part(d,541962534);west=part(d,541962536)
    _planted_tower(m,east,17,116,lod,0);_planted_tower(m,west,17,64.5,lod,1)
    # The console reaches west from the taller block over the shared retail court.
    # Its two-storey deep triangulated frame remains connected to the east tower.
    ec=east.mean(0);wc=west.mean(0);axis=(wc-ec)/np.linalg.norm(wc-ec);cross=np.array([-axis[1],axis[0]])
    start=ec+axis*8;end=start+axis*42;halfwidth=8
    deck=clean([start-cross*halfwidth,end-cross*halfwidth,end+cross*halfwidth,start+cross*halfwidth])
    slab(m,deck,96,.5,(161,170,159))
    for s in (-1,1):
        a=start+cross*halfwidth*s;b=end+cross*halfwidth*s
        for y in (89.5,96):m.beam((a[0],y,a[1]),(b[0],y,b[1]),.22,(143,157,156),SCULPTURE_METAL,6)
        for i in range(6):
            q=a+(b-a)*i/6;r=a+(b-a)*(i+1)/6
            m.beam((q[0],89.5,q[1]),(r[0],96,r[1]),.19,(150,163,160),SCULPTURE_METAL,6)
            m.beam((q[0],96,q[1]),(r[0],89.5,r[1]),.19,(150,163,160),SCULPTURE_METAL,6)
    for i in range((20,10,5)[lod]):
        for j in range((16,8,4)[lod]):
            q=start+axis*(42*(i+.5)/(20,10,5)[lod])+cross*(16*(j+.5)/(16,8,4)[lod]-8)
            a=q-axis*.62-cross*.60;b=q+axis*.62-cross*.60;c=q+axis*.62+cross*.60;e=q-axis*.62+cross*.60
            face(m,[(a[0],89.4,a[1]),(b[0],89.4,b[1]),(c[0],89.4,c[1]),(e[0],89.4,e[1])],(153,175,179),SCULPTURE_METAL,(0,-1,0))
    # Forty-two tracking mirrors on the west roof; lower LODs preserve the field.
    for i in range((7,5,3)[lod]):
        for j in range((6,4,3)[lod]):
            q=wc+axis*((i-(6,4,2)[lod]/2)*4)+cross*((j-(5,3,2)[lod]/2)*3.9)
            m.beam((q[0],64.5,q[1]),(q[0],65.6,q[1]),.08,(162,172,170),SCULPTURE_METAL,4)
            a=q-axis*1.25-cross*1.25;b=q+axis*1.25-cross*1.25;c=q+axis*1.25+cross*1.25;e=q-axis*1.25+cross*1.25
            face(m,[(a[0],65.1,a[1]),(b[0],65.1,b[1]),(c[0],66.1,c[1]),(e[0],66.1,e[1])],(159,184,188),SCULPTURE_METAL,(0,1,0))
    for p,y in [(west,64.5),(east,116),(deck,96)]:
        c=p.mean(0)
        for k,a in enumerate(p):
            q=a*.8+c*.2;vegetation(m,q[0],y+.8,q[1],1.4,.8,1.4,lod,k)
    return m


REF=Path(__file__).parent/'references'/GROUP
ENTRIES=[]
for name in SITES:
    a=json.loads((REF/(name+'-architecture.json')).read_text());d=SITES[name]
    owned=[part(d,13307317),part(d,270552645)]if name=='eureka-tower'else[part(d,245274989)]
    ENTRIES.append(make_entry(d,a['city'],BASES[name],owned,a['architectural_height_m'],a['primary_evidence'],a['references'],{
        'estimates':a['estimates'],'survey_file':f'tools/skyscraper_catalog/references/{GROUP}/{name}-site.json',
        'architecture_file':f'tools/skyscraper_catalog/references/{GROUP}/{name}-architecture.json','contact_camera':a['contact_camera'],
        'ground_method':'Fixed approach datum; lower-only footprint cuts and measured perimeter support toes buried 0.5m. Final engine ground-contact acceptance is separate.'},
        {'distance':680 if name=='eureka-tower'else 350,'azimuth':225,'elevation':23,'target_height':130 if name=='eureka-tower'else 55}))
DEPENDENCIES=[Path(__file__).parent/'pacific_geometry.py',Path(__file__).parent/'geometry.py',Path(__file__).parent/'inventory.py',
              Path(__file__).resolve().parents[2]/'crates/vearth-gen/src/highrise.rs',*REF.glob('*.json')]


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('LOD must be 0, 1 or 2')
    return {'eureka-tower':_eureka,'one-central-park':_central_park}[name](lod)
