"""Dohány Street Synagogue: measured plan, polychrome masonry and twin towers."""
import json
import math
from pathlib import Path

import numpy as np
from city_catalog.common import Mesh
from heritage_catalog import geometry as G, materials as M

HERE=Path(__file__).parent/'references'/'sacred_dohany'
DEPENDENCIES=[Path(G.__file__),Path(M.__file__)]
SOURCE=json.loads((HERE/'survey.json').read_text())
PLAN=json.loads((HERE/'plan.json').read_text())
NAME='dohany-street-synagogue'
BUFF=(202,167,107)
RED=(128,69,51)
STONE=(218,199,155)
RECESS=(43,48,44)
METAL=(92,105,105)
GOLD=(199,157,63)


def source_ring(ident):
    p=np.asarray(next(w for w in PLAN['ways'] if w['id']==ident)['xz'],float)
    return p[:-1] if np.linalg.norm(p[0]-p[-1])<1e-8 else p


def centroid(p):
    q=np.roll(p,-1,axis=0);cross=p[:,0]*q[:,1]-q[:,0]*p[:,1]
    return ((p+q)*cross[:,None]).sum(0)/(3*cross.sum())


# Long straight nave sides determine bearing; independent tower polygons locate
# the west facade without using the erroneous 5.91 m whole-complex OSM height.
coarse=next(b for b in SOURCE['buildings'] if b['source_index']==35)['rings'][0]
axis=np.asarray(coarse[5])-coarse[4]
HEADING=math.degrees(math.atan2(axis[0],axis[1]))
cs,sn=math.cos(math.radians(HEADING)),math.sin(math.radians(HEADING))
ROTATION=np.array([[cs,sn],[-sn,cs]])
ORIGIN=(centroid(source_ring(223530621))+centroid(source_ring(223530622)))/2
local=lambda p:(np.asarray(p)-ORIGIN)@ROTATION
TOWERS=[local(centroid(source_ring(i))) for i in [223530622,223530621]]
RING=local(source_ring(223530620))
LAT=SOURCE['lat']+ORIGIN[1]/111195
LON=SOURCE['lon']+ORIGIN[0]/(111195*math.cos(math.radians(SOURCE['lat'])))


def face(m,p,color=BUFF,material=M.BRICK_WALL,normal=(0,0,-1)):
    G.face(m,p,color,material,normal)


def striped_prism(m,polygon,bottom,top,lod,decorated=True):
    p=np.asarray(polygon,float)
    G.plan_solid(m,p,bottom,top,BUFF,M.BRICK_WALL)
    direction=np.sign(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(p,np.roll(p,-1,axis=0))))
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        edge=b-a;length=np.linalg.norm(edge)
        normal=direction*np.array([edge[1],-edge[0]])/length
        aa,bb=a+normal*.025,b+normal*.025
        # Burnt red facing courses wrap the corners, with broader buff fields.
        for y in np.arange(max(.5,bottom),top,.91):
            high=min(top,y+.25)
            face(m,[(aa[0],y,aa[1]),(bb[0],y,bb[1]),(bb[0],high,bb[1]),(aa[0],high,aa[1])],RED,M.BRICK_WALL,(normal[0],0,normal[1]))
        if decorated and lod<2:
            count=max(1,int(length/.85))
            for row,y in enumerate(np.arange(bottom+.65,top-.5,1.82)):
                for j in range(count):
                    if (j+row)%4:continue
                    u=(j+.5)/count;center=a+edge*u+normal*.04
                    v=edge/length*.18
                    tint=tuple(v-8 if row%3==0 else v+5 for v in BUFF)
                    face(m,[(center[0]-v[0],y,center[1]-v[1]),(center[0],y+.22,center[1]),(center[0]+v[0],y,center[1]+v[1]),(center[0],y-.22,center[1])],tint,M.BRICK_WALL,(normal[0],0,normal[1]))


def rectangular(m,x,z,w,d,top,lod,bottom=-.5,decorated=True):
    # A subgrade footing seats the facade on the slightly sloping street.
    striped_prism(m,[(x-w/2,z-d/2),(x+w/2,z-d/2),(x+w/2,z+d/2),(x-w/2,z+d/2)],bottom,top,lod,decorated)


