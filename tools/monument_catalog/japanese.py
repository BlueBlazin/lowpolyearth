"""Metric Japanese gates; published dimensions and approximation limits in references/japanese."""
import math
import numpy as np
from city_catalog.common import Mesh
from heritage_catalog.geometry import face, lathe, tube, turn
from heritage_catalog.materials import (STONE_WALL, TIMBER_WALL, PLASTER_WALL,
    METAL_ROOF, TILE_ROOF, SHINGLE_ROOF, SCULPTURE_METAL, STEEL)

VERMILION=(190,49,27)
RED_SHADOW=(143,35,24)
RED_LIGHT=(210,64,37)
BLACK=(41,43,38)
GOLD=(190,145,54)
WHITE=(233,228,210)
COPPER=(66,102,84)
STONE=(130,132,123)


def _beam(m, width, y, depth, thickness, rise, color, material, lod, z=0):
    """Closed curved lintel with a shallow central span and upswept ends."""
    xs=np.linspace(-width/2,width/2,(24,16,8)[lod]+1)
    def yy(x):return y+rise*(abs(x)/(width/2))**3
    for a,b in zip(xs,xs[1:]):
        for side in (-1,1):
            face(m,[(a,yy(a),z+side*depth/2),(b,yy(b),z+side*depth/2),
                (b,yy(b)+thickness,z+side*depth/2),(a,yy(a)+thickness,z+side*depth/2)],color,material,(0,0,side))
        for offset,normal in [(0,(0,-1,0)),(thickness,(0,1,0))]:
            face(m,[(a,yy(a)+offset,z-depth/2),(b,yy(b)+offset,z-depth/2),
                (b,yy(b)+offset,z+depth/2),(a,yy(a)+offset,z+depth/2)],color,material,normal)
    for x in (-width/2,width/2):
        face(m,[(x,yy(x),z-depth/2),(x,yy(x),z+depth/2),
            (x,yy(x)+thickness,z+depth/2),(x,yy(x)+thickness,z-depth/2)],color,material,(x,0,0))


def _post(m,x,height,radius,color,material,lod,lean=0,stone=False):
    p=Mesh();n=(24,16,10)[lod]
    colors=lambda k,j:tuple(max(0,min(255,c+(-3,0,2,1)[j%4])) for c in color)
    profile=[(-.5,radius*1.13),(0,radius*1.13),(.18,radius),
             (height*.34,radius*.95),(height*.68,radius*.90),(height,radius*.84)]
    lathe(p,(0,0),profile,colors,material,n)
    matrix=np.eye(4);matrix[0,1]=lean/height;matrix[0,3]=x;p.transform(matrix);m.merge(p)
    if stone:
        for y in (height*.34,height*.68):
            lathe(m,(x+lean*y/height,0),[(y-.015,radius*(1-.16*y/height)+.006),(y+.015,radius*(1-.16*y/height)+.006)],(102,105,99),material,n)
    else:
        lathe(m,(x,0),[(-.4,radius*1.17),(.1,radius*1.17),(.22,radius*1.06)],STONE,STONE_WALL,n)



def _foundation_box(m,x,width,depth,bottom,top,color):
    """Keep metre-spaced toe vertices along long piers for local ground fitting."""
    corners=np.array([[-width/2,-depth/2],[width/2,-depth/2],
                      [width/2,depth/2],[-width/2,depth/2]],float)
    ring=[]
    for a,b in zip(corners,np.roll(corners,-1,axis=0)):
        segments=max(1,math.ceil(np.linalg.norm(b-a)))
        ring.extend(a+(b-a)*t/segments for t in range(segments))
    for a,b in zip(ring,ring[1:]+ring[:1]):
        edge=b-a
        face(m,[(x+a[0],bottom,a[1]),(x+b[0],bottom,b[1]),
                (x+b[0],top,b[1]),(x+a[0],top,a[1])],color,STONE_WALL,(edge[1],0,-edge[0]))
        for y,normal in [(bottom,(0,-1,0)),(top,(0,1,0))]:
            face(m,[(x,y,0),(x+a[0],y,a[1]),(x+b[0],y,b[1])],color,STONE_WALL,normal)


def _crest(m,x,y,z,r,lod,color=GOLD):
    if lod==2:return
    for j in range(16 if lod==0 else 8):
        a=math.tau*j/(16 if lod==0 else 8)
        face(m,[(x,y,z-.03),(x+r*math.cos(a-.1),y+r*math.sin(a-.1),z),
                (x+r*1.13*math.cos(a),y+r*1.13*math.sin(a),z-.04),
                (x+r*math.cos(a+.1),y+r*math.sin(a+.1),z)],color,SCULPTURE_METAL,(0,0,-1))
    m.ellipsoid((x,y,z-.04),(r*.24,r*.24,.055),color,SCULPTURE_METAL,8,3)


