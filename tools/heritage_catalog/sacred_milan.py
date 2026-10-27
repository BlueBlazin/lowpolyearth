"""Candoglia marble cathedral: measured plan, stepped terraces and Gothic relief."""
import json
import math
from pathlib import Path

import numpy as np

from heritage_catalog.geometry import (
    Mesh, GOLD, arch, arch_bay, face, gable, lathe, plan_solid, plan_triangles,
    ring, stairs, stone_panel, tube, turn,
)
from heritage_catalog.materials import STONE_WALL, VEINED_MARBLE, SCULPTURE_METAL, retag

SOURCE = Path(__file__).parent/'references/sacred/milan-source.json'
SURVEY = json.loads(SOURCE.read_text())
PARTS = {b['survey_index']: b for b in SURVEY['buildings']}
MARBLE = (214, 207, 197)
EDGE = (230, 223, 212)
ROOF = (193, 185, 173)
GLASS = (49, 65, 62)


def _figure(m,x,y,z,height,lod,color=EDGE,material=17):
    lathe(m,(x,z),[(y,.20*height),(y+.14*height,.22*height),
          (y+.66*height,.10*height),(y+.79*height,.17*height)],color,material,(8,6,4)[lod])
    m.ellipsoid((x,y+.89*height,z),(.10*height,.11*height,.10*height),color,material,(8,6,4)[lod],4)


def _pinnacle(m,x,z,bottom,height,lod,radius=.8,figure=True):
    stem=height*.40
    lathe(m,(x,z),[(bottom,radius*1.15),(bottom+.35,radius*1.15),
          (bottom+.55,radius*.8),(bottom+stem,radius*.8),
          (bottom+stem+.35,radius*1.08)],MARBLE,11,8,math.pi/8)
    tip=bottom+height-(1.8 if figure else 0)
    lathe(m,(x,z),[(bottom+stem+.35,radius),(tip-.3,.08)],EDGE,17,8,math.pi/8)
    if lod<2:
        for i in range(4):
            a=i*math.pi/2+math.pi/4
            xx=x+radius*.70*math.cos(a);zz=z+radius*.70*math.sin(a)
            lathe(m,(xx,zz),[(bottom+.4,.12),(bottom+stem-.3,.12)],EDGE,17,4)
        for t in (.2,.4,.6,.8):
            y=bottom+stem+(tip-bottom-stem)*t
            r=radius*(1-t)
            for i in range(4):
                a=i*math.pi/2+math.pi/4
                # Short branching crockets are silhouette detail, not texture.
                tube(m,(x+r*math.cos(a),y,z+r*math.sin(a)),
                     (x+(r+.22)*math.cos(a),y+.25,z+(r+.22)*math.sin(a)),.12,EDGE,17,4)
    if figure:
        _figure(m,x,tip,z,1.8,lod)


def _window(m,x,y,z,width,height,lod,pointed=True):
    arch(m,(x,y,z),width,height,.20,EDGE,pointed,(24,16,10)[lod],GLASS,17)
    shaft=height*(.56 if pointed else .78)
    for i in range(1,4):
        xx=x-width/2+width*i/4
        m.box((xx,y+shaft/2,z-.055),(.11,shaft,.13),EDGE,17)
    for yy in (height*.18,height*.37):
        m.box((x,y+yy,z-.06),(width*.96,.11,.14),EDGE,17)
    if lod<2:
        # Exterior glazing is dark, with restrained ruby/olive/blue panes.
        colors=[(72,60,57),(55,67,68),(69,72,58),(62,56,69)]
        for row in range(3):
            for col in range(4):
                a=x-width/2+width*col/4+.10
                b=x-width/2+width*(col+1)/4-.10
                yy=y+.15+row*shaft/3
                face(m,[(a,yy,z+.018),(b,yy,z+.018),(b,yy+shaft/3-.2,z+.018),(a,yy+shaft/3-.2,z+.018)],
                     colors[(row+col)%4],15,(0,0,-1))
        if pointed:
            for dx,dy,r in [(-width*.20,height*.62,width*.17),(width*.20,height*.62,width*.17),(0,height*.79,width*.19)]:
                ring(m,(x+dx,y+dy,z-.075),r,.08,EDGE,(16,12)[lod],17,'xy')


