"""Original metric geometry for a small, static catalog of city landmarks.

Dimensions are architectural envelopes, not survey-grade reconstructions. All
decoration is original geometry; source URLs establish scale and identity only.
"""
from __future__ import annotations

import math

from common import Mesh

TAU = math.tau
METAL = 16
STONE = 11
GLOW = 94


def entry(name, city, lat, lon, heading, description, references, distance,
          azimuth=155, elevation=25, suppress=True, activation_pad=0., **overrides):
    result = dict(name=name, city=city, lat=lat, lon=lon, heading=heading,
                base="terrain", suppress_buildings=suppress, activation_pad=activation_pad,
                suppress_roads=False, suppress_trees=suppress,
                description=description, references=references,
                capture=dict(distance=distance, azimuth=azimuth, elevation=elevation))
    result.update(overrides)
    return result


ENTRIES = [
    entry("singapore-supertrees", "Singapore", 1.28172, 103.86365, 0,
          "Twelve branching vertical gardens with violet crowns and a suspended skyway.",
          ["https://www.gardensbythebay.com.sg/en/things-to-do/attractions/supertree-grove.html"], 240, activation_pad=8.),
    entry("hollywood-sign", "Los Angeles", 34.13412, -118.32155, 0,
          "White hillside letter structures on individual steel supports.",
          ["https://www.hollywoodsign.org/history", "https://www.hollywoodsign.org/history-mission"],
          235, 180, 12, False),
    entry("dubai-frame", "Dubai", 25.23545595, 55.300367325, 21.51,
          "A monumental open gold frame with circular facade tracery and a sky bridge.",
          ["https://www.visitdubai.com/en/places-to-visit/dubai-frame",
           "https://www.openstreetmap.org/way/386649647"], 360, 220),
    entry("atomium", "Brussels", 50.89494, 4.34155, 15,
          "Nine steel spheres joined by twenty tubes in a body-centered cubic lattice.",
          ["https://www.brussels.be/atomium", "https://atomium.be/the_atomium_in_figures"], 235),
    entry("las-vegas-sphere", "Las Vegas", 36.12069, -115.16166, 0,
          "The monumental spherical screen carries one fixed original aurora color composition.",
          ["https://www.thesphere.com/science", "https://www.thesphere.com/studios-proxy/spherestudios/canvases"], 355),
    entry("fremont-canopy", "Las Vegas", 36.17070453, -115.14406563, 117.90565,
          "A long vaulted luminous canopy, steel arches and warm street-side marquee edges.",
          ["https://fremontstreetexperience.com/the-canopy/", "https://fremontstreetexperience.com/countdown-under-the-canopy/",
           "https://www.openstreetmap.org/way/1428180251"],
          260, 297.9, 2, False,
          suppress_buildings=True, suppress_bbox=[[-18.,-211.],[18.,211.]]),
]


def _ring(mesh, center, radius, thickness, color, material=METAL, segments=32,
          plane="xz"):
    def point(a,b):
        r=radius+thickness*math.cos(b)
        u,v,w=math.cos(a)*r,math.sin(a)*r,thickness*math.sin(b)
        delta=(u,w,v) if plane=="xz" else (u,v,-w)
        return tuple(center[i]+delta[i] for i in range(3))
    # Closed shared rings avoid hidden end caps at every tiny rail segment.
    for j in range(segments):
        a0,a1=TAU*j/segments,TAU*(j+1)/segments
        for k in range(6):
            b0,b1=TAU*k/6,TAU*(k+1)/6
            mesh.quad(point(a0,b0),point(a0,b1),point(a1,b1),point(a1,b0),color,material)


def _taper(mesh, cx, cz, levels, color, segments=16, material=STONE):
    for (y0, r0), (y1, r1) in zip(levels, levels[1:]):
        for i in range(segments):
            a, b = TAU*i/segments, TAU*(i+1)/segments
            p = [(cx+r*math.cos(t), y, cz+r*math.sin(t))
                 for y, r, t in [(y0,r0,a),(y1,r1,a),(y1,r1,b),(y0,r0,b)]]
            shade = tuple(max(0,min(255,int(v*(0.96+0.04*math.cos(a*3))))) for v in color)
            mesh.quad(*p, shade, material)
    y, r = levels[-1]
    for i in range(segments):
        a,b=TAU*i/segments,TAU*(i+1)/segments
        mesh.triangle((cx,y,cz),(cx+r*math.cos(b),y,cz+r*math.sin(b)),
                      (cx+r*math.cos(a),y,cz+r*math.sin(a)),color,material)