def _torii(lod,w,h,span,radius,color,material,cap,plaque=True,stone=False,collar=False):
    m=Mesh();top=h*.88;rise=h*.055;thick=h*.065
    for side in (-1,1):
        x=side*span/2;lean=-side*h*.033
        _post(m,x,top,radius,color,material,lod,lean,stone)
        if collar:
            lathe(m,(x,0),[(0,radius*1.07),(h*.115,radius*1.07)],BLACK,STEEL,(24,16,10)[lod])
            lathe(m,(x,0),[(h*.115,radius*1.11),(h*.13,radius*1.11)],GOLD,SCULPTURE_METAL,(24,16,10)[lod])
    _beam(m,w*.97,top,w*.041,thick*.58,rise,color,material,lod)
    _beam(m,w,top+thick*.58,w*.048,thick*.42,rise,cap,material,lod)
    tie_w=w if stone else w*.80
    m.box((0,h*.68,0),(tie_w,h*.055,w*.034),color,material)
    if not stone:
        for side in (-1,1):
            m.box((side*(span/2-h*.023),h*.681,0),(w*.032,h*.084,w*.045),color,material)
    m.box((0,h*.79,0),(w*.036,h*.20,w*.027),color,material)
    if plaque:
        m.box((0,h*.773,-w*.032),(w*.079,h*.139,w*.018),GOLD if collar else color,SCULPTURE_METAL if collar else material)
        m.box((0,h*.773,-w*.042),(w*.061,h*.12,w*.003),BLACK if collar else tuple(max(0,v-30) for v in color),material)
    return m


def _heian(lod):
    m=_torii(lod,33.9,24.4,18.18,1.818,VERMILION,PLASTER_WALL,(61,63,56),False)
    # The concrete monument has heavy stone collars and a green-black crown.
    for s in (-1,1):
        lathe(m,(s*9.09,0),[(-.5,2.10),(.15,2.10),(.5,1.85)],(121,125,119),STONE_WALL,(24,16,10)[lod])
    return m


def _meiji(lod):
    m=_torii(lod,17.1,12,10.7,.6,(135,108,74),TIMBER_WALL,(109,91,65),False)
    for x in (-5.7,0,5.7):_crest(m,x,11.38,-.425,.28,lod)
    return m


def _oyunohara(lod):
    m=_torii(lod,42,33.9,26.0,1.75,(49,48,42),STEEL,(35,38,33),False)
    for x in (-12.9,12.9):
        lathe(m,(x,0),[(-.5,2.15),(.06,2.15),(.28,1.85)],(144,143,130),STONE_WALL,(24,16,10)[lod])
    return m


def _kirishima(lod):
    m=_torii(lod,28,22.4,16,1.25,(199,51,30),STEEL,BLACK,False)
    for x in (-8,8):lathe(m,(x,0),[(-.5,1.42),(.9,1.42)],(202,200,186),STONE_WALL,(24,16,10)[lod])
    _crest(m,0,20.55,-.71,.43,lod)
    if lod<2:
        for x in np.linspace(-13.8,13.8,(81,41)[lod]):
            yy=22.4*.88+22.4*.065+22.4*.055*(abs(x)/14)**3
            tube(m,(x,yy,-.67),(x,yy,.67),.035,(67,70,66),TILE_ROOF,4)
    return m


def _hakozaki(lod):
    return _torii(lod,6.3,4.75,4.224,.4895,(111,115,106),STONE_WALL,(121,124,114),True,stone=True)


def _mitsumine(lod):
    m=_torii(lod,5.2,5.8,3.8,.28,(214,215,203),STONE_WALL,(224,224,213),True,stone=True)
    # The flanking torii share the main inner uprights; only outer posts descend.
    for side in (-1,1):
        sub=Mesh();_post(sub,side*3.8,3.6,.22,(211,212,201),STONE_WALL,lod,-side*.12,True)
        part=Mesh();_beam(part,2.6,3.3,.28,.25,.16,(220,220,208),STONE_WALL,lod)
        part.box((0,2.65,0),(2.45,.22,.24),(205,208,198),STONE_WALL)
        part.box((0,3.02,0),(.18,.62,.22),(211,212,201),STONE_WALL)
        m.merge(turn(part,0,side*2.85,0,0));m.merge(sub)
    if lod<2:
        # Shimenawa sags below the main nuki; folded white shide hang from it.
        xs=np.linspace(-1.6,1.6,13)
        for a,b in zip(xs,xs[1:]):tube(m,(a,3.25+.23*(a/1.6)**2,-.20),(b,3.25+.23*(b/1.6)**2,-.20),.035,(164,141,91),TIMBER_WALL,6)
        for x in (-1.1,-.4,.4,1.1):
            for j in range(3):
                m.box((x+(j%2)*.085,3.1-j*.12,-.22),(.14,.14,.025),WHITE,PLASTER_WALL)
    return m