def arch_window(m,x,y,z,w,h,lod,door=False,mullions=2):
    radius=w/2;spring=y+h-radius;n=(24,16,10)[lod]
    arc=[(x+radius*math.cos(t),spring+radius*math.sin(t),z) for t in np.linspace(0,math.pi,n+1)]
    face(m,[(x-radius,y,z),(x+radius,y,z),*arc],(59,47,35) if door else RECESS,M.TIMBER_WALL if door else M.WINDOW)
    # Alternating voussoirs are true relief bands surrounding the dark recess.
    for i in range(n):
        a,b=math.pi*i/n,math.pi*(i+1)/n
        pts=[(x+r*math.cos(t),spring+r*math.sin(t),z-.07) for r,t in [(radius,a),(radius,b),(radius+.28,b),(radius+.28,a)]]
        face(m,pts,STONE if i%2 else RED,M.WEATHERED_LIMESTONE if i%2 else M.BRICK_WALL)
    for side in [-1,1]:
        for j,yy in enumerate(np.arange(y,spring,.37)):
            m.box((x+side*(radius+.14),min(spring,yy+.37)/2+yy/2,z-.05),(.28,min(.37,spring-yy),.12),STONE if j%2 else RED,M.WEATHERED_LIMESTONE if j%2 else M.BRICK_WALL)
    if lod<2:
        for k in range(1,mullions+1):
            xx=x-radius+w*k/(mullions+1)
            m.beam((xx,y,z-.12),(xx,spring+math.sqrt(max(0,radius*radius-(xx-x)**2)),z-.12),.055,(85,80,61),M.STEEL,4)
        for yy in np.arange(y+.9,spring,.9):m.beam((x-radius,yy,z-.12),(x+radius,yy,z-.12),.04,(91,82,62),M.STEEL,4)
        if door:
            for xx in np.arange(x-radius+.25,x+radius,.3):m.beam((xx,y+.2,z-.04),(xx,spring,z-.04),.025,(92,63,40),M.TIMBER_WALL,4)


def circular_window(m,x,y,z,r,lod,clock=False):
    n=(36,24,16)[lod]
    color=(211,201,162) if clock else (48,69,64)
    face(m,[(x+r*math.cos(a),y+r*math.sin(a),z) for a in np.linspace(0,math.tau,n,endpoint=False)],color,M.PLASTER_WALL if clock else M.WINDOW)
    for i in range(n):
        a,b=i*math.tau/n,(i+1)*math.tau/n
        face(m,[(x+rr*math.cos(t),y+rr*math.sin(t),z-.08) for rr,t in [(r,a),(r,b),(r+.20,b),(r+.20,a)]],STONE,M.WEATHERED_LIMESTONE)
    if clock:
        for i in range(12):
            a=i*math.tau/12
            m.beam((x+r*.78*math.sin(a),y+r*.78*math.cos(a),z-.12),(x+r*.94*math.sin(a),y+r*.94*math.cos(a),z-.12),.024,RECESS,M.STEEL,4)
        # Static dial geometry; no claim is made about the clocks' current time.
        for a,length in [(math.radians(-60),.48),(math.radians(55),.35)]:m.beam((x,y,z-.14),(x+length*math.sin(a),y+length*math.cos(a),z-.14),.034,RECESS,M.STEEL,4)
    else:
        for i in range(12):
            a=i*math.tau/12
            m.beam((x+r*.35*math.cos(a),y+r*.35*math.sin(a),z-.12),(x+r*.96*math.cos(a),y+r*.96*math.sin(a),z-.12),.035 if lod<2 else .055,STONE,M.WEATHERED_LIMESTONE,4)
        for phase in [math.pi/2,-math.pi/2]:
            star=[(x+r*.72*math.cos(phase+i*math.tau/3),y+r*.72*math.sin(phase+i*math.tau/3),z-.15) for i in range(3)]
            for a,b in zip(star,star[1:]+star[:1]):m.beam(a,b,.045,STONE,M.WEATHERED_LIMESTONE,4)


