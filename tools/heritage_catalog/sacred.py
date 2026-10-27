"""Individually composed churches, synagogues, and mosques in metric units."""
import math
import json
from pathlib import Path

import numpy as np

from heritage_catalog.geometry import (
    Mesh, STONE, TRIM, SHADOW, LEAD, GOLD, arch, arch_bay, balustrade,
    column, cross, dome, face, gable, lathe, ring, stairs, stone_panel,
    tube, turn, window, plan_triangles,
)
from heritage_catalog.materials import WEATHERED_LIMESTONE, METAL_ROOF, SCULPTURE_METAL, retag


_PAULS_SOURCE = json.loads((Path(__file__).parent/'references/sacred/st-pauls-source.json').read_text())
_PAULS_HEADING = math.radians(83.60227301952843)
_PAULS_ROTATION = np.array([[math.cos(_PAULS_HEADING),-math.sin(_PAULS_HEADING)],
                            [math.sin(_PAULS_HEADING),math.cos(_PAULS_HEADING)]])
# The same OSM building is split at a source-tile boundary. All four retained
# pieces are required to own the projecting transept bays and porticoes.
_PAULS_PLANS = [(np.asarray(b['rings'][0])-[2.6983,-7.0883])@_PAULS_ROTATION.T
                for b in _PAULS_SOURCE['buildings']
                if b['seed']=='3285129857158104402' and b['kind']=='Building' and b['height']==35]
assert len(_PAULS_PLANS) == 4, 'St Pauls requires all surveyed building fragments'
_PAULS_OWNERSHIP = [p.tolist() for plan in _PAULS_PLANS for p in plan_triangles(plan)]
_PAULS_OWNERSHIP.append([[-29,-98.5],[29,-98.5],[29,-83],[-29,-83]])


def _statue(m, x, y, z, h, lod, color=TRIM):
    # Small skyline figures keep head/robe proportions; sub-metre carving is
    # intentionally removed before the distant mesh, not replaced by texture.
    if lod == 2:
        return
    lathe(m, (x,z), [(y,.25*h),(y+.08*h,.25*h),(y+.68*h,.14*h),
          (y+.78*h,.17*h)],color,17,8)
    m.ellipsoid((x,y+.9*h,z),(.13*h,.16*h,.13*h),color,17,8,4)


def _pauls_tower(m, x, z, lod):
    # Foundations cross the local ground slope; published stage heights use y=0.
    m.box((x,16.85,z),(12.6,34.7,12.6),STONE)
    for y, size, h in [(31.5,13.3,.7),(33.5,13.6,.7),(35.0,12.3,.8),
                       (49.5,11.9,.8),(51.2,12.7,.8)]:
        m.box((x,y+h/2,z),(size,h,size),TRIM,17)
    m.box((x,36.3,z),(11.1,8.8,11.1),STONE)
    m.box((x,46.0,z),(9.0,12.0,9.0),STONE)
    for angle in (0,90,180,270):
        panel=Mesh()
        window(panel,0,42.0,-4.54,3.1,7.0,lod,TRIM,False,True)
        # The western clock faces and louvred belfries occupy different tiers.
        if angle == 0 and x > 0:
            lathe_clock=Mesh()
            for i in range((36,24,16)[lod]):
                a=i*math.tau/(36,24,16)[lod];b=(i+1)*math.tau/(36,24,16)[lod]
                face(lathe_clock,[(0,36.8,-5.62),(2.3*math.cos(a),36.8+2.3*math.sin(a),-5.62),
                    (2.3*math.cos(b),36.8+2.3*math.sin(b),-5.62)],SHADOW,15,(0,0,-1))
            ring(lathe_clock,(0,36.8,-5.69),2.35,.13,GOLD,(36,24,16)[lod],17,'xy')
            for i in range(12):
                a=i*math.tau/12
                tube(lathe_clock,(1.87*math.sin(a),36.8+1.87*math.cos(a),-5.76),
                    (2.1*math.sin(a),36.8+2.1*math.cos(a),-5.76),.06,GOLD,16,4)
            tube(lathe_clock,(0,36.8,-5.79),(.65,38.05,-5.79),.075,GOLD,16,4)
            tube(lathe_clock,(0,36.8,-5.79),(-1.65,36.2,-5.79),.07,GOLD,16,4)
            panel.merge(lathe_clock)
        m.merge(turn(panel,angle,x,0,z))
    for dx in (-4.75,4.75):
        for dz in (-4.75,4.75):
            column(m,x+dx,z+dz,35.6,13.8,.6,TRIM,lod)
    # Open octagonal lantern and curved cap, rather than a solid cone.
    lathe(m,(x,z),[(52,4.6),(53,4.6)],TRIM,17,8,angle=math.pi/8)
    lathe(m,(x,z),[(53,2.7),(60,2.7)],STONE,11,8,angle=math.pi/8)
    for i in range(8):
        a=i*math.tau/8+math.pi/8
        column(m,x+3.5*math.cos(a),z+3.5*math.sin(a),53,7.7,.38,TRIM,lod)
    lathe(m,(x,z),[(60.4,4),(61.1,4),(61.4,3.4),(62.4,3.1),
          (64,2.45),(65,1.35),(65.5,.8)],LEAD,12,(32,20,12)[lod])
    lathe(m,(x,z),[(65.5,.8),(66.1,.8)],TRIM,17,12)
    m.ellipsoid((x,66.7,z),(.7,.7,.7),GOLD,16,12,6)
    lathe(m,(x,z),[(67.25,.20),(68.0,.04)],GOLD,16,8)