def _supertrees(lod):
    mesh = Mesh()
    # Unequal heights and irregular spacing preserve the grove's layered canopy.
    trees = [(-19,4,50,17), (27,19,42,15), (-54,-22,42,15),
             (8,-43,34,13), (54,-23,30,12), (-18,55,34,13),
             (-70,27,29,11), (60,43,28,11), (-47,63,25,10),
             (22,72,27,11), (80,8,26,10), (-42,-63,28,11)]
    ribs = (16,12,8)[lod]
    seg = (24,18,12)[lod]
    bark, steel = (77,87,59), (111,68,79)
    for index, (cx,cz,h,r) in enumerate(trees):
        _taper(mesh,cx,cz,[(0,4.3),(2,4.8),(h*.56,3.0),(h*.75,3.6),(h*.85,5.0)],bark,seg)
        _ring(mesh,(cx,1.1,cz),5,.23,(145,134,112),STONE,seg)
        _ring(mesh,(cx,h-.7,cz),r,.28,steel,METAL,seg)
        _ring(mesh,(cx,h-.48,cz),r,.14,(170,83,221),GLOW,seg)
        _ring(mesh,(cx,h-3.1,cz),r*.70,.23,steel,METAL,seg)
        _ring(mesh,(cx,h-3.0,cz),r*.70,.12,(75,163,173),GLOW,seg)
        for j in range(ribs):
            a = TAU*j/ribs+.17*index
            def p(rad,y,offset=0):
                return (cx+rad*math.cos(a+offset),y,cz+rad*math.sin(a+offset))
            stations = [p(4.2,1),p(3.5,h*.47),p(4.1,h*.71),p(r*.62,h-5),p(r,h)]
            for v,w in zip(stations,stations[1:]):
                mesh.beam(v,w,.30,steel,METAL,segments=6)
            mesh.beam(stations[-2],p(r,h,TAU/ribs*.48),.18,steel,METAL,segments=5)
            mesh.beam(stations[-2],p(r,h,-TAU/ribs*.48),.18,steel,METAL,segments=5)
            mesh.beam(p(4.2,h*.71),p(r*.62,h-4.9),.11,(160,74,204),GLOW,segments=5)
            # Clustered planting breaks up the trunk without hiding its taper.
            for k in range((5,3,2)[lod]):
                y = 4+(h*.66-4)*(k+.5)/(5,3,2)[lod]
                foliage=(54+((index+j+k)%3)*7,91+((j+k)%3)*8,54)
                mesh.ellipsoid(p(3.5,y), (1.15,2.6,1.15),foliage,STONE,
                               segments=(8,6,6)[lod],rings=4)
        if index == 0:
            _taper(mesh,cx,cz,[(h-2.4,7),(h-1.5,7),(h-.4,6.4)],(94,89,83),seg,METAL)
            _ring(mesh,(cx,h-.1,cz),6.4,.2,(238,199,131),GLOW,seg)
    # Curving suspended deck between two tall trees, 22 m above the garden.
    points=[]
    for i in range((36,24,16)[lod]+1):
        t=i/(36,24,16)[lod]
        points.append((-54+81*t,22, -22+41*t-20*math.sin(math.pi*t)))
    for a,b in zip(points,points[1:]):
        mesh.beam(a,b,1.1,(158,133,107),METAL,segments=6)
        for side in (-1,1):
            aa=(a[0],23.15,a[2]+side*1.15)
            bb=(b[0],23.15,b[2]+side*1.15)
            mesh.beam(aa,bb,.08,(229,185,111),GLOW,segments=5)
    for i in range(0,len(points),max(1,3-lod)):
        x,_,z=points[i]
        for side in (-1,1):
            mesh.beam((x,22,z+side*1.15),(x,23.2,z+side*1.15),.065,steel,METAL,segments=5)
    return mesh