def _rail(m,a,b,y,lod):
    a,b=np.asarray(a,float),np.asarray(b,float)
    length=np.linalg.norm(b-a)
    count=max(1,round(length/(1.5 if lod<2 else 2.4)))
    segment=length/count
    local=Mesh()
    local.box((0,y+.13,0),(length,.26,.55),EDGE,17)
    for i in range(count+1):
        x=-length/2+i*segment
        local.box((x,y+.85,0),(.18,1.7,.45),EDGE,17)
    for i in range(count):
        x=-length/2+(i+.5)*segment
        if lod<2:
            for sign in (-1,1):
                tube(local,(x+sign*segment*.5,y+1.05,0),(x,y+1.65,0),.13,EDGE,17,4)
        else:
            local.box((x,y+1.5,0),(segment,.22,.45),EDGE,17)
    angle=math.degrees(math.atan2(-(b-a)[1],(b-a)[0]))
    m.merge(turn(local,angle,float((a[0]+b[0])/2),0,float((a[1]+b[1])/2)))


def _marble_front(m,a,b,ya,yb,z,lod):
    colors=[(214,207,197),(218,207,201),(211,205,196),(220,214,206),(213,206,202)]
    width=(2.2,3.7,7.0)[lod];height=(1.2,2.0,4.0)[lod]
    for row in range(math.ceil(max(ya,yb)/height)):
        shift=width*.5*(row%2)
        for col in range(-1,math.ceil((b-a)/width)+1):
            x0=max(a,a+col*width+shift);x1=min(b,a+(col+1)*width+shift)
            if x1-x0<.02:continue
            low=row*height
            high0=min(low+height,ya+(yb-ya)*(x0-a)/(b-a))
            high1=min(low+height,ya+(yb-ya)*(x1-a)/(b-a))
            if min(high0,high1)<=low+.02:continue
            face(m,[(x0+.008,low+.008,z),(x1-.008,low+.008,z),
                 (x1-.008,high1-.008,z),(x0+.008,high0-.008,z)],
                 colors[(row*7+col*3)%len(colors)],11,(0,0,-1))


def _pediment(m,x,y,z,width,rise,lod,curved=False):
    m.box((x,y-.15,z),(width,.3,.7),EDGE,17)
    if curved:
        points=[(x+width*(t-.5),y+rise*math.sin(math.pi*t),z-.35)for t in np.linspace(0,1,(16,10,6)[lod]+1)]
    else:
        points=[(x-width/2,y,z-.35),(x,y+rise,z-.35),(x+width/2,y,z-.35)]
    for a,b in zip(points,points[1:]):
        face(m,[(a[0],y,z),(b[0],y,z),b,a],MARBLE,11,(0,0,-1))
        tube(m,a,b,.20,EDGE,17,4)
    if lod<2:
        for xx in (-width*.23,0,width*.23):
            _figure(m,x+xx,y+.25,z-.3,min(1.8,rise*.62),lod)


def _flying_buttress(m,side,z,lod):
    # Two masonry flights cross the aisles; the void below remains open.
    for inner,outer,y0,y1 in [(10.2,21.0,43.6,36.6),(21.0,31.0,36.6,31.8)]:
        x0,x1=side*inner,side*outer
        for i in range((12,8,5)[lod]):
            t0=i/(12,8,5)[lod];t1=(i+1)/(12,8,5)[lod]
            a=x0+(x1-x0)*t0;b=x0+(x1-x0)*t1
            ya=y0+(y1-y0)*t0;yb=y0+(y1-y0)*t1
            # The lower edge bows upward beneath the straight weathering course.
            ua=ya-1.1-1.2*math.sin(math.pi*t0)
            ub=yb-1.1-1.2*math.sin(math.pi*t1)
            for dz,normal in [(-.5,(0,0,-1)),(.5,(0,0,1))]:
                face(m,[(a,ua,z+dz),(b,ub,z+dz),(b,yb,z+dz),(a,ya,z+dz)],MARBLE,11,normal)
            face(m,[(a,ya,z-.5),(b,yb,z-.5),(b,yb,z+.5),(a,ya,z+.5)],EDGE,17,(0,1,0))
            face(m,[(a,ua,z-.5),(a,ua,z+.5),(b,ub,z+.5),(b,ub,z-.5)],MARBLE,11,(0,-1,0))