def tile_frieze(m,x,z,width,y,lod):
    m.box((x,y,z),(width,.55,.16),RED,M.BRICK_WALL)
    if lod==2:return
    for xx in np.arange(x-width/2+.25,x+width/2,.6):
        face(m,[(xx-.19,y,z-.1),(xx,y+.21,z-.1),(xx+.19,y,z-.1),(xx,y-.21,z-.1)],STONE,M.WEATHERED_LIMESTONE)


def cornice(m,x,z,w,d,y,lod,merlons=False):
    for dy,gap,color in [(-.3,.22,RED),(0,.45,STONE),(.26,.15,BUFF)]:m.box((x,y+dy,z),(w+gap,.23,d+gap),color,M.WEATHERED_LIMESTONE if color==STONE else M.BRICK_WALL)
    if merlons:
        for xx in np.arange(x-w/2+.2,x+w/2,.82):m.box((xx,y+.62,z-d/2),(.35,.6,.45),STONE,M.WEATHERED_LIMESTONE)


def roof(m,x,z,w,d,eave,ridge,lod):
    # The museum aerial photograph shows a shallow metal roof with seams and
    # small rectangular roof lights; the metal composition is not documented.
    a,b,c,d0=(x-w/2,eave,z-d/2),(x+w/2,eave,z-d/2),(x+w/2,eave,z+d/2),(x-w/2,eave,z+d/2)
    lo,hi=(x,ridge,z-d/2+3),(x,ridge,z+d/2-3)
    for poly in [[a,b,lo],[b,c,hi,lo],[c,d0,hi],[d0,a,lo,hi]]:face(m,poly,METAL,M.METAL_ROOF,(0,1,0))
    for zz in np.arange(z-d/2+3,z+d/2-2,1.0 if lod==0 else 2.5):
        for side in [-1,1]:m.beam((x+side*w/2,eave+.035,zz),(x,ridge+.035,zz),.024,(122,132,128),M.METAL_ROOF,4)
    m.beam(lo,hi,.12,(118,127,121),M.METAL_ROOF,5)
    if lod<2:
        for zz in np.arange(z-d/2+8,z+d/2-3,10):
            for side in [-1,1]:
                xx=x+side*w*.22
                def skylight(dx,dz):
                    px=xx+dx
                    return (px,eave+(ridge-eave)*(1-abs(px-x)/(w/2))+.06,zz+dz)
                face(m,[skylight(-.6,-.85),skylight(.6,-.85),skylight(.6,.85),skylight(-.6,.85)],(43,58,59),M.WINDOW,(0,1,0))


def onion(m,x,z,lod):
    profile=[(35.0,1.5),(35.7,1.6),(36.4,2.15),(37.4,2.7),(38.5,2.8),(39.6,2.25),(40.6,1.25),(41.5,.38),(42.0,.08)]
    n=(48,32,24)[lod]
    for (y0,r0),(y1,r1) in zip(profile,profile[1:]):
        for i in range(n):
            a,b=i*math.tau/n,(i+1)*math.tau/n
            gold=y0>=36.4 and y1<=39.6 and i%2==0
            color=GOLD if gold else (66+(i%3)*3,75+(i%3)*3,67+(i%3)*3)
            material=M.SCULPTURE_METAL if gold else M.METAL_ROOF
            face(m,[(x+r0*math.cos(a),y0,z+r0*math.sin(a)),(x+r0*math.cos(b),y0,z+r0*math.sin(b)),(x+r1*math.cos(b),y1,z+r1*math.sin(b)),(x+r1*math.cos(a),y1,z+r1*math.sin(a))],color,material,(math.cos((a+b)/2),.2,math.sin((a+b)/2)))
    G.lathe(m,(x,z),[(41.8,.12),(42.25,.12),(42.3,.20),(42.55,.20),(42.7,.08),(43.6,.025)],GOLD,M.SCULPTURE_METAL,(12,10,8)[lod])