def _stroke(mesh,a,b,width,color=(236,234,221)):
    dx,dy=b[0]-a[0],b[1]-a[1]
    length=math.hypot(dx,dy)
    px,py=-dy/length*width/2,dx/length*width/2
    verts=[(a[0]+px,a[1]+py,a[2]),(b[0]+px,b[1]+py,b[2]),
           (b[0]-px,b[1]-py,b[2]),(a[0]-px,a[1]-py,a[2])]
    mesh.quad(*verts,color,METAL)
    back=[(x,y,z+.28) for x,y,z in verts]
    mesh.quad(*reversed(back),(160,162,157),METAL)
    for i in range(4):
        mesh.quad(verts[i],back[i],back[(i+1)%4],verts[(i+1)%4],(194,195,185),METAL)


def _hollywood(lod):
    mesh=Mesh()
    letters="HOLLYWOOD"
    # Each glyph clears its highest sampled hillside footing by three metres;
    # a common baseline buries the western letters in the source DEM slope.
    baselines=[9.6,11.5,11.7,9.9,6.0,2.3,.5,-.5,1.7]
    widths=[9.3,10.0,8.7,8.7,9.5,12.7,10,10,9.7]
    gap=2.0
    x=-sum(widths)/2-gap*4
    for i,(letter,w) in enumerate(zip(letters,widths)):
        y=baselines[i]
        z=1.8*math.sin(i*.65)
        height=13.716
        def p(u,v):return (x+u*w,y+v*height,z)
        def line(a,b,thickness=1.75):_stroke(mesh,p(*a),p(*b),thickness)
        if letter == "H":
            line((.1,.065),(.1,.935));line((.9,.065),(.9,.935));line((.1,.51),(.9,.51))
        elif letter == "L":
            line((.1,.065),(.1,.935));line((.1,.065),(.91,.065))
        elif letter == "Y":
            line((.08,.94),(.5,.48));line((.92,.94),(.5,.48));line((.5,.48),(.5,.065))
        elif letter == "W":
            for a,b in zip([(.06,.94),(.27,.065),(.50,.59),(.73,.065)],
                           [(.27,.065),(.50,.59),(.73,.065),(.94,.94)]):line(a,b)
        else:
            if letter=="D":
                path=[(.1,.065),(.1,.935),(.57,.935),(.82,.84),(.92,.67),(.92,.33),(.82,.16),(.57,.065),(.1,.065)]
            else:
                path=[(.28,.065),(.12,.17),(.075,.35),(.075,.65),(.12,.83),(.28,.935),(.72,.935),(.88,.83),(.925,.65),(.925,.35),(.88,.17),(.72,.065),(.28,.065)]
            for a,b in zip(path,path[1:]):line(a,b)
        if lod<2:
            for u in (.16,.82):
                xx=x+w*u
                mesh.beam((xx,-5,z+1),(xx,y+height-.3,z+.55),.11,(105,110,103),METAL,segments=5)
                mesh.beam((xx,-3,z+5),(xx,y+height*.7,z+.6),.085,(105,110,103),METAL,segments=5)
            for v in (.25,.73):
                mesh.beam((x,y+height*v,z+.6),(x+w,y+height*v,z+.6),.1,(105,110,103),METAL,segments=5)
        x+=w+gap
    return mesh