def _roof(m,w,d,y,rise,lod,color,material,hip=True,irimoya=False):
    # Concave hipped eaves meet a short gable ridge rather than a pyramidal cap.
    n=(12,8,4)[lod];levels=np.linspace(0,1,n+1)
    ridge=w*.60
    def section(t):
        return (w-(w-ridge)*min(1,t*2) if irimoya else w-(w-ridge)*t if hip else w), d*(1-t), y+rise*t**1.5
    for a,b in zip(levels,levels[1:]):
        wa,da,ya=section(a);wb,db,yb=section(b)
        tint=tuple(min(255,c+(2 if int(a*n)%2 else -2)) for c in color)
        for side in (-1,1):
            face(m,[(-wa/2,ya,side*da/2),(wa/2,ya,side*da/2),(wb/2,yb,side*db/2),(-wb/2,yb,side*db/2)],tint,material,(0,1,side))
            if hip and not (irimoya and a>=.5):
                face(m,[(side*wa/2,ya,-da/2),(side*wa/2,ya,da/2),(side*wb/2,yb,db/2),(side*wb/2,yb,-db/2)],tint,material,(side,1,0))
    if irimoya or not hip:
        for side in (-1,1):
            gx=side*(ridge/2 if irimoya else w/2)
            base_y=y+rise*.5**1.5 if irimoya else y
            gz=d*.25 if irimoya else d*.5
            face(m,[(gx,base_y,-gz),(gx,base_y,gz),(gx,y+rise,0)],WHITE,PLASTER_WALL,(side,0,0))
            for z in (-gz,gz):tube(m,(gx,base_y,z),(gx,y+rise,0),.10,VERMILION,TIMBER_WALL,4)
    m.box((0,y+rise+.09,0),((ridge if hip else w)+.45,.22,.30),BLACK,material)
    for side in (-1,1):
        m.box((0,y-.10,side*d/2),(w,.23,.22),RED_SHADOW,TIMBER_WALL)
    if lod<2:
        for x in np.linspace(-w/2+.2,w/2-.2,int(w/(.40 if lod==0 else .8))):
            for side in (-1,1):
                tube(m,(x,y-.18,side*d/2),(x*.87,y+.20,side*d*.40),.04,(211,129,73),TIMBER_WALL,4)
        for side in (-1,1):
            tube(m,(side*ridge/2,y+rise,0),(side*(ridge/2+.23),y+rise+.45,0),.09,BLACK,material,6)


def _rail(m,w,d,y,lod):
    for s in (-1,1):
        for yy in (y+.12,y+.67):m.box((0,yy,s*d/2),(w,.12,.13),VERMILION,TIMBER_WALL)
        for x in np.linspace(-w/2,w/2,(25,17,9)[lod]):m.box((x,y+.4,s*d/2),(.095,.78,.095),RED_LIGHT,TIMBER_WALL)
        for yy in (y+.12,y+.67):m.box((s*w/2,yy,0),(.13,.12,d),VERMILION,TIMBER_WALL)
        for z in np.linspace(-d/2,d/2,(9,7,5)[lod]):m.box((s*w/2,y+.4,z),(.095,.78,.095),RED_LIGHT,TIMBER_WALL)