def _pauls(lod):
    m=Mesh()
    stone=(210,204,185)
    n=(64,40,24)[lod]
    # The origin is the surveyed centre of the crossing dome, +z points east.
    m.box((0,.5,-10),(42.0,2.4,143),stone)
    m.box((0,16,-10),(40.6,30.0,140),stone)
    gable(m,0,-10,15.4,140,32,36,stone,LEAD)
    m.box((0,15.6,0),(75,32.0,36),stone)
    lathe(m,(0,60.6),[(-.4,11.6),(31.5,11.6)],stone,11,n)
    for y in (1.5,15.2,29.2,30.4,31.5):
        m.box((0,y,-10),(41.4,.45,141.3),TRIM,17)
        m.box((0,y,0),(75.8,.45,36.8),TRIM,17)
        lathe(m,(0,60.6),[(y,11.95),(y+.35,11.95)],TRIM,17,n)
    for side in (-1,1):
        for end in (-1,1):
            m.box((side*22.2,15.6,end*21.3),(5.8,32.0,8.2),stone)
    # Repeated bays follow the nave and choir; crossing faces are kept clear.
    for angle in (-90,90):
        f=Mesh()
        for along in (-65,-56,-47,-38,-29,29,38,47,56):
            for y,h in [(4,8.5),(19,7.4)]:
                window(f,along,y,-20.36,3.4,h,lod,TRIM,False,True)
            for dx in (-3.4,3.4):
                f.box((along+dx,15.3,-20.63),(.8,28.0,.55),TRIM,17)
            if lod<2:
                stone_panel(f,along,13,-20.4,5.8,4.3,lod,stone)
        m.merge(turn(f,angle,0,0,-10))
    for z in (-18,18):
        f=Mesh()
        for x in np.linspace(-35,35,11):
            window(f,float(x),5,-.07,3.3,9,lod,TRIM,False,True)
            window(f,float(x),20,-.07,3.0,6.2,lod,TRIM,False,True)
        m.merge(turn(f,0 if z<0 else 180,0,0,z))
    for side in (-1,1):
        # Semicircular transept portico with open supporting columns.
        for i in range(6):
            a=-math.pi/2+i*math.pi/5
            xx=side*(37.5+5.1*math.cos(a));zz=5.1*math.sin(a)
            column(m,xx,zz,1.5,12.3,.60,TRIM,lod)
        for radius,y,h in [(5.7,-.4,1.0),(5.4,.6,.6),(5.1,1.2,.3),(5.7,13.8,.7)]:
            for a,b in zip(np.linspace(-math.pi/2,math.pi/2,n//2+1),np.linspace(-math.pi/2,math.pi/2,n//2+1)[1:]):
                p=(side*(37.5+radius*math.cos(a)),radius*math.sin(a))
                q=(side*(37.5+radius*math.cos(b)),radius*math.sin(b))
                face(m,[(side*37.5,y+h,0),(p[0],y+h,p[1]),(q[0],y+h,q[1])],TRIM,17,(0,1,0))
                face(m,[(p[0],y,p[1]),(q[0],y,q[1]),(q[0],y+h,q[1]),(p[0],y+h,p[1])],TRIM,17,(side*math.cos((a+b)/2),0,math.sin((a+b)/2)))
        f=Mesh()
        window(f,0,18,-37.56,5.2,8.4,lod,TRIM,False,True)
        for xx in (-12,-8,8,12):
            f.box((xx,16,-37.7),(.75,29.5,.6),TRIM,17)
        face(f,[(-8,29.4,-37.85),(8,29.4,-37.85),(0,34.4,-37.85)],TRIM,17,(0,0,-1))
        m.merge(turn(f,-90 if side>0 else 90))
    # Perimeter parapet has actual baluster gaps at the nearest two LODs.
    for x in (-20.7,20.7):
        for a,b in [(-79,-19),(19,60)]:
            balustrade(m,(x,a),(x,b),32.1,1.7,lod,TRIM)
    for z in (-18.6,18.6):
        for a,b in [(-37.5,-21),(21,37.5)]:
            balustrade(m,(max(a,-37.5),z),(min(b,37.5),z),32.1,1.7,lod,TRIM)
    # West front: two genuine column screens support separate entablatures.
    m.box((0,15.75,-82.5),(57.2,32.5,4.5),stone)
    m.box((0,-.5,-91),(58,1.0,12.01),stone)
    stairs(m,(0,-97.0),58,12,1.5,(12,8,5)[lod],TRIM)
    for x in (-11,-6,0,6,11):
        arch(m,(x,1.5,-85),4.0,12.3,.35,TRIM,False,(20,12,8)[lod],SHADOW,17)
    for x in (-14.2,-12.4,-9.2,-7.4,-4.2,-2.4,2.4,4.2,7.4,9.2,12.4,14.2):
        column(m,x,-88.2,1.5,15.8,.82,TRIM,lod)
    m.box((0,17.5,-87),(32.1,1.5,7.0),TRIM,17)
    for x in (-12.2,-10.2,-4.2,-2.2,2.2,4.2,10.2,12.2):
        column(m,x,-87.8,18.5,11.7,.62,TRIM,lod)
    m.box((0,30.5,-87),(31.5,1.1,6.2),TRIM,17)
    face(m,[(-16,31,-90.15),(16,31,-90.15),(0,39.5,-90.15)],TRIM,17,(0,0,-1))
    for a,b in [((-16,31,-90.4),(0,39.5,-90.4)),((0,39.5,-90.4),(16,31,-90.4))]:
        tube(m,a,b,.35,TRIM,17,6)
    for x,y in [(-16,31),(0,39.5),(16,31)]:
        _statue(m,x,y+.3,-87.8,3.6,lod)
    for x in (-20.72,20.09):
        _pauls_tower(m,x,-77.33,lod)
    # Poley's surveyed vertical stages, published by the City of London.
    lathe(m,(0,0),[(33.9,22.4),(36,22.4),(36.5,21.7),(42,21.7)],stone,11,n)
    lathe(m,(0,0),[(41.7,22),(42.3,22)],TRIM,17,n)
    lathe(m,(0,0),[(42,17.3),(56.5,17.3)],stone,11,n)
    for i in range(32):
        a=i*math.tau/32
        column(m,19.6*math.sin(a),19.6*math.cos(a),42.3,13.8,.62,TRIM,lod)
        f=Mesh()
        window(f,0,44,-17.36,1.8,7.8,lod,TRIM,False,True)
        m.merge(turn(f,math.degrees(a)))
    for y,radius,h in [(56.1,21.3,1.0),(57.2,21.8,.65),(58.1,22.1,.65)]:
        lathe(m,(0,0),[(y,radius),(y+h,radius)],TRIM,17,n)
    for i in range((96,64,32)[lod]):
        a=i*math.tau/(96,64,32)[lod]
        lathe(m,(21.45*math.cos(a),21.45*math.sin(a)),[(58.75,.14),(60.3,.14)],TRIM,17,6)
    lathe(m,(0,0),[(60.2,22),(60.5,22)],TRIM,17,n)
    lathe(m,(0,0),[(60.5,16.7),(65.5,16.7)],stone,11,n)
    for i in range(32):
        f=Mesh();window(f,0,61.5,-16.76,1.05,2.5,lod,TRIM,False,False)
        m.merge(turn(f,i*360/32))
    profile=[]
    for i in range((24,16,10)[lod]+1):
        t=i/(24,16,10)[lod]
        profile.append((65.5+19.3*t,max(.03,16.1*math.cos(t*math.pi/2))))
    lathe(m,(0,0),profile,lambda k,j: (89+(j%4),101+(j%4),104+(j%4)),12,n)
    for i in range(32):
        a=i*math.tau/32
        for (y0,r0),(y1,r1) in zip(profile,profile[1:]):
            if r1<3.9:
                break
            tube(m,((r0+.045)*math.cos(a),y0+.035,(r0+.045)*math.sin(a)),
                 ((r1+.045)*math.cos(a),y1+.035,(r1+.045)*math.sin(a)),.065,(122,130,128),12,4)
    lathe(m,(0,0),[(84.5,4.15),(86,4.15),(86,3.25),(96.3,3.25)],TRIM,17,n//2)
    for i in range(8):
        a=i*math.tau/8
        column(m,3.6*math.cos(a),3.6*math.sin(a),86,9.1,.33,TRIM,lod)
    lathe(m,(0,0),[(95.1,4.0),(96.2,4.0),(96.5,3.1),(99,2.7),
          (102,1.9),(104.5,.8),(106,.35)],TRIM,17,n//2)
    m.ellipsoid((0,106.8,0),(.9,.9,.9),GOLD,16,16,8)
    cross(m,(0,107.45,0),4.15,2.1,.22,GOLD)
    # Portland stone, lead sheets and gilt fittings use the same physical
    # materials as ordinary world architecture; carved trim stays smooth.
    return retag(m, {8:WEATHERED_LIMESTONE, 11:WEATHERED_LIMESTONE, 12:METAL_ROOF, 16:SCULPTURE_METAL})


from heritage_catalog import (
    sacred_milan, sacred_mosques, sacred_gothic, sacred_dohany, sacred_vatican,
)

MODULES = (sacred_mosques, sacred_gothic, sacred_dohany, sacred_vatican)
DEPENDENCIES = [Path(sacred_milan.__file__)]
for module in MODULES:
    DEPENDENCIES.append(Path(module.__file__))
    DEPENDENCIES.extend(getattr(module, 'DEPENDENCIES', []))
    directory = Path(__file__).parent/'references'/module.__name__.split('.')[-1]
    DEPENDENCIES.extend(p for p in directory.rglob('*')
                        if p.is_file() and '__pycache__' not in p.parts
                        and p.suffix not in ('.pyc', '.pyo'))

ENTRIES = [sacred_milan.ENTRY, {
    'name':'st-pauls-cathedral', 'city':'London',
    'lat':51.51378628946351, 'lon':-.09831113056309254,
    'heading':83.60227301952843, 'base':16.1,
    'suppress_buildings':True, 'suppress_roads':True, 'suppress_trees':True,
    'activation_pad':2.0,
    'source_exclusion_polygons':_PAULS_OWNERSHIP,
    'terrain_cuts':[{'polygon_xz':[[-29,-97],[29,-97],[29,-85],[-29,-85]],
                     'plane':[0,0,-.2],'feather_m':3}],
    'road_exclusion_polygons':[[[-29,-97],[29,-97],[29,-85],[-29,-85]]],
    'activation_bounds_xz':[np.concatenate(_PAULS_OWNERSHIP).min(0).tolist(),
                            np.concatenate(_PAULS_OWNERSHIP).max(0).tolist()],
    'description':'English Baroque cathedral with surveyed dome stages, paired western belfries, open peristyle and columned porticoes.',
    'references':[
        'https://www.cityoflondon.gov.uk/assets/Services-Environment/planning-protected-views-tall-buildings-st-pauls-heights-study-part-1-2015.pdf',
        'https://www.stpauls.co.uk/explore-our-map',
        'https://www.stpauls.co.uk/5-designs-for-dome-c-1685-1710',
        'https://www.openstreetmap.org/',
    ],
    'dimensions_m':{'overall_length_with_steps':169.2,'transept_wall_width':75,'width_including_porticoes':86.4,
        'cross_top':111.6,'dome_top':84.8,'dome_base':65.5,'peristyle_base':42,
        'drum_base':36,'nave_roof':36,'balustrade_top':33.9},
    'placement_evidence':{'source':'references/sacred/st-pauls-source.json',
        'anchor':'centre of mapped dome pieces; EN offset (+2.6983,-7.0883) from survey anchor',
        'heading':'perpendicular to line joining mapped western belfry centres',
        'width':'75m between transept walls; mapped semicircular porticoes extend the full plan to approximately86.4m',
        'base':'16.1m AOD, published top-of-cross AOD127.7 minus architectural height111.6; surrounding engine terrain16–17m'},
    'interpretive_details':'Window mouldings, figure silhouettes and stone-course coloration are simplified from exterior photographs; primary lengths and vertical stages follow the cited survey.',
    'capture':{'distance':260,'azimuth':215,'elevation':25},
}]


for module in MODULES:
    ENTRIES.extend(module.ENTRIES)
BUILDERS = {entry['name']: module.build for module in MODULES for entry in module.ENTRIES}


def build(name, lod):
    if name == 'milan-cathedral':
        return sacred_milan.build(lod)
    if name == 'st-pauls-cathedral':
        return _pauls(lod)
    if name in BUILDERS:
        return BUILDERS[name](name, lod)
    raise ValueError(name)