def _frame(lod):
    mesh=Mesh()
    gold=(174,126,49); edge=(233,185,81); inset=(102,77,40)
    # Outer dimensions 93 x 150 m; the central void must remain genuinely open.
    for x in (-41,41):
        mesh.box((x,73,0),(11,146,13),gold,METAL)
        for xx in (x-5.2,x+5.2):
            for z in (-6.65,6.65):
                mesh.box((xx,75,z),(.35,150,.30),edge,GLOW)
        for z in (-6.52,6.52):
            mesh.box((x,72,z),(6.9,135,.18),(152,108,41),METAL)
            for k in range(1,19):
                y=k*7.6
                _ring(mesh,(x,y,z+math.copysign(.22,z)),3.28,.19,inset,METAL,(20,16,12)[lod],"xy")
                _ring(mesh,(x,y,z+math.copysign(.28,z)),2.50,.12,edge,METAL,(20,16,12)[lod],"xy")
                if lod==0:
                    for off in (-2.5,2.5):
                        _ring(mesh,(x+off,y+3.5,z+math.copysign(.24,z)),.62,.1,inset,METAL,10,"xy")
    mesh.box((0,144.5,0),(93,11,13),gold,METAL)
    mesh.box((0,6,0),(93,12,15),gold,METAL)
    for z in (-6.8,6.8):
        for y in (139.1,149.8):mesh.box((0,y,z),(93,.36,.3),edge,GLOW)
        mesh.box((0,143.8,z),(70,4.4,.22),(72,93,98),METAL)
        for x in range(-33,34,6):
            mesh.box((x,143.8,z+math.copysign(.20,z)),(.32,4.6,.20),edge,METAL)
        for x in range(-36,37,8):
            _ring(mesh,(x,7,z+math.copysign(1,z)),3.1,.20,inset,METAL,(20,16,12)[lod],"xy")
    mesh.box((0,1,0),(103,2,27),(159,150,130),STONE)
    mesh.box((0,138.75,0),(69,.2,5),(100,169,168),METAL)
    return mesh


def _atomium(lod):
    mesh=Mesh()
    side=84/math.sqrt(3)
    def transform(p):
        i,j,k=p
        return ((i-j)*side/math.sqrt(2),51+(i+j+k)*side/math.sqrt(3),
                (i+j-2*k)*side/math.sqrt(6))
    corners=[(i,j,k) for i in (-.5,.5) for j in (-.5,.5) for k in (-.5,.5)]
    pts=[transform(p) for p in corners]
    silver=(173,183,190); dark=(96,113,126)
    for i,a in enumerate(corners):
        for j,b in enumerate(corners[i+1:],i+1):
            if sum(a[k]!=b[k] for k in range(3))==1:
                mesh.beam(pts[i],pts[j],1.65,silver,METAL,segments=(12,10,8)[lod])
        mesh.beam((0,51,0),pts[i],1.65,silver,METAL,segments=(12,10,8)[lod])
    for p in pts+[(0,51,0)]:
        mesh.ellipsoid(p,(9,9,9),silver,METAL,segments=(32,24,16)[lod],rings=(16,12,8)[lod])
        x,y,z=p
        for h in (-3.0,3.0):
            _ring(mesh,(x,y+h,z),math.sqrt(81-h*h)+.025,.07,dark,METAL,(32,24,16)[lod])
        _ring(mesh,(x,y-.7,z),8.99,.10,(168,197,207),GLOW,(32,24,16)[lod])
        if lod==0:
            for j in range(12):
                a=TAU*j/12
                mesh.ellipsoid((x+9.07*math.cos(a),y+.6,z+9.07*math.sin(a)),
                               (.13,.13,.13),(216,224,221),GLOW,segments=6,rings=4)
    # The three lower corner spheres sit on the characteristic splayed bipods.
    for x,y,z in pts:
        if abs(y-37)<.1:
            for spread in (-5,5):
                mesh.beam((x+spread,0,z*1.15),(x,y-5,z),1.0,dark,METAL,segments=8)
                mesh.box((x+spread,.25,z*1.15),(4,.5,4),(139,142,137),STONE)
    _taper(mesh,0,0,[(0,12),(3,11),(4,9)],(129,137,140),(24,18,12)[lod],METAL)
    return mesh