def _romon(lod,w,d,h,ornate=False,takeo=False):
    m=Mesh();colx=[-w*.36,-w*.14,w*.14,w*.36];colz=d*.26
    for x in colx:
        for z in (-colz,colz):
            lathe(m,(x,z),[(-.5,.38),(.12,.38),(.22,.30),(h*.44,.30)],VERMILION,TIMBER_WALL,(16,12,8)[lod])
            lathe(m,(x,z),[(-.5,.43),(.08,.43)],STONE,STONE_WALL,(16,12,8)[lod])
    for side in (-1,1):
        x=side*w*.25
        m.box((x,h*.20,0),(w*.19,h*.37,d*.50),RED_SHADOW,TIMBER_WALL)
        for z in (-d*.26,d*.26):
            m.box((x,h*.23,z),(w*.145,h*.27,.12),BLACK,TIMBER_WALL)
            if lod<2:
                # Recessed guardian silhouettes behind the protective lattice.
                m.ellipsoid((x,h*.18,z-.10),(w*.042,h*.08,.2),(89,76,55),TIMBER_WALL,8,4)
                m.ellipsoid((x,h*.275,z-.10),(.20,.25,.18),(151,125,83),TIMBER_WALL,8,4)
                for xx in np.linspace(x-w*.07,x+w*.07,9 if lod==0 else 5):m.box((xx,h*.235,z-.22),(.045,h*.25,.06),BLACK if not ornate else VERMILION,TIMBER_WALL)
                if not ornate:
                    for xx in np.linspace(x-w*.07,x+w*.07,9 if lod==0 else 5):
                        m.box((xx,.8,z-.29),(.09,1.05,.08),(84,157,132),TIMBER_WALL)
    m.box((0,h*.42,0),(w*.83,.35,d*.64),VERMILION,TIMBER_WALL)
    _rail(m,w*.82,d*.68,h*.44,lod)
    upper_y=h*.455;upper_h=h*.27
    m.box((0,upper_y+upper_h/2,0),(w*.66,upper_h,d*.46),(190,154,94) if ornate else (185,151,83),PLASTER_WALL)
    for x in (-w*.33,-w*.11,w*.11,w*.33):
        for z in (-d*.24,d*.24):m.box((x,upper_y+upper_h/2,z),(.22,upper_h,.20),VERMILION,TIMBER_WALL)
    for side in (-1,1):
        z=side*d*.245
        for yy in (upper_y,upper_y+upper_h*.8):m.box((0,yy,z),(w*.70,.20,.25),VERMILION,TIMBER_WALL)
        if not takeo:
            for x in (-w*.22,0,w*.22):
                m.box((x,upper_y+upper_h*.48,z),(w*.12,upper_h*.62,.10),(68,78,63),TIMBER_WALL)
                for xx in np.linspace(x-w*.06,x+w*.06,8 if lod==0 else 4):m.box((xx,upper_y+upper_h*.48,z+side*.065),(.045,upper_h*.63,.09),RED_LIGHT,TIMBER_WALL)
        else:
            # Takeo's distinctive paired round-headed upper windows.
            for x in (-w*.16,w*.16):
                ww=w*.095;yy=upper_y+upper_h*.24
                m.box((x,yy+upper_h*.19,z+side*.055),(ww*2,upper_h*.38,.10),BLACK,TIMBER_WALL)
                for j in range((12,8,6)[lod]):
                    a=math.pi*j/(12,8,6)[lod];b=math.pi*(j+1)/(12,8,6)[lod]
                    face(m,[(x,yy+upper_h*.38,z+side*.11),(x+ww*math.cos(a),yy+upper_h*.38+ww*math.sin(a),z+side*.11),(x+ww*math.cos(b),yy+upper_h*.38+ww*math.sin(b),z+side*.11)],BLACK,TIMBER_WALL,(0,0,side))
        if lod<2:
            palette=[(54,106,134),(103,135,73),(198,131,65),GOLD]
            for j,x in enumerate(np.linspace(-w*.30,w*.30,17 if lod==0 else 9)):
                m.box((x,upper_y+upper_h*.91,z+side*.12),(w*.029,.23,.13),palette[j%4],TIMBER_WALL)
                _crest(m,x,upper_y-.16,z+side*.20,.115,lod,GOLD)
    _roof(m,w,d,h*.72,h*.245,lod,(85,91,85) if ornate else (117,120,118),METAL_ROOF if ornate else TILE_ROOF,irimoya=not takeo)
    if takeo:_roof(m,w*1.06,d*1.08,h*.425,h*.105,lod,(76,80,70),TILE_ROOF)
    for x in colx:
        for z in (-colz,colz):
            for i in range(3 if lod<2 else 1):
                m.box((x,h*.39+i*.17,z),(1.0+i*.12,.15,.65+i*.15),RED_LIGHT,TIMBER_WALL)
    for side in (-1,1):
        z=side*d*.28
        m.box((0,h*.675,z),(w*.081,h*.15,.18),GOLD,SCULPTURE_METAL)
        m.box((0,h*.675,z+side*.12),(w*.065,h*.137,.06),BLACK,TIMBER_WALL)
    # No solid plinth crosses the pass-through aperture.
    return m


def _nezu(lod):return _romon(lod,14.2,10.6,11.5)
def _yutoku(lod):
    m=_romon(lod,15.8,12.0,15.5,ornate=True)
    for side in (-1,1):
        z=side*3.2
        m.box((0,5.73,z),(12.1,1.12,.40),(124,66,33),TIMBER_WALL)
        for j,x in enumerate(np.linspace(-5.3,5.3,(13,9,5)[lod])):
            m.box((x,5.75,z+side*.25),(.69,.69,.12),(183,178,124),TIMBER_WALL)
            if lod<2:
                for k in range(5):
                    a=math.tau*k/5
                    m.ellipsoid((x+.21*math.cos(a),5.75+.23*math.sin(a),z+side*.34),(.13,.11,.07),[(78,126,149),(93,143,113),(219,180,92)][(j+k)%3],TIMBER_WALL,6,3)
        for x in (-3.95,3.95):
            for xx in np.linspace(x-.9,x+.9,(13,9,5)[lod]):
                m.box((xx,1.15,z+side*.08),(.10,1.9,.12),(72,145,125),TIMBER_WALL)
    return m