def build(lod):
    m=Mesh()
    outline=PARTS[686]['local_rings'][0]
    plan_solid(m,outline,-1.0,31.0,MARBLE)
    # The mapped roof-part heights are generic estimates; the Fabbrica's
    # surveyed visitor terraces establish the authored 31m/45m roof levels.
    nave=[[-10.4,-98.5],[12.0,-98.5],[12.0,-11.3],[-10.4,-11.3]]
    choir=PARTS[627]['local_rings'][0]
    plan_solid(m,nave,31,43.2,MARBLE)
    plan_solid(m,choir,31,43.2,MARBLE)
    for idx in (631,632):
        plan_solid(m,PARTS[idx]['local_rings'][0],31,43.2,MARBLE)
    gable(m,.8,-54.9,22.4,87.2,43.2,45,MARBLE,ROOF)
    gable(m,.5,27.9,20.3,33.0,43.2,45,MARBLE,ROOF)
    transept=Mesh();gable(transept,0,0,23.0,82,43.2,45,MARBLE,ROOF)
    m.merge(turn(transept,90))
    # Outer buttresses and their mapped pinnacle positions retain the slight
    # north/south plan asymmetry instead of mirroring the cadastral outline.
    spires=[]
    for idx,b in PARTS.items():
        p=np.array(b['local_rings'][0]);size=np.ptp(p,axis=0)
        if b['height']==35 and max(size)<8:
            x,z=(p.min(axis=0)+p.max(axis=0))/2
            plan_solid(m,p,-.4,33.0,MARBLE)
            spires.append((float(x),float(z),33.0,15.0,1.0))
        if b['height']==60 and max(size)<8:
            x,z=(p.min(axis=0)+p.max(axis=0))/2
            spires.append((float(x),float(z),43.0,17.0,.85))
    # Nave bays are fitted to the seven paired, mapped transverse buttresses.
    bay_z=(-85.8,-75.9,-66.4,-56.6,-46.9,-37.1,-27.5)
    for side in (-1,1):
        for z in bay_z:
            _flying_buttress(m,side,z,lod)
            spires.append((side*21,z,36.3,12,.62))
        facade=Mesh()
        for z in (-91.0,-80.8,-71.1,-61.4,-51.8,-42.1,-32.3):
            # Turning onto opposite side walls reverses their along-wall axis.
            along=side*z
            wall=[]
            for a,b in zip(outline,outline[1:]+outline[:1]):
                if min(a[1],b[1])<=z<=max(a[1],b[1]) and abs(b[1]-a[1])>1e-8:
                    wall.append(a[0]+(b[0]-a[0])*(z-a[1])/(b[1]-a[1]))
            distance=(max(wall) if side>0 else -min(wall))+.055
            _window(facade,along,6.3,-distance,5.0,20.4,lod)
            for dx in (-3.3,3.3):
                facade.box((along+dx,15.5,-distance-.18),(.28,27.8,.5),EDGE,17)
            _window(facade,along,33.0,-(12.06 if side>0 else 10.46),3.8,8.5,lod)
        m.merge(turn(facade,-90 if side>0 else 90))
        _rail(m,(side*31.8,-96),(side*31.8,-22),31.1,lod)
        _rail(m,(side*10.5,-95),(side*10.5,-12),45,lod)
    # Apse glazing occupies three tall polygonal east faces.
    apse=[([-31,38],[-10,54]),([-10,54],[10,54]),([10,54],[31,38])]
    for a,b in apse:
        a,b=np.array(a,float),np.array(b,float)
        d=b-a;mid=(a+b)/2
        f=Mesh();_window(f,0,5.5,-.08,min(14,np.linalg.norm(d)-5),23.5,lod)
        m.merge(turn(f,180+math.degrees(math.atan2(-d[1],d[0])),mid[0],0,mid[1]))
        _rail(m,a,b,31.1,lod)
    # West front has five bronze doors beneath a stepped marble screen.
    z=-100.75
    skyline=[(-33.8,34),(-27.8,36.8),(-21,40),(-12,46),(0,55),
             (12,46),(21,40),(27.8,36.8),(33.8,34)]
    for (a,ya),(b,yb) in zip(skyline,skyline[1:]):
        face(m,[(a,-.4,z),(b,-.4,z),(b,yb,z),(a,ya,z)],MARBLE,11,(0,0,-1))
        _marble_front(m,a,b,ya,yb,z-.018,lod)
        face(m,[(a,ya,z),(b,yb,z),(b,yb,z+1.8),(a,ya,z+1.8)],EDGE,17,(0,1,0))
        tube(m,(a,ya,z-.15),(b,yb,z-.15),.32,EDGE,17,4)
        count=max(1,round((b-a)/(1.6 if lod<2 else 2.3)))
        for i in range(count):
            x=a+(b-a)*(i+.5)/count
            yy=ya+(yb-ya)*(x-a)/(b-a)
            f=Mesh();arch_bay(f,(x,yy,z),1.35,2.8,.45,.15,EDGE,lod,True)
            m.merge(f)
            if lod<2:
                _pinnacle(m,x,z,yy+2.8,2.8,lod,.18,False)
    for i,x in enumerate((-26.7,-17.8,0,17.8,26.7)):
        width=5.8 if i==2 else 4.0
        height=10.5 if i==2 else 7.0
        m.box((x,height/2,z-.07),(width,height,.16),(65,74,63),16)
        for xx in (-width/2,width/2):
            m.box((x+xx, height/2,z-.30),(.45,height+.8,.55),EDGE,17)
        m.box((x,height+.35,z-.3),(width+1.0,.6,.55),EDGE,17)
        if lod<2:
            for row in range(4):
                for side in (-1,1):
                    m.box((x+side*width/4,(row+.5)*height/4,z-.2),
                          (width*.42,height/4-.3,.12),(84+row*2,89+row*2,71),16)
        _pediment(m,x,height+1.0,z-.4,width+1.9,2.1,lod,True)
        _window(m,x,18.0 if i==2 else 16.0,z-.25,5.8 if i==2 else 4.2,8.5 if i==2 else 7.5,lod,False)
        _pediment(m,x,27.2 if i==2 else 24.2,z-.42,8.0 if i==2 else 6.3,2.1,lod)
        if i==2:
            _window(m,x,32.0,z-.25,6.7,12.2,lod,True)
        elif i in (1,3):
            _window(m,x,27.2,z-.25,4.2,7.4,lod,True)
    for x,top in [(-33.5,34),(-21.5,40),(-10.8,47),(10.8,47),(21.5,40),(33.5,34)]:
        m.box((x,(top-.4)/2,z-.42),(.85,top+.4,.75),EDGE,17)
        for y in (5,14,24):
            if lod<2:
                _figure(m,x,y,z-.9,1.5,lod)
        spires.append((x,z,top,8.0,.65))
    for y in (1.0,12.0,28.7):
        m.box((0,y,z-.28),(67.6,.35,.6),EDGE,17)
    stairs(m,(0,-103.5),69,2.7,.75,(7,5,3)[lod],EDGE)
    for side in (-1,1):
        f=Mesh();stairs(f,(0,-46.5),22,3.3,.75,(7,5,3)[lod],EDGE)
        m.merge(turn(f,-90 if side>0 else 90))
        f=Mesh()
        _window(f,0,5.2,-43.3,13.0,24.0,lod)
        face(f,[(-12,43.2,-43.3),(12,43.2,-43.3),(0,51,-43.3)],MARBLE,11,(0,0,-1))
        m.merge(turn(f,-90 if side>0 else 90))
    # Octagonal crossing lantern and its eight secondary spires support the
    # openwork main spire; the gilded Madonnina itself is 4.16m tall.
    n=(32,24,16)[lod]
    lathe(m,(0,0),[(44.5,12.5),(46.0,12.5),(46.0,10.6),(61.8,10.6),(63.0,10.6)],MARBLE,11,8,math.pi/8)
    for i in range(8):
        a=i*math.tau/8
        f=Mesh();_window(f,0,49.0,-9.86,4.5,10.6,lod)
        m.merge(turn(f,math.degrees(a)))
        spires.append((11.1*math.sin(a+math.pi/8),11.1*math.cos(a+math.pi/8),61.5,18,.85))
    lathe(m,(0,0),[(62.4,11.0),(64.2,11.0),(69.3,4.4),(70.0,4.4)],EDGE,17,8,math.pi/8)
    for y,r,h in [(70,4.2,.6),(83.3,4.2,1.1),(90.6,2.9,.5),(96.4,1.8,.7)]:
        lathe(m,(0,0),[(y,r),(y+h,r)],EDGE,17,8,math.pi/8)
    for i in range(8):
        a=i*math.tau/8
        tube(m,(3.8*math.cos(a),70.5,3.8*math.sin(a)),(3.8*math.cos(a),83.3,3.8*math.sin(a)),.28,EDGE,17,8)
        tube(m,(3.8*math.cos(a),84.4,3.8*math.sin(a)),(1.3*math.cos(a),96.4,1.3*math.sin(a)),.22,EDGE,17,6)
        f=Mesh();arch_bay(f,(0,70.6,-3.51),2.8,12.7,.36,.26,EDGE,lod,True)
        m.merge(turn(f,math.degrees(a)+22.5))
    lathe(m,(0,0),[(97,1.5),(100.5,.8),(103.7,.40),(104.34,.55)],EDGE,17,n//2)
    _figure(m,0,104.34,0,4.16,lod,GOLD,SCULPTURE_METAL)
    for x,zz,y,h,r in spires:
        _pinnacle(m,x,zz,y,h,lod,r)
    # The block facade carries masonry joints and normal relief; fine carved
    # pinnacles retain the smoother marble response.
    return retag(m, {11:STONE_WALL,12:VEINED_MARBLE,17:VEINED_MARBLE,16:SCULPTURE_METAL})


OUTLINE=PARTS[686]['local_rings'][0]
ENTRY={
    'name':'milan-cathedral','city':'Milan',
    'lat':SURVEY['anchor_lat'],'lon':SURVEY['anchor_lon'],'heading':SURVEY['heading_deg'],
    'base':128.5,'activation_pad':2.0,
    'suppress_buildings':True,'suppress_roads':False,'suppress_trees':True,
    'source_exclusion_polygons':[p.tolist() for p in plan_triangles(OUTLINE)],
    'terrain_cuts':[
        {'polygon_xz':[[-65,-226],[65,-226],[65,-99],[-65,-99]],'plane':[0,0,0],'feather_m':22},
        {'polygon_xz':[[-65,-106],[65,-106],[65,85],[-65,85]],'plane':[0,0,0],'feather_m':22},
    ],
    'description':'Candoglia marble Gothic cathedral with measured five-aisle plan, flying buttresses, stepped terraces, traced glazed openings, sculptural pinnacles and gilded Madonnina.',
    'references':[
        'https://www.duomomilano.it/en/art-and-culture/the-terraces/',
        'https://www.duomomilano.it/en/spire/madonnina-main-spire/',
        'https://www.duomomilano.it/en/art-and-culture/the-cathedral/',
        'https://www.italia.it/es/lombardia/milan/catedral-de-milan',
        'https://www.openstreetmap.org/',
    ],
    'dimensions_m':{'main_spire_including_statue':108.5,'madonnina':4.16,'lower_terrace':31,'upper_terrace':45},
    'placement_evidence':{'source':'references/sacred/milan-source.json',
        'anchor':'mapped main spire centre; axis fitted to the long nave roof edge',
        'base':'128.5m production datum from open southwest piazza; roof-contaminated source centre is144.94m and is locally cut',
        'piazza_ground':'references/sacred/milan-piazza-ground.json; the mapped open forecourt is capped at the entry datum, preserving lower measured ground and retaining ordinary buildings and paths' },
    'interpretive_details':'Primary plan is surveyed. Small carvings and secondary pinnacle heights are simplified; mapped roof-part height estimates are replaced by the Fabbrica terrace measurements. The complete135-spire inventory remains to be reconciled with a roof plan.',
    'capture':{'distance':245,'azimuth':215,'elevation':25},
}