def tower(m,x,z,lod):
    rectangular(m,x,z,4.55,4.55,27.8,lod)
    for y in [8.8,18.6,23.3,27.5]:cornice(m,x,z,4.65,4.65,y,lod)
    for angle in [0,90,180,270]:
        part=Mesh()
        for xx in [-1,0,1]:arch_window(part,xx,24.2,-2.31,.58,2.7,lod,mullions=0)
        tile_frieze(part,0,-2.34,4.35,22.7,lod)
        m.merge(G.turn(part,angle,x,0,z))
    # Clock drum has eight sides and four 1.34 m clock faces (BZSH).
    G.lathe(m,(x,z),[(27.8,2.55),(28.2,2.55),(28.3,2.25),(34.2,2.25),(34.55,2.55),(35,2.55)],BUFF,M.BRICK_WALL,8)
    for y in [28.15,30.6,33.8,34.7]:G.lathe(m,(x,z),[(y,2.58),(y+.2,2.58)],STONE,M.WEATHERED_LIMESTONE,8)
    for angle in [0,90,180,270]:
        part=Mesh();circular_window(part,0,32.3,-2.32,.67,lod,True)
        for xx in [-.62,.62]:arch_window(part,xx,28.8,-2.32,.55,1.4,lod,mullions=0)
        m.merge(G.turn(part,angle,x,0,z))
    onion(m,x,z,lod)


def build(name,lod):
    if name!=NAME or lod not in (0,1,2):raise ValueError('Dohány model expects its exact name and LOD0–2')
    m=Mesh()
    # The temple body is distinct from the lower western foyer and its two
    # projecting entrance-court wings. Published 53.1 m controls the body length.
    rectangular(m,1.25,37.05,26.5,53.1,27,lod,decorated=False)
    roof(m,1.25,37.05,27.3,54.0,27.2,30.4,lod)
    rectangular(m,-.3,4.3,23.7,12.5,22.8,lod)
    roof(m,-.3,5.4,23.9,10.1,22.85,25.2,lod)
    left=[(-20.4,-10.2),(-12.1,-10.1),(-12.0,10.4),(-20.5,10.3)]
    right=[(11.7,-10.6),(18.7,-7.8),(18.1,7.1),(11.6,7.3)]
    for poly in [left,right]:
        striped_prism(m,poly,-.5,19.8,lod)
        p=np.asarray(poly);lo,hi=p.min(0),p.max(0);center=(lo+hi)/2
        roof(m,*center,*(hi-lo),19.9,21.3,lod)
    for x,z in TOWERS:tower(m,float(x),float(z),lod)
    # Three entrance bays and a fine rose window articulate the middle facade.
    for x in [-4.8,-.3,4.2]:arch_window(m,x,1.08,-2.02,3.0,5.8,lod,door=x==-.3,mullions=2)
    circular_window(m,-.3,15.9,-2.08,2.25,lod)
    for x in [-5.4,4.8]:
        for side in [-1,1]:arch_window(m,x+side*.55,12.0,-2.04,.85,6.7,lod,mullions=0)
    for x in np.arange(-6.5,6.3,1.28):arch_window(m,float(x),20.0,-2.04,.66,1.6,lod,mullions=0)
    for y in [7.5,10.6,19.25,22.25]:tile_frieze(m,-.3,-2.06,14.6,y,lod)
    cornice(m,-.3,4.3,24,12.5,22.8,lod,True)
    # Six shallow limestone steps lead to the main entrance (BZSH facade text).
    for i in range(6):m.box((-.3,(i+1)*.09-.25,-3.8+i*.3),(4.4,(i+1)*.18+.5,2.3-i*.3),STONE,M.WEATHERED_LIMESTONE)
    for side,x in [(-1,-12.03),(1,14.53)]:
        facade=Mesh()
        for z in np.linspace(16,58,6):
            for y,h in [(2.0,6.2),(10.5,6.0)]:arch_window(facade,side*float(z),y,0,2.6,h,lod,mullions=2)
        for z in [21.5,37.0,52.5]:circular_window(facade,side*z,22.1,0,1.7,lod)
        # Local facade x is the hall's long axis; rotate onto the side wall.
        m.merge(G.turn(facade,-side*90,x,0,0))
    for x in [-8.2,1.25,10.7]:
        f=Mesh();arch_window(f,x,3,0,3.8,10.0,lod,mullions=3);arch_window(f,x,16,0,3.0,7.5,lod,mullions=2)
        m.merge(G.turn(f,180,2.5,0,63.64))
    # The original corner wings retain round-headed upper windows and polygonal
    # pilasters; the separately mapped 1932 museum is not duplicated.
    for x,z in [(-16.2,-10.25),(15.1,-9.25)]:
        for y,h in [(2,5.0),(10.2,6.5)]:arch_window(m,x,y,z,3.5,h,lod,mullions=3)
    for side,x in [(-1,-11.95),(1,11.55)]:
        courtyard_face=Mesh()
        for z in [-5.6,2.0]:
            for y,h in [(2,5.0),(10.2,6.5)]:
                arch_window(courtyard_face,-side*z,y,0,2.7,h,lod,mullions=2)
        m.merge(G.turn(courtyard_face,side*90,x,0,0))
    for x,z in [(-20.4,-10.2),(-12.1,-10.1),(11.7,-10.6),(18.7,-7.8)]:
        G.lathe(m,(x,z),[(-.5,.58),(1,.58),(19.8,.48),(20.2,.72),(22.0,.12)],STONE,M.WEATHERED_LIMESTONE,(12,10,8)[lod])
    # Double tablets rise behind the towers, above the nave's western gable.
    for x in [.75,1.75]:
        m.box((x,30.5,10.5),(.92,1.5,.3),STONE,M.WEATHERED_LIMESTONE)
        G.lathe(m,(x,10.5),[(31.2,.46),(31.45,.35),(31.65,.04)],STONE,M.WEATHERED_LIMESTONE,10)
    return m