def _takeo(lod):
    m=Mesh();w=8.8;d=5.8;top=5.65;opening=4.1
    # White plaster piers and rounded-corner lintel form the actual tunnel.
    for side in (-1,1):
        m.box((side*(w+opening)/4,(top+.6)/2,0),((w-opening)/2,top-.6,d),WHITE,PLASTER_WALL)
        _foundation_box(m,side*(w+opening)/4,(w-opening)/2,d+.12,-.5,.6,(138,132,114))
    xs=np.linspace(-opening/2,opening/2,(24,16,8)[lod]+1)
    def arch_y(x):return 4.3 if abs(x)<=1.35 else 3.6+math.sqrt(max(0,.7**2-(abs(x)-1.35)**2))
    for a,b in zip(xs,xs[1:]):
        for side in (-1,1):
            face(m,[(a,arch_y(a),side*d/2),(b,arch_y(b),side*d/2),(b,top,side*d/2),(a,top,side*d/2)],WHITE,PLASTER_WALL,(0,0,side))
        face(m,[(a,arch_y(a),-d/2),(b,arch_y(b),-d/2),(b,arch_y(b),d/2),(a,arch_y(a),d/2)],(196,190,173),PLASTER_WALL,(0,-1,0))
    m.box((0,top-.05,0),(w,.2,d),WHITE,PLASTER_WALL)
    for side in (-1,1):
        for x in (-3.25,3.25):
            z=side*(d/2+.025)
            m.box((x,1.7,z),(.47,1.5,.1),VERMILION,TIMBER_WALL)
            m.box((x,1.73,z+side*.07),(.33,1.27,.04),(73,92,84),TIMBER_WALL)
            m.box((x,1.30,z+side*.10),(.37,.07,.06),VERMILION,TIMBER_WALL)
    m.box((0,5.97,0),(9.5,.35,6.5),VERMILION,TIMBER_WALL)
    _rail(m,9.5,6.5,6.15,lod)
    m.box((0,7.65,0),(8.0,2.8,4.4),WHITE,PLASTER_WALL)
    for side in (-1,1):
        z=side*2.24
        for x in (-3.9,-1.4,1.4,3.9):m.box((x,7.68,z),(.22,2.9,.23),VERMILION,TIMBER_WALL)
        for x in (-2.65,0,2.65):
            m.box((x,7.45,z+side*.06),(1.8,1.65,.1),(82,111,111),TIMBER_WALL)
            for xx in np.linspace(x-.84,x+.84,(7,5,3)[lod]):m.box((xx,7.45,z+side*.13),(.075,1.68,.06),VERMILION,TIMBER_WALL)
        for yy in (6.6,8.4,9.0):m.box((0,yy,z),(8.2,.17,.25),VERMILION,TIMBER_WALL)
        m.box((0,8.91,z+side*.21),(2.6,.94,.14),(95,75,66),TIMBER_WALL)
        m.box((0,8.91,z+side*.30),(2.36,.72,.05),(63,45,33),TIMBER_WALL)
        for x in np.linspace(-4.3,4.3,(13,9,5)[lod]):
            for yy in (5.55,8.92):
                m.box((x,yy,z),(.32,.55,.7),VERMILION,TIMBER_WALL)
                m.box((x,yy+.2,z+side*.35),(.28,.16,.15),GOLD,SCULPTURE_METAL)
    _roof(m,12.0,8.2,9.36,2.63,lod,(124,127,124),TILE_ROOF,irimoya=True)
    # Gray ceramic ridge shachi preserve the Tenpyo roof silhouette.
    for side in (-1,1):
        x=side*3.55
        lathe(m,(x,0),[(12,.13),(12.17,.20),(12.36,.10),(12.5,.18)],(117,121,118),TILE_ROOF,(10,8,6)[lod])
    return m


def _usa(lod):
    m=Mesh()
    for x in (-2.25,2.25):
        for z in (-1.1,1.1):
            lathe(m,(x,z),[(-.5,.38),(.12,.38),(.25,.30),(5.7,.30)],VERMILION,TIMBER_WALL,(16,12,8)[lod])
            lathe(m,(x,z),[(-.5,.43),(.1,.43)],STONE,STONE_WALL,(16,12,8)[lod])
    for z in (-1.1,1.1):
        m.box((0,4.7,z),(5.7,.35,.40),VERMILION,TIMBER_WALL)
        m.box((0,5.5,z),(6.1,.30,.48),RED_LIGHT,TIMBER_WALL)
    _roof(m,7.8,5.2,5.9,2.7,lod,(86,74,54),SHINGLE_ROOF,hip=False)
    # Front and rear cusped karahafu canopies distinguish the west gate.
    for side in (-1,1):
        n=(24,16,8)[lod];xs=np.linspace(-2.8,2.8,n+1)
        def cy(x):return 6.0+1.18*math.exp(-(x/1.18)**2)-.13*math.cos(x*2.0)
        for a,b in zip(xs,xs[1:]):
            face(m,[(a,cy(a),side*2.9),(b,cy(b),side*2.9),(b,cy(b)+.27,side*2.9),(a,cy(a)+.27,side*2.9)],WHITE,TIMBER_WALL,(0,0,side))
            face(m,[(a,cy(a)+.27,side*2.9),(b,cy(b)+.27,side*2.9),(b,cy(b)+.45,side*.7),(a,cy(a)+.45,side*.7)],(84,72,51),SHINGLE_ROOF)
        for x in (-2.0,2.0):m.box((x,5.5,side*1.9),(.22,.35,2),VERMILION,TIMBER_WALL)
    for side in (-1,1):
        x=side*4.6
        for dx in (-1.0,1.0):m.box((x+dx,1.9,0),(.22,4.8,.25),VERMILION,TIMBER_WALL)
        m.box((x,2.8,0),(2.2,.25,.28),VERMILION,TIMBER_WALL)
        sub=Mesh();_roof(sub,2.8,2.5,3.3,.9,lod,(87,72,54),SHINGLE_ROOF);m.merge(turn(sub,0,x,0,0))
    return m