def _sphere(lod):
    mesh=Mesh()
    n,rings=(96,64,40)[lod],(48,32,20)[lod]
    radius,cy=78.65,33.0
    low=math.asin(-cy/radius)
    def p(a,t):
        return (radius*math.cos(t)*math.cos(a),cy+radius*math.sin(t),radius*math.cos(t)*math.sin(a))
    bounds=[low,-.03,.14,.28,.48,.82,math.pi/2]
    palette=[(43,75,130),(99,73,150),(193,103,76),(214,151,76),
             (57,184,186),(39,150,171)]
    def wave(a,t):
        # Mesh rows follow the artwork's curved boundaries, so its broad bands
        # stay clean at every LOD instead of acquiring pixel-like jagged edges.
        fade=math.sin(math.pi*(t-low)/(math.pi/2-low))
        return t-(.24*math.sin(a*2)+.08*math.cos(a*3))*fade
    for band,(lower,upper) in enumerate(zip(bounds,bounds[1:])):
        rows=max(2,round(rings*(upper-lower)/(math.pi/2-low)))
        for j in range(rows):
            t0=lower+(upper-lower)*j/rows
            t1=lower+(upper-lower)*(j+1)/rows
            for i in range(n):
                a0,a1=TAU*i/n,TAU*(i+1)/n
                factor=.88+.12*math.cos((a0+a1)/2-.5)**2
                color=tuple(int(v*factor) for v in palette[band])
                if band==len(palette)-1 and j==rows-1:
                    mesh.triangle(p(a0,wave(a0,t0)),(0,cy+radius,0),p(a1,wave(a1,t0)),color,GLOW)
                else:
                    mesh.quad(p(a0,wave(a0,t0)),p(a0,wave(a0,t1)),
                              p(a1,wave(a1,t1)),p(a1,wave(a1,t0)),color,GLOW)
    _ring(mesh,(0,.75,0),math.sqrt(radius*radius-cy*cy),.55,(52,62,76),METAL,(96,64,40)[lod])
    mesh.box((0,5,-70),(45,10,14),(53,59,68),METAL)
    mesh.box((0,3,-78),(29,5,.4),(35,54,66),METAL)
    mesh.box((0,6.2,-78.4),(31,.3,.25),(118,174,181),GLOW)
    return mesh


def _fremont(lod):
    mesh=Mesh()
    length=420.0
    span=27.4
    segments=(20,16,12)[lod]
    bays=28
    def p(u,z,dy=0):
        return (u*span/2,20.5+6.9*math.sqrt(max(0,1-u*u))+dy,z)
    palette=[(91,73,147),(73,119,164),(65,166,159),(190,139,80),(169,79,123)]
    for j in range(bays):
        z0,z1=-length/2+length*j/bays,-length/2+length*(j+1)/bays
        for i in range(segments):
            u0,u1=-1+2*i/segments,-1+2*(i+1)/segments
            color=palette[(int(j/4)+int((u0+1)*1.5))%len(palette)]
            q=[p(u0,z0),p(u0,z1),p(u1,z1),p(u1,z0)]
            mesh.quad(*q,(100,109,119),METAL)
            underside=[(x,y-.1,z) for x,y,z in reversed(q)]
            mesh.quad(*underside,color,GLOW)
    steel=(120,124,131)
    for j in range(15):
        z=-length/2+length*j/14
        for i in range(segments):
            u0,u1=-1+2*i/segments,-1+2*(i+1)/segments
            mesh.beam(p(u0,z,.28),p(u1,z,.28),.30,steel,METAL,segments=6)
            if lod<2:
                mesh.beam(p(u0,z,-.4),p(u1,z,-.4),.11,(218,177,110),GLOW,segments=5)
        for side in (-1,1):
            x=side*(span/2+.8)
            mesh.beam((x,0,z),(x,20.5,z),.50,steel,METAL,segments=8)
            mesh.beam((x,15,z),p(side*.70,z),.24,steel,METAL,segments=6)
    for side in (-1,1):
        mesh.beam((side*span/2,20.5,-length/2),(side*span/2,20.5,length/2),.25,(213,159,87),GLOW,segments=6)
        # Compact projecting marquee shelves sit below the canopy edge.
        for j in range(7):
            z=-176+j*57
            mesh.box((side*16,7.3,z),(5,1.5,31),(143,100,68),METAL)
            mesh.box((side*13.35,7.1,z),(.3,.45,31),(236,165,74),GLOW)
            if lod==0:
                for dz in range(-14,15,2):
                    mesh.ellipsoid((side*13.12,7.65,z+dz),(.16,.16,.16),
                                   (235,192,113),GLOW,segments=6,rings=4)
    return mesh


def build(name, lod):
    if lod not in (0,1,2):
        raise ValueError(f"unsupported LOD {lod}")
    return {"singapore-supertrees":_supertrees, "hollywood-sign":_hollywood,
            "dubai-frame":_frame, "atomium":_atomium,
            "las-vegas-sphere":_sphere, "fremont-canopy":_fremont}[name](lod)