# Inset ownership avoids deleting the museum, long courtyard arcades and Heroes'
# Temple that share mapped edges with the synagogue's exterior shell.
OWNERSHIP=[p.tolist() for p in G.plan_triangles(RING)]
OWNERSHIP=[(np.asarray(p)*.985+np.asarray(p).mean(0)*.015).tolist() for p in OWNERSHIP]
ALL_POINTS=np.concatenate([np.asarray(p) for p in OWNERSHIP])
ENTRIES=[dict(name=NAME,city='Budapest',lat=float(LAT),lon=float(LON),heading=HEADING,base=114.3,
    suppress_buildings=True,suppress_roads=False,suppress_trees=True,
    source_exclusion_polygons=OWNERSHIP,activation_bounds_xz=[ALL_POINTS.min(0).tolist(),ALL_POINTS.max(0).tolist()],
    terrain_flatten=[],description='Striped yellow and red Moorish synagogue, twin 43.6 m onion-domed clock towers, rose tracery, shallow metal nave roof and oblique entrance wings.',
    dimensions={'published_temple_body_m':[53.1,26.5],'published_tower_height_m':43.6,'published_clock_diameter_m':1.34,'published_nave_internal_height_m':26,'source_rear_rectangle_length_m':56.5},
    references=['https://dohany-zsinagoga.hu/?page_id=789','https://cdn.culture.hu/uploads/documents/8/8a/8ac/8ac27401f6b50bbd45aa3330860c4782e1e38e5.pdf','https://hegedus.bzsh.hu/web/index.php/dr-schner-alfred-irasai/904-koenyv-a-dohany-utcai-zsinagoga.html','https://hrcak.srce.hr/file/70144','https://synagogues-360.anumuseum.org.il/gallery/dohany-street-synagogue/'],
    audit={'source_file':'tools/heritage_catalog/references/sacred_dohany/survey.json','source_anchor_latlon':[SOURCE['lat'],SOURCE['lon']],'source_offset_east_north_m':ORIGIN.tolist(),'tower_local_centers_xz_m':[p.tolist() for p in TOWERS],'tower_osm_ways':[223530622,223530621],'source_height_rejected_m':5.91,'ground_center_m':SOURCE['ground_m'],'dimension_qualification':'Published53.1m body length is used; mapped rear rectangle56.5m differs6.4%. Separate foyer/entrance wings make the full complex longer. Secondary elevations and ornamental subdivisions follow museum photographs.'},
    capture={'distance':145,'azimuth':215,'elevation':24,'target_height':20,'time_utc':'2026-03-20T14:00:00Z'})]