BUILDERS={'heian-jingu-great-torii':_heian,'meiji-jingu-second-torii':_meiji,
    'oyunohara-great-torii':_oyunohara,'kirishima-jingu-great-torii':_kirishima,
    'hakozaki-first-torii':_hakozaki,'mitsumine-mitsutorii':_mitsumine,
    'nezu-jinja-romon':_nezu,'yutoku-inari-romon':_yutoku,
    'takeo-onsen-romon':_takeo,'usa-jingu-saidaimon':_usa}

def build(name,lod):return BUILDERS[name](lod)


ENTRIES = [{'name': 'heian-jingu-great-torii',
  'city': 'Kyoto',
  'lat': 35.012566275,
  'lon': 135.78268225,
  'heading': 356.02,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.heianjingu.or.jp/print/heianjingu.pdf',
                 'https://www.ktc.or.jp/wp-content/uploads/2017/03/a3b57d0bfd01e985c689a2b12ad8bf51.pdf'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Published height 24.4m, lintel approximately34m, shaft diameter3.636m and centers18.18m; '
                 'reinforced concrete vermilion finish. Profile and cap depth estimated. OSM polygon '
                 'supplies center and long-axis alignment; its height tag is not used.',
  'source_exclusion_polygons': [[[-16.95, -2.15], [16.95, -2.15], [16.95, 2.15], [-16.95, 2.15]]],
  'max_source_building_span_m': 58.091763615851775,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-3.15, 3.15]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/heian-jingu-great-torii.md',
            'placement_file': 'tools/monument_catalog/references/japanese/heian-placement.json',
            'architectural_dimensions_m': [33.9, 24.4, 4.3],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/heian-jingu-great-torii.json'},
  'capture': {'distance': 67.8,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 10.98,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-16.95, -2.15], [16.95, 2.15]]},
 {'name': 'meiji-jingu-second-torii',
  'city': 'Tokyo',
  'lat': 35.6740966,
  'lon': 139.70130855,
  'heading': 266.26,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.meijijingu.or.jp/en/map/',
                 'https://www.mlit.go.jp/tagengo-db/common/001559485.pdf'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Shrine publishes height12m, width17.1m and pillar diameter1.2m; unpainted Japanese '
                 'cypress, curved myojin lintel, three imperial chrysanthemum crests. OSM explicitly '
                 'identifies the second/Otorii and its width. Remaining joinery proportions estimated from '
                 'reference photographs.',
  'source_exclusion_polygons': [[[-8.55, -0.75], [8.55, -0.75], [8.55, 0.75], [-8.55, 0.75]]],
  'max_source_building_span_m': 29.181627781876735,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-1.75, 1.75]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/meiji-jingu-second-torii.md',
            'placement_file': 'tools/monument_catalog/references/japanese/meiji-placement.json',
            'architectural_dimensions_m': [17.1, 12, 1.5],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/meiji-jingu-second-torii.json'},
  'capture': {'distance': 34.2,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 5.4,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-8.55, -0.75], [8.55, 0.75]]},
 {'name': 'oyunohara-great-torii',
  'city': 'Tanabe',
  'lat': 33.8358362,
  'lon': 135.77555985,
  'heading': 186.475,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.tb-kumano.jp/en/kumano-kodo/world-heritage/kumano-hongu-taisha/',
                 'https://www.hongutaisha.jp/english/'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Tanabe tourism authority publishes33.9m height,42m width and steel construction; black '
                 'coating. OSM42m line supplies center and alignment. Its concrete tag conflicts with the '
                 'architectural authority and is deliberately ignored. Member diameters and cap profile '
                 'estimated.',
  'source_exclusion_polygons': [[[-21.0, -2.2], [21.0, -2.2], [21.0, 2.2], [-21.0, 2.2]]],
  'max_source_building_span_m': 71.7907403499922,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-3.2, 3.2]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/oyunohara-great-torii.md',
            'placement_file': 'tools/monument_catalog/references/japanese/oyunohara-placement.json',
            'architectural_dimensions_m': [42, 33.9, 4.4],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/oyunohara-great-torii.json'},
  'capture': {'distance': 84.0,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 15.254999999999999,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-21.0, -2.2], [21.0, 2.2]]},
 {'name': 'kirishima-jingu-great-torii',
  'city': 'Kirishima',
  'lat': 31.852984,
  'lon': 130.869394,
  'heading': 40.0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://kirishimajingu.or.jp/precincts/',
                 'https://kirishima-sjc.or.jp/guide/d-otorii.html'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Local guide publishes22.4m height,28m lintel and16m pillar interval. Shrine photographs '
                 'show vermilion shafts, white footing collars, black tiled cap and one gold chrysanthemum '
                 'crest, without a hanging plaque. Diameter2.5m and detailed profile estimated. Center '
                 'inferred on mapped approach approximately16m northeast of Route223 junction; heading40deg '
                 'follows that road. Placement uncertainty approximately10m; no individually mapped torii '
                 'was found.',
  'source_exclusion_polygons': [[[-14.0, -1.5], [14.0, -1.5], [14.0, 1.5], [-14.0, 1.5]]],
  'max_source_building_span_m': 47.87243465711766,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-2.5, 2.5]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/kirishima-jingu-great-torii.md',
            'placement_file': 'tools/monument_catalog/references/japanese/kirishima-placement.json',
            'architectural_dimensions_m': [28, 22.4, 3.0],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/kirishima-jingu-great-torii.json'},
  'capture': {'distance': 56.0,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 10.08,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-14.0, -1.5], [14.0, 1.5]]},
 {'name': 'hakozaki-first-torii',
  'city': 'Fukuoka',
  'lat': 33.615009,
  'lon': 130.4223162,
  'heading': 111.0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://hakozakigu.or.jp/keidai/',
                 'https://ritsumei.repo.nii.ac.jp/record/9377/files/dmuch_13_nakajima2.pdf'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Study lists4.750m height and0.979m shaft measure; shrine identifies three-piece shafts, '
                 'unified kasagi/shimagi, and equally long nuki and lintel. Dark weathered Takashima stone '
                 'follows Fukuoka city cultural record. Width6.3m and4.224m centers are model '
                 'interpretation, not a survey. First torii node at inner approach, heading perpendicular to '
                 'OSM path. This is not the concrete great gate demolished in2018.',
  'source_exclusion_polygons': [[[-3.15, -0.6], [3.15, -0.6], [3.15, 0.6], [-3.15, 0.6]]],
  'max_source_building_span_m': 10.902554746480295,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-1.6, 1.6]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/hakozaki-first-torii.md',
            'placement_file': 'tools/monument_catalog/references/japanese/hakozaki-placement.json',
            'architectural_dimensions_m': [6.3, 4.75, 1.2],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/hakozaki-first-torii.json'},
  'capture': {'distance': 12.6,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 2.1375,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-3.15, -0.6], [3.15, 0.6]]},
 {'name': 'mitsumine-mitsutorii',
  'city': 'Chichibu',
  'lat': 35.9232617,
  'lon': 138.93237745,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.mitsuminejinja.or.jp/keidai/',
                 'https://www.mitsuminejinja.or.jp/info/2023/09/633/'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Official precinct map places the triple torii southeast of museum and south of Zuishinmon; '
                 'this distinguishes it from the torii beside eastern lookout. Pale stone triple '
                 'construction and shimenawa are visual interpretation. Overall8.3x5.8m dimensions are '
                 'estimated; mapped17m gate line is a placement guide rather than a measured architectural '
                 'span. No survey-grade dimensions found.',
  'source_exclusion_polygons': [[[-4.15, -0.35], [4.15, -0.35], [4.15, 0.35], [-4.15, 0.35]]],
  'max_source_building_span_m': 14.160091807611982,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-1.35, 1.35]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/mitsumine-mitsutorii.md',
            'placement_file': 'tools/monument_catalog/references/japanese/mitsumine-placement.json',
            'architectural_dimensions_m': [8.3, 5.8, 0.7],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled. A 1.10m surveyed component shift '
                              'above the center datum keeps the uphill auxiliary passage about 2.07m clear '
                              'across a 0.6m walking band while allowing the existing uphill lower shaft to '
                              'embed about 0.73m. This reduces the longest exposed downhill footing without '
                              'moving the upper lintels relative to one another.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/mitsumine-mitsutorii.json'},
  'capture': {'distance': 16.6,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 2.61,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-4.15, -0.35], [4.15, 0.35]],
  'surveyed_component_shifts_m': [1.1]},
 {'name': 'nezu-jinja-romon',
  'city': 'Tokyo',
  'lat': 35.71966199999999,
  'lon': 139.7609411,
  'heading': 341.8,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://nedujinja.or.jp/keidaiannai/', 'https://www.openstreetmap.org/way/164586227'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Shrine confirms original lacquered1706 tower gate, guardian niches and two-storey '
                 'structure. Width14.2m/depth10.6m approximate mapped outer roof dimensions;11.5m height and '
                 'structural bay proportions estimated from photographs, not published measurement. Lacquer '
                 'vermilion, ochre upper panels, green lattice, gray tile roof, hipped-gable ends, balcony '
                 'rails and projecting bracket stacks modeled. Heading derived from straight front polygon '
                 'edge, not a biased vertex-average PCA.',
  'source_exclusion_polygons': [[[-7.1, -5.3], [7.1, -5.3], [7.1, 5.3], [-7.1, 5.3]]],
  'max_source_building_span_m': 30.124076749337892,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-6.3, 6.3]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/nezu-jinja-romon.md',
            'placement_file': 'tools/monument_catalog/references/japanese/nezu-placement.json',
            'architectural_dimensions_m': [14.2, 11.5, 10.6],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/nezu-jinja-romon.json'},
  'capture': {'distance': 28.4,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 5.175,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-7.1, -5.3], [7.1, 5.3]]},
 {'name': 'yutoku-inari-romon',
  'city': 'Kashima',
  'lat': 33.07401125,
  'lon': 130.1083555,
  'heading': 190.6,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://yutokusan.jp/en/about/',
                 'https://www.city.saga-kashima.lg.jp/main/323.html',
                 'https://www.city.saga-kashima.lg.jp/main/32779.html'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Shrine and city identify polychrome lacquered gate.15.8m width and12m depth approximate '
                 'OSM roof footprint;15.5m height, bay proportions and ornamental relief estimated from '
                 'photographs. Dark weathered metal roof, vermilion structure, ochre upper panels, gold '
                 'rosettes and blue/green/ochre bracket frieze kept as separate materials and geometry. Fine '
                 'representational carving simplified.',
  'source_exclusion_polygons': [[[-7.9, -6.0], [7.9, -6.0], [7.9, 6.0], [-7.9, 6.0]]],
  'max_source_building_span_m': 33.728616929841635,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-7.0, 7.0]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/yutoku-inari-romon.md',
            'placement_file': 'tools/monument_catalog/references/japanese/yutoku-placement.json',
            'architectural_dimensions_m': [15.8, 15.5, 12.0],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/yutoku-inari-romon.json'},
  'capture': {'distance': 31.6,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 6.9750000000000005,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-7.9, -6.0], [7.9, 6.0]]},
 {'name': 'takeo-onsen-romon',
  'city': 'Takeo',
  'lat': 33.196231825,
  'lon': 130.0146574,
  'heading': 317.9,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.takeo-kk.net/sightseeing/001373.php',
                 'https://www.city.takeo.lg.jp/uploads/2021takeosougoupanfu.pdf'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Official tourism authority identifies Tatsuno Kingo1915 Tenpyo-style vermilion gate;12.5m '
                 'height and12m main width are estimated architectural proportions. OSM gate footprint '
                 'includes a narrow rear appendage: anchor and heading use central four roof corners only, '
                 'not the appendage centroid. White plaster rounded-corner ground portal, rectangular upper '
                 'windows, vermilion balcony and gray tile roof with shachi ridge ornaments. This is the '
                 'historic hot-spring gateway, not a shrine gate.',
  'source_exclusion_polygons': [[[-6.36, -4.325], [6.36, -4.325], [6.36, 4.325], [-6.36, 4.325]]],
  'max_source_building_span_m': 26.15022755159121,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-5.325, 5.325]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/takeo-onsen-romon.md',
            'placement_file': 'tools/monument_catalog/references/japanese/takeo-placement.json',
            'architectural_dimensions_m': [12.72, 12.5, 8.65],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/takeo-onsen-romon.json'},
  'capture': {'distance': 25.44,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 5.625,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-6.36, -4.325], [6.36, 4.325]]},
 {'name': 'usa-jingu-saidaimon',
  'city': 'Usa',
  'lat': 33.5234031,
  'lon': 131.3765314,
  'heading': 112.4,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://oita-digitalzukan.jp/cultural_property/%E8%A5%BF%E5%A4%A7%E9%96%80/',
                 'https://sitereports.nabunken.go.jp/files/attach/45/45875/122119_4_%E5%8F%B2%E8%B7%A1%E5%AE%87%E4%BD%90%E7%A5%9E%E5%AE%AE%E5%A2%83%E5%86%85%E5%A4%A9%E7%84%B6%E8%A8%98%E5%BF%B5%E7%89%A9%E5%AE%87%E4%BD%90%E7%A5%9E%E5%AE%AE%E7%A4%BE%E5%8F%A2%E4%BF%9D%E5%AD%98%E6%B4%BB%E7%94%A8%E8%A8%88%E7%94%BB.pdf'],
  'attribution': 'Original geometry authored for Low Poly Earth. Placement: © OpenStreetMap contributors (ODbL 1.0).',
  'description': 'Prefectural record identifies four-legged gate. Conservation report identifies gabled '
                 'roof, front/rear karahafu and flanking wickets, cypress bark covering.12x9.1x5.8m model '
                 'including eaves is estimated; mapped footprint anchors center and112.4deg long axis. '
                 'Vermilion timber, pale cusped bargeboards and brown bark roof retain distinct surfaces.',
  'source_exclusion_polygons': [[[-6.0, -2.9], [6.0, -2.9], [6.0, 2.9], [-6.0, 2.9]]],
  'max_source_building_span_m': 22.657881630902743,
  'passage_probe': {'x': 0, 'y': 1.5, 'z_range': [-3.9, 3.9]},
  'audit': {'reference_file': 'tools/monument_catalog/references/japanese/usa-jingu-saidaimon.md',
            'placement_file': 'tools/monument_catalog/references/japanese/usa-placement.json',
            'architectural_dimensions_m': [12, 9.1, 5.8],
            'grounding_note': 'Natural terrain is unchanged. The builder selects one surveyed fixed datum '
                              'from raw ground at the structural supports, keeps the connected upper gate '
                              'rigid, and extends only buried footing toes to 0.5m below the measured local '
                              'terrain. Approach roads remain enabled.',
            'ground_samples_file': 'tools/monument_catalog/references/ground/usa-jingu-saidaimon.json'},
  'capture': {'distance': 24.0,
              'azimuth': 190,
              'elevation': 18,
              'target_height': 4.095,
              'time_utc': '2026-03-20T03:00:00Z'},
  'suppress_bbox': [[-6.0, -2.9], [6.0, 2.9]]}]
