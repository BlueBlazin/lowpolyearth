"""Authored metric temple meshes; evidence and approximation bounds live in references/temples.

Coordinates use x east, y up and z north before the registry's clockwise heading.
Small sculptural relief is represented by faceted geometry, never downloaded artwork.
"""
import math
import numpy as np
from city_catalog.common import Mesh
from heritage_catalog.materials import (STONE_WALL, PLASTER_WALL, TIMBER_WALL, TILE_ROOF, METAL_ROOF, FLAT_ROOF, SCULPTURE_METAL, WEATHERED_LIMESTONE, VEINED_MARBLE, EARTHEN_PLASTER)
from city_catalog.expansion_heritage import _lathe, _turn, _arch, _tube, _dome, _ring
from heritage_catalog.geometry import face as _face

STONE=(132,128,115)
ANDESITE=(111,111,103)
DARK=(43,47,43)
IVORY=(228,222,201)
MARBLE=(237,233,218)
GOLD=(211,170,64)
OCHRE=(167,125,81)
RED=(136,52,40)
TAU=math.tau


def _frustum(m,x,z,y,h,w,d,tw,td,color,material=11):
    low=[(x-w/2,y,z-d/2),(x+w/2,y,z-d/2),(x+w/2,y,z+d/2),(x-w/2,y,z+d/2)]
    high=[(x-tw/2,y+h,z-td/2),(x+tw/2,y+h,z-td/2),(x+tw/2,y+h,z+td/2),(x-tw/2,y+h,z+td/2)]
    for j in range(4):
        k=(j+1)%4
        _face(m,[low[j],high[j],high[k],low[k]],color,material,((0,0,-1),(1,0,0),(0,0,1),(-1,0,0))[j])
    _face(m,high,color,material)
    _face(m,low,color,material,(0,-1,0))


def _polygon_prism(m,points,y,h,color,material=11):
    # Concave footprints use a center fan only when star-shaped about the origin.
    pts=np.asarray(points,float); center=pts.mean(axis=0)
    for j,a in enumerate(pts):
        b=pts[(j+1)%len(pts)]
        _face(m,[(a[0],y,a[1]),(b[0],y,b[1]),(b[0],y+h,b[1]),(a[0],y+h,a[1])],color,material,(b[1]-a[1],0,a[0]-b[0]))
        _face(m,[(center[0],y+h,center[1]),(a[0],y+h,a[1]),(b[0],y+h,b[1])],color,material)


def _steps(m,x,z,w,run,rise,y=0,n=12,color=STONE):
    for i in range(n):
        h=rise*(i+1)/n
        m.box((x,y+h/2,z+run*(i+.5)/n),(w,h,abs(run)/n+.02),color,8)


def _facade(m,x,z,w,y,h,lod,color=STONE,windows=5,rows=1,trim=None,slope=0):
    for row in range(rows):
        for j in range(windows):
            xx=x+(j-(windows-1)/2)*w/windows;yy=y+(row+.48)*h/rows
            ww=w/windows*.38;hh=h/rows*.47;zz=z+(yy-y)*slope
            def panel(width,height,depth,c,material=15):
                _face(m,[(xx-width/2,yy-height/2,zz-depth-height/2*slope),(xx+width/2,yy-height/2,zz-depth-height/2*slope),(xx+width/2,yy+height/2,zz-depth+height/2*slope),(xx-width/2,yy+height/2,zz-depth+height/2*slope)],c,material,(0,0,-1))
            if trim:panel(ww+.42,hh+.42,.03,trim,TIMBER_WALL)
            panel(ww,hh,.10,DARK)
            if lod==0:m.box((xx,yy-hh/2-.12,zz-.19-hh/2*slope),(ww+.6,.24,.32),color,17)
            if lod<2:panel(.12,hh,.15,color,17)


def _finial(m,x,z,y,h,color=GOLD,lod=0):
    _lathe(m,(x,z),[(y,.28*h),(y+.12*h,.31*h),(y+.2*h,.18*h),(y+.48*h,.22*h),(y+.68*h,.08*h),(y+h,.015)],color,SCULPTURE_METAL,(12,8,6)[lod])


def _relief(m,x,y,z,size,lod,color):
    # Faceted figure-sized relief preserves rhythm without copying iconography.
    m.box((x,y,z),(size*.45,size*.64,size*.22),color,17)
    if lod<2:
        m.ellipsoid((x,y+size*.48,z-.08),(size*.23,size*.26,size*.18),color,17,6,3)
        for s in (-1,1):
            _tube(m,(x+s*size*.2,y+size*.15,z),(x+s*size*.37,y-size*.17,z-.1),size*.09,color,17,4)


def _hip_roof(m,x,z,w,d,y,rise,color,trim,lod=0,roof_material=TILE_ROOF):
    # Two-stage concave eaves, with a short ridge and rising corner tips.
    _frustum(m,x,z,y,.4,w,d,w*.99,d*.99,trim,SCULPTURE_METAL)
    _frustum(m,x,z,y+.4,rise*.43,w*.99,d*.99,w*.70,d*.65,color,roof_material)
    _frustum(m,x,z,y+.4+rise*.43,rise*.57,w*.70,d*.65,w*.45,.45,color,roof_material)
    m.box((x,y+rise+.7,z),(w*.48,.45,.65),trim,SCULPTURE_METAL)
    if lod<2:
        for s in (-1,1):
            for t in (-1,1):
                _tube(m,(x+s*w*.35,y+rise*.42,z+t*d*.33),(x+s*w*.5,y+.8,z+t*d*.5),.13,trim,SCULPTURE_METAL,5)


def _borobudur_outline(half):
    q=[(0,-half),(.28*half,-half),(.28*half,-half+2.7),(.60*half,-half+2.7),(.60*half,-half+5.4),(half-5.4,-half+5.4),(half-5.4,-.60*half),(half-2.7,-.60*half),(half-2.7,-.28*half),(half,-.28*half)]
    out=[]
    for side in range(4):
        a=side*math.pi/2;c=math.cos(a);ss=math.sin(a)
        out.extend((x*c-z*ss,x*ss+z*c) for x,z in q)
    return out


def _borobudur(lod):
    m=Mesh(); n=(32,24,16)[lod]
    _polygon_prism(m,_borobudur_outline(61.5),-1.4,5.4,(126,123,110),8)
    # Five square galleries are separated by real walkable setbacks.
    for level,(half,top) in enumerate([(49,7.0),(45,11.0),(42,14.0),(38.5,17.0),(31.2,20.0)]):
        bottom=[4,7,11,14,17][level]
        _polygon_prism(m,_borobudur_outline(half),bottom,top-bottom,ANDESITE)
        for yy,over in [(top-.7,.2),(top-.16,.45)]:
            _polygon_prism(m,_borobudur_outline(half+over*.5),yy-.12,.24,(137,135,124),17)
        for side in range(4):
            face=Mesh(); count=(20,13,7)[lod]-level
            for j in range(count):
                xx=(j-(count-1)/2)*(half*2-14)/count
                if abs(xx)<3.7:continue
                front=-half+(0 if abs(xx)<half*.28 else 2.7 if abs(xx)<half*.60 else 5.4)
                face.box((xx,top-1.8,front-.13),((half*2-6)/count-.25,2.0,.28),(103+3*(j%3),106+2*(j%3),100),17)
                if lod<2:
                    _relief(face,xx,top-1.8,front-.34,1.25,lod,(137,135,124))
                # Stupa-headed niches form the unmistakable gallery skyline.
                face.box((xx,top+.65,front+.9),(2.0,1.3,1.8),(119,121,113))
                _arch(face,(xx,top+.05,front-.04),1.22,1.12,.09,STONE,True,(12,8,6)[lod],DARK)
                _lathe(face,(xx,front+.9),[(top+1.3,1.14),(top+1.65,.9),(top+2.1,.5),(top+2.45,.09)],STONE,17,8)
            m.merge(_turn(face,side*90))
    for radius,y in [(26,22.0),(20,24.0),(13.5,26.0)]:
        _lathe(m,(0,0),[(y-1.6,radius),(y,radius)],ANDESITE,8,(64,48,32)[lod])
    for count,radius,y in [(32,23.5,22.0),(24,17.3,24.0),(16,11.5,26.0)]:
        for j in range(count):
            a=TAU*j/count;x=radius*math.cos(a);z=radius*math.sin(a)
            _lathe(m,(x,z),[(y,1.6),(y+.35,1.6),(y+.6,1.37)],STONE,17,n//2)
            # Open shell cells contain visible gaps at both detailed levels.
            for row in range((4,3,2)[lod]):
                lo=y+.6+row*.55;hi=lo+.55
                r0=1.37-.16*row;r1=r0-.16
                for k in range(n//2):
                    ang=TAU*k/(n//2); step=TAU/(n//2)
                    def p(t,r,yy):return (x+r*math.cos(t),yy,z+r*math.sin(t))
                    a0=p(ang,r0,lo);a1=p(ang+step,r0,lo);b0=p(ang,r1,hi);b1=p(ang+step,r1,hi)
                    mid=(np.asarray(a0)+a1+b0+b1)/4
                    for u,v in [(a0,a1),(a1,b1),(b1,b0),(b0,a0)]:
                        uu=np.asarray(u)*.56+mid*.44;vv=np.asarray(v)*.56+mid*.44
                        _face(m,[u,v,vv,uu],ANDESITE,17,(math.cos(ang+step/2),0,math.sin(ang+step/2)))
            capy=y+.6+(4,3,2)[lod]*.55
            _lathe(m,(x,z),[(capy,.85),(y+3.3,.64),(y+3.55,.38),(y+4.6,.05)],STONE,17,n//2)
    _lathe(m,(0,0),[(26.4,5.45),(27.2,5.45),(27.7,5.0),(29.0,4.5),(30.5,3.9),(31.7,2.85),(32.1,2.5),(32.45,2.6),(32.8,1.8),(35.4,.06)],STONE,11,n)
    for ang in (0,90,180,270):
        st=Mesh();st.box((0,-.2,-63),(5.5,.4,4),(144,140,126),STONE_WALL)
        for start,end,bottom,top in [(-65,-61,0,4),(-55,-49,4,7),(-49,-45,7,11),(-45,-42,11,14),(-42,-38.5,14,17),(-38.5,-31.2,17,20),(-31.2,-26,20,22)]:
            _steps(st,0,start,5.5,end-start,top-bottom,bottom,(12,8,5)[lod],(144,140,126))
        for yy,zz in [(7,-49),(11,-45),(14,-42),(17,-38.5),(20,-31.2)]:
            for s in (-1,1):st.box((s*3.0,yy+.9,zz),(1,1.8,2),STONE)
        m.merge(_turn(st,ang))
    return m


def _burmese_pavilion(m,x,z,w,d,y,h,lod):
    m.box((x,y+h*.33,z),(w,h*.66,d),(208,197,165))
    _facade(m,x,z-d/2-.1,w,y,h*.66,lod,IVORY,max(2,int(w/3)),1,RED)
    for i in range(4):
        ww=w*(1-i*.19);dd=d*(1-i*.19)
        _hip_roof(m,x,z,ww+1,dd+1,y+h*.66+i*1.7,2.1,RED,GOLD,lod)
    _finial(m,x,z,y+h*.66+7.5,2.2,GOLD,lod)


def _shwedagon(lod):
    m=Mesh();n=(64,48,32)[lod]
    m.box((0,-1.5,0),(196,3.0,190),(211,204,180),8)
    # Official 1,420-foot basal perimeter and 326-foot summit above platform.
    _lathe(m,(0,0),[(0,70.7),(2,70.7),(2.4,65),(4.8,65),(5.1,60),(7.5,60),(8,56)],GOLD,SCULPTURE_METAL,8,math.pi/8)
    profile=[(8,56),(10,52),(12,47),(16,42),(19,38),(23,34),(29,29),(36,24),(44,21),(49,19.4),(52,16.6),(55,13.2),(57,12.4),(60,11.1),(65,9.4),(70,7.8),(75,6.5),(80,5.2),(85,3.8),(89,2.7),(92,1.6),(96,1.1),(99.365,.04)]
    _lathe(m,(0,0),profile,lambda k,j: [(211,168,58),(220,180,66),(204,157,46)][(j//4+k//4)%3],SCULPTURE_METAL,n)
    for yy,rr in [(10,52),(12,47),(16,42),(19,38),(49,19.4),(52,16.6),(55,13.2),(57,12.4),(60,11.1)]:
        _lathe(m,(0,0),[(yy-.25,rr+.30),(yy+.18,rr+.30)],(237,193,77),17,n)
    for yy in np.arange(62,89,1.5 if lod<2 else 3):
        rr=float(np.interp(yy,[p[0] for p in profile],[p[1] for p in profile]))
        _lathe(m,(0,0),[(yy,rr+.18),(yy+.15,rr+.18)],(236,189,72),17,n)
    # The hti has distinct stacked umbrella rings and a needle, not a cone.
    for yy,rr in [(90,2.4),(91,2.1),(92,1.8),(93,1.45),(94,1.2),(95,.9)]:
        _lathe(m,(0,0),[(yy-.14,rr*.8),(yy,rr),(yy+.15,rr*.7)],(222,188,91),17,n//2)
    for j in range(64):
        a=TAU*j/64;rad=74+2*(j%2);x=rad*math.cos(a);z=rad*math.sin(a);hh=7.8+(j%4)*.7
        _lathe(m,(x,z),[(0,2.1),(1.3,2.1),(1.6,1.8),(3,1.5),(4.1,1.2),(5,.65),(hh,.02)],GOLD,SCULPTURE_METAL,(16,12,8)[lod])
    for x,z,w,d,h in [(-82,-29,14,20,13),(80,-30,17,21,14),(-35,-84,20,15,12),(34,-84,24,16,13),(-33,82,24,16,12),(35,82,19,15,12),(-85,24,17,20,12),(84,25,16,24,14)]:
        _burmese_pavilion(m,x,z,w,d,0,h,lod)
    for j in range(16):
        a=TAU*j/16
        if j%4==0:continue
        x=86*math.cos(a);z=86*math.sin(a)
        _burmese_pavilion(m,x,z,7.5,8,0,5,lod)
    return m


def _prang(m,x,z,base,height,lod,porcelain=True):
    n=(24,16,12)[lod];c=IVORY if porcelain else ANDESITE
    profile=[(0,base),(height*.06,base),(height*.10,base*.90),(height*.22,base*.77),(height*.27,base*.79),(height*.31,base*.65),(height*.48,base*.57),(height*.55,base*.46),(height*.70,base*.33),(height*.85,base*.18),(height*.95,base*.09),(height,.01)]
    _lathe(m,(x,z),profile,c,11,n,math.pi/4)
    for i,(y,r) in enumerate(profile[1:-1]):
        _lathe(m,(x,z),[(y-.15,r+.25),(y+.25,r+.25)],(159,154,128) if porcelain else STONE,17,n)
        if lod<2:
            for j in range(n):
                a=TAU*(j+.5)/n+math.pi/4
                color=[(115,155,134),(75,126,155),(186,121,99),(193,167,91),(185,183,159)][(j+i*2)%5] if porcelain else (136,133,120)
                # Porcelain rosettes retain their individual muted ceramic colors.
                xx=x+(r+.18)*math.cos(a);zz=z+(r+.18)*math.sin(a)
                m.ellipsoid((xx,y+.85,zz),(.30,.62,.30),color,17,6,3)
    if porcelain:
        # The upper prang is a stack of projecting floral registers; continuous
        # taper alone loses its Khmer silhouette even when porcelain is visible.
        for yy in np.linspace(height*.51,height*.94,(19,15,10)[lod]):
            rr=float(np.interp(yy,[p[0] for p in profile],[p[1] for p in profile]))
            projection=min(.48,base*.035)
            _lathe(m,(x,z),[(yy-.18,rr),(yy,rr+projection),(yy+.34,rr+projection*.55),(yy+.52,rr*.96)],IVORY,11,n)
            for j in range(n):
                a=TAU*j/n;rr2=rr+projection
                p=np.array((x+rr2*math.cos(a),yy+.28,z+rr2*math.sin(a)))
                tangent=np.array((-math.sin(a),0,math.cos(a)))*min(.3,rr*.11)
                _face(m,[p-tangent,p+tangent,p+np.array((0,.65,0))],(186,194,169),17,(math.cos(a),.3,math.sin(a)))
        for yy in np.arange(height*.12,height*.92,(1.45,2.4,4.4)[lod]):
            rr=float(np.interp(yy,[p[0] for p in profile],[p[1] for p in profile]))
            count=max(8,int(TAU*rr/(1.8,2.8,4.5)[lod]))
            for j in range(count):
                a=TAU*(j+.5*(int(yy)%2))/count;radial=np.array([math.cos(a),0,math.sin(a)]);tangent=np.array([-math.sin(a),0,math.cos(a)])
                center=np.array([x,yy,z])+radial*(rr*1.013+.08)
                width=min(.45,rr*.1);hh=(.48,.65,.9)[lod]
                color=[(115,155,134),(171,120,97),(133,164,166),(191,169,119),(209,206,181)][(j*3+int(yy))%5]
                _face(m,[center+np.array([0,hh,0]),center+tangent*width,center-np.array([0,hh,0]),center-tangent*width],color,17,radial)
        for j in range(4):
            face=Mesh();niche_y=height*.33;niche_z=-base*.66
            _arch(face,(0,niche_y,niche_z),base*.38,height*.115,.26,IVORY,True,(16,12,8)[lod],(99,96,81))
            if height>50:
                # Indra's Erawan occupies the central prang's four niches.
                face.ellipsoid((0,niche_y+2.3,niche_z-.35),(1.6,1.2,.8),IVORY,17,10,5)
                for dx in (-1.05,0,1.05):
                    face.ellipsoid((dx,niche_y+3.0,niche_z-.95),(.62,.72,.45),IVORY,17,8,4)
                    _tube(face,(dx,niche_y+2.8,niche_z-1.2),(dx*.87,niche_y+1.0,niche_z-1.4),.20,IVORY,17,6)
                face.ellipsoid((0,niche_y+4.5,niche_z-.35),(.48,.75,.38),(98,143,106),17,8,4)
                face.ellipsoid((0,niche_y+5.45,niche_z-.35),(.32,.39,.3),(137,166,123),17,8,4)
                _finial(face,0,niche_z-.35,niche_y+5.8,.9,GOLD,lod)
            m.merge(_turn(face,j*90,x,0,z))


def _wat_arun(lod):
    m=Mesh();m.box((0,-2.1,0),(101,4.2,101),(209,200,176),8)
    for i,(w,y) in enumerate([(72,2.0),(60,5.5),(45,11.5)]):
        m.box((0,y/2,0),(w,y,w),IVORY)
        for side in range(4):
            deco=Mesh()
            for j in range((22,14,8)[lod]):
                x=(j-((22,14,8)[lod]-1)/2)*(w-3)/(22,14,8)[lod]
                _relief(deco,x,y-1,-w/2-.2,1.5,lod,[(122,161,138),(173,130,106),(104,145,159)][j%3])
            m.merge(_turn(deco,side*90))
    core=Mesh();_prang(core,0,0,16.4,70.5,lod);m.merge(_turn(core,0,0,11.5,0))
    for x in (-35,35):
        for z in (-35,35):_prang(m,x,z,6.0,31.0,lod)
    for a in range(4):
        q=Mesh();_steps(q,0,-39,6,20,22,0,(30,20,12)[lod],IVORY)
        for s in (-1,1):
            _tube(q,(s*3.5,.8,-39),(s*3.5,22.8,-19),.32,(174,171,149),17,6)
        m.merge(_turn(q,a*90))
    return m


def _tibetan_block(m,x,z,w,d,y,h,lod,red=False):
    color=(127,48,37) if red else (230,223,200)
    _frustum(m,x,z,y,h,w,d,w-2.2,d-2.2,color)
    for face in range(4):
        f=Mesh();fw=w-2.2 if face%2==0 else d-2.2
        _facade(f,0,-(d/2 if face%2==0 else w/2)+1.1*max(3,h*.25)/h-.02,fw-4,y+max(3,h*.25),h*.68,lod,color,max(2,round(fw/5)),max(1,round(h/5.7)),(84,51,36),slope=1.1/h)
        m.merge(_turn(f,face*90,x,0,z))
    m.box((x,y+h-.7,z),(w-1.2,1.7,d-1.2),(99,47,32),17)
    m.box((x,y+h+.25,z),(w-1.0,.45,d-1.0),(179,159,111),12)


def _potala(lod):
    m=Mesh()
    def palace_block(x,z,w,d,y,h,red=False,central=False):
        color=(128,49,41) if red else (229,225,210)
        inset=min(3.5,h*.045)
        # The conservation survey shows window ranks concentrated at the top of
        # battered, largely blind white walls, with dark tamarisk parapet bands.
        low=[(-w/2,y,-d/2),(w/2,y,-d/2),(w/2,y,d/2),(-w/2,y,d/2)]
        high=[(-w/2+inset,y+h,-d/2+inset),(w/2-inset,y+h,-d/2+inset),(w/2-inset,y+h,d/2-inset),(-w/2+inset,y+h,d/2-inset)]
        body=Mesh()
        for side in range(4):
            j=(side+1)%4;normal=((0,0,-1),(1,0,0),(0,0,1),(-1,0,0))[side]
            if central and side==0:
                # The central balcony belt is a genuine2.2m recess, rather than
                # a dark rectangle on an uninterrupted red facade.
                for a,b in [(-w/2,-8),(8,w/2)]:
                    aa=a+inset if a==-w/2 else a;bb=b-inset if b==w/2 else b
                    _face(body,[(a,y,-d/2),(b,y,-d/2),(bb,y+h,-d/2+inset),(aa,y+h,-d/2+inset)],color,EARTHEN_PLASTER,normal)
                _face(body,[(-8,y,-d/2+2.2),(8,y,-d/2+2.2),(8,y+h,-d/2+inset+2.2),(-8,y+h,-d/2+inset+2.2)],(72,43,35),TIMBER_WALL,normal)
                for xx,n in [(-8,(1,0,0)),(8,(-1,0,0))]:
                    _face(body,[(xx,y,-d/2),(xx,y+h,-d/2+inset),(xx,y+h,-d/2+inset+2.2),(xx,y,-d/2+2.2)],color,EARTHEN_PLASTER,n)
            else:_face(body,[low[side],low[j],high[j],high[side]],color,EARTHEN_PLASTER,normal)
        _face(body,high,(174,155,120),EARTHEN_PLASTER)
        m.merge(_turn(body,0,x,0,z))
        for side in range(4):
            f=Mesh();span=w if side%2==0 else d;depth=d if side%2==0 else w
            slope=inset/h
            def panel(xx,yy,ww,hh,c,mat,offset=.06,flare=1):
                def v(dx,dy):return (xx+dx,yy+dy,-depth/2+(yy+dy-y)*slope-offset)
                _face(f,[v(-ww/2,-hh/2),v(ww/2,-hh/2),v(ww*flare/2,hh/2),v(-ww*flare/2,hh/2)],c,mat,(0,0,-1))
            upper=y+h;rows=max(3,min(6,round(h/7))) if red else 2
            if h<23:rows=2
            pitch_y=4.7 if red else 4.2
            count=max(2,round((span-2*inset-3)/4.1));pitch=(span-2*inset-3)/count
            for row in range(rows):
                yy=upper-(5.8 if red else 4.3)-row*pitch_y
                for j in range(count):
                    xx=(j-(count-1)/2)*pitch
                    if central and side==0 and abs(xx)<10:continue
                    ww=min(1.35,pitch*.33)
                    panel(xx,yy,ww*1.6,3.3,(47,39,34),TIMBER_WALL,.08,1.16)
                    panel(xx,yy,ww,2.3,(104,47,30),TIMBER_WALL,.12)
                    panel(xx,yy,ww-.25,2.05,(29,33,31),15,.15)
                    if lod<2:
                        panel(xx,yy,.12,2.2,(137,105,70),TIMBER_WALL,.18)
                        panel(xx,yy+1.78,ww*2.05,.25,(179,158,116),TIMBER_WALL,.46)
                        panel(xx,yy+.2,ww,.10,(115,80,46),TIMBER_WALL,.18)
                    # Annual splashing leaves narrow gravity-driven flows;
                    # measured finish buildup is50–100mm, not exposed joints.
                    if lod==0 and not red and row==rows-1:
                        bottom=max(y+.8,yy-7-(j%3)*1.5)
                        panel(xx-.5,(bottom+yy-1.9)/2,.18,yy-1.9-bottom,(220,217,203),EARTHEN_PLASTER,.045,.4)
            if not red and h>=40 and side==0:
                # Small blind warehouse openings sit below the principal ranks;
                # their scale preserves the solid lower retaining wall.
                for yy in [upper-15.2,upper-19.4]:
                    for j in range(count):
                        panel((j-(count-1)/2)*pitch,float(yy),.58,.82,(75,69,57),EARTHEN_PLASTER,.055)
            band=3.0 if red else 1.7
            panel(0,upper-band/2,span-2*inset+.45,band,(76,35,30),TIMBER_WALL,.12)
            panel(0,upper-band-.45,span-2*inset+.2,.75,(228,222,206),EARTHEN_PLASTER,.14)
            panel(0,upper+.08,span-2*inset+.65,.22,(162,125,74),TIMBER_WALL,.28)
            if red:
                for xx in np.arange(-span/2+inset+4,span/2-inset-2,8):panel(float(xx),upper-1.5,.65,.85,GOLD,SCULPTURE_METAL,.2)
            if central and side==0:
                for xx in [-7.4,7.4]:panel(xx,y+h-19,1.2,33,(228,222,206),EARTHEN_PLASTER,.08)
                for yy in np.arange(y+h-32,y+h-3,5.7):
                    panel(0,float(yy),14.2,.9,(223,216,196),EARTHEN_PLASTER,.05)
                    for xx in [-5.4,-2.7,0,2.7,5.4]:panel(xx,float(yy)+2.4,.2,3.7,(160,111,57),TIMBER_WALL,-1.95)
            m.merge(_turn(f,side*90,x,0,z))
    # The compact authored outcrop supports the palace; the wider Marpo Ri
    # landscape is supplied by the DEM, not an invented flat rectangle.
    for k in range(18):
        a=TAU*k/18;b=TAU*(k+1)/18
        outer=lambda t:(190*math.cos(t),-10+3*math.sin(t*3),86*math.sin(t))
        inner=lambda t:(130*math.cos(t),30+8*math.sin(t),55*math.sin(t))
        _face(m,[outer(a),outer(b),inner(b),inner(a)],[(142,124,99),(153,131,102),(136,119,96)][k%3],4,(math.cos(a),1,math.sin(a)))
    blocks=[(-112,-36,55,39,0,33,False),(-78,-20,40,45,13,45,False),(-56,2,32,66,27,51,False),
            (105,8,74,60,0,56,False),(178,25,86,21,18,17,False),(86,0,40,81,18,58,False),(57,11,38,86,32,51,False),
            (2,12,105,79,27,72,True),(-29,-25,77,27,15,57,False),
            (22,-33,38,30,22,52,False),(60,-28,33,29,24,48,False),
            (-58,30,48,60,40,55,True),(28,45,62,32,42,58,True)]
    for x,z,w,d,y,h,r in blocks:palace_block(x,z,w,d,y,h,r,central=x==2 and r)
    _frustum(m,178,25,10,8,88,23,86,21,(229,225,210),EARTHEN_PLASTER)
    # White eastern assembly courtyard remains open.
    for x,z,w,d,y,h in [(109,-26,44,13,45,20),(104,28,53,15,42,28),(137,5,14,45,36,25)]:
        palace_block(x,z,w,d,y,h)
    for x,z,w,d,y in [(-33,14,22,20,99),(-9,40,20,18,100),(20,9,25,22,101),(42,42,15,14,100),(-60,39,16,17,95),(-32,-13,13,11,97),(24,-15,17,13,97)]:
        m.box((x,y+1.4,z),(w-3,2.8,d-3),RED)
        _hip_roof(m,x,z,w,d,y+2.8,5,GOLD,(234,193,78),lod,SCULPTURE_METAL)
        _finial(m,x,z,y+8.5,3.5,GOLD,lod)
    # Southern zigzag ascent is masonry stepped into the natural hill.
    for x0,x1 in [(-115,-81),(65,99)]:
        for i in range(4):
            start=x0 if i%2==0 else x1;end=x1 if i%2==0 else x0;zz=-70+i*7
            st=Mesh();_steps(st,0,0,4,x1-x0,7.5,0,(20,12,8)[lod],IVORY)
            m.merge(_turn(st,90 if i%2==0 else -90,start,i*7.5,zz))
            if i<3:m.box((end,(i+1)*7.5-.35,zz+3.5),(4.2,.7,7.0),IVORY,8)
    return m


def _paro(lod):
    m=Mesh()
    # Only the immediate cantilevering cliff is modeled; the 900 m valley relief
    # is terrain. Buildings occupy distinct ledges, not one leveling pad.
    ridge=[(-42,-4,-15),(-30,1,-19),(-17,3,-17),(0,9,-16),(17,12,-11),(33,15,-3),(48,18,10)]
    def rock_skin(a,b,c,d,normal,sector):
        # Broad vertical joints break the exposed gneiss into irregular facets;
        # boundary vertices stay fixed so neighboring cliff panels remain closed.
        nx,ny=((7,14),(5,10),(3,6))[lod]
        grid=[]
        for j in range(ny+1):
            v=j/ny;row=[]
            for k in range(nx+1):
                u=k/nx;vv=v+.025*math.sin(u*math.pi)*math.sin(v*math.pi)*math.sin(sector+u*4)
                p=(1-vv)*((1-u)*a+u*b)+vv*((1-u)*d+u*c)
                relief=math.sin(math.pi*u)*math.sin(math.pi*v)
                p=p+np.array(normal)*relief*(1.1*math.sin((k+sector)*2.1+j*1.7)+.65*math.sin(j*3.1))
                row.append(p)
            grid.append(row)
        for j in range(ny):
            for k in range(nx):
                center=(grid[j][k]+grid[j+1][k+1])*.5
                tone=3*math.sin(center[0]*.12+center[1]*.06)+2*math.cos(center[1]*.21-center[2]*.1)
                shade=tuple(int(c+tone)for c in (112,113,101))
                _face(m,[grid[j][k],grid[j][k+1],grid[j+1][k+1]],shade,4,normal)
                _face(m,[grid[j][k],grid[j+1][k+1],grid[j+1][k]],shade,4,normal)
    # Exact rear edge DEM samples remove the formerly elevated west slab.
    rear=[3.323,11.948,34.961,78.325,111.860,129.446,138.058]
    for i in range(len(ridge)-1):
        a=np.array(ridge[i],float);b=np.array(ridge[i+1],float)
        am=a+np.array([(-1)**i*2,-38,4]);bm=b+np.array([(-1)**(i+1)*2,-38,4])
        al=np.array([a[0]+4,-90,a[2]+15]);bl=np.array([b[0]+4,-90,b[2]+15])
        rock_skin(a,b,bm,am,(0,0,-1),i)
        rock_skin(am,bm,bl,al,(0,0,-1),i+6)
        rock_skin(a,b,np.array((b[0],rear[i+1],40)),np.array((a[0],rear[i],40)),(0,1,-1),i+12)
    for end,normal,back in [(ridge[0],(-1,0,0),rear[0]),(ridge[-1],(1,0,0),rear[-1])]:
        x,y,z=end;rock_skin(np.array((x,y,z)),np.array((x,back,40)),np.array((x+4,-90,40)),np.array((x+4,-90,z+15)),normal,20)
    for x,z,w,d,y,h in [(-23,11,20,29,0,18),(-8,5,27,30,6,20),(6,21,22,20,9,24),(22,5,18,16,14,18),(-28,17,14,13,10,15),(20,-25,26,17,-5,15),(-2,-21,14,11,-4,16)]:
        foundation_depth=14 if z<0 else 12 if x<-25 else 8
        _frustum(m,x,z,y-foundation_depth,foundation_depth,w+1,d+1,w,d,(150,139,118),STONE_WALL)
        _tibetan_block(m,x,z,w,d,y,h,lod)
        # Bhutanese rafters and stacked broad red/gold eaves differ from Tibet.
        for i in range(2):
            _hip_roof(m,x,z,w+3-i*3,d+3-i*3,y+h+i*2.8,3.0,(146,61,42),GOLD,lod,METAL_ROOF)
        _finial(m,x,z,y+h+6.8,2.1,GOLD,lod)
        if lod<2:
            for xx in np.arange(x-w/2+1,x+w/2,1):m.box((xx,y+h-.4,z-d/2-.7),(.18,1.0,1.2),(106,52,29),17)
    # Open gallery joins the lower southern rooms.
    m.box((6,8,-20),(22,.9,3.5),IVORY,8)
    for x in np.arange(-4,18,3):m.box((x,10,-21),(.3,4,.3),RED,17)
    _hip_roof(m,6,-20,25,5,12,1.6,RED,GOLD,lod,METAL_ROOF)
    return m


def _javanese_shrine(m,x,z,w,height,lod):
    # Stepped cruciform plinth, deep entrance bays and clustered ratna pinnacles.
    for i in range(3):
        ww=w*(1-i*.08);y=i*height*.07
        m.box((x,y+height*.035,z),(ww,height*.07,ww),ANDESITE)
        m.box((x,y+height*.065,z),(ww+.25,.28,ww+.25),STONE,17)
    body_y=height*.21;body_h=height*.30;bw=w*.69
    m.box((x,body_y+body_h/2,z),(bw,body_h,bw),ANDESITE)
    for side in range(4):
        f=Mesh();f.box((0,body_y+body_h/2,-bw/2),(bw*.36,body_h,2.0),(123,122,111))
        _arch(f,(0,body_y,-bw/2-1.06),bw*.22,body_h*.75,.22,STONE,False,(12,8,6)[lod],DARK)
        if lod<2:
            for xx in [-bw*.32,bw*.32]:_relief(f,xx,body_y+body_h*.52,-bw/2-.22,bw*.15,lod,(145,140,123))
        m.merge(_turn(f,side*90,x,0,z))
    # Ratna ranks articulate a narrow roof core; the bell shoulders and necks
    # are original proportional interpretations of the surviving vajra crowns.
    def ratna(px,pz,y,r,h):
        profile=[(0,.84),(.08,1),(.17,.72),(.29,.70),(.43,.98),
                 (.55,.93),(.67,.54),(.81,.43),(1,.025)]
        _lathe(m,(px,pz),[(y+h*t,r*q) for t,q in profile],
               (124,122,111),STONE_WALL,(12,8,6)[lod])
        if lod<2:
            for angle in range(0,360,90):
                theta=math.radians(angle)
                _lathe(m,(px+math.cos(theta)*r*.74,pz+math.sin(theta)*r*.74),
                       [(y+h*.32,r*.20),(y+h*.57,r*.23),(y+h*.77,.012)],
                       (116,115,104),STONE_WALL,(6,5)[lod])
    for tier,ratio in enumerate((.75,.57,.43,.31,.20)):
        y=height*(.51+tier*.071);ww=w*ratio;th=height*.071
        _frustum(m,x,z,y,th,ww,ww,ww*.79,ww*.79,ANDESITE,STONE_WALL)
        for frac,scale in ((.02,1.025),(.14,1.015),(.80,.83)):
            m.box((x,y+th*frac,z),(ww*scale,th*.07,ww*scale),
                  (123,121,110),STONE_WALL)
        rank=(4,3,3,2,1)[tier]
        for side in range(4):
            for j in range(rank):
                along=(-.39+.78*j/max(1,rank-1))*ww if rank>1 else 0
                px,pz=((along,-ww*.45),(ww*.45,along),
                       (-along,ww*.45),(-ww*.45,-along))[side]
                ratna(x+px,z+pz,y+th*.16,w*(.034-tier*.003),height*(.10-tier*.007))
    ratna(x,z,height*.855,w*.065,height*.145)
    # East-facing stair projects toward +x.
    s=Mesh();_steps(s,0,0,w*.26,w*.5,body_y,0,(18,12,7)[lod],STONE);m.merge(_turn(s,-90,x+w*.88,0,z))


def _prambanan(lod):
    m=Mesh();m.box((0,-1.1,0),(112,2.2,112),(167,162,140),8)
    for x,z,w,h in [(-24,0,34,47),(-24,-37,23,33),(-24,37,23,33),(24,0,19,25),(24,-37,15,22),(24,37,15,22),(0,-47,8,16),(0,47,8,16)]:
        _javanese_shrine(m,x,z,w,h,lod)
    for x,z in [(-49,-49),(-49,49),(49,-49),(49,49),(0,-53),(0,53),(-53,0),(53,0)]:
        _javanese_shrine(m,x,z,4,5.7,lod)
    # The 224 perwara locations are ruins, not a fictional fully rebuilt ring.
    for ring,count in [(56,44),(67,52),(78,60),(89,68)]:
        per_side=count//4
        for side in range(4):
            for j in range(per_side):
                along=-ring+(j+.5)*2*ring/per_side
                if side==0:x,z=along,-ring
                elif side==1:x,z=ring,along
                elif side==2:x,z=-along,ring
                else:x,z=-ring,-along
                h=1.0+.35*((j*7+side*3)%5)
                m.box((x,(h-.35)/2,z),(6,h+.35,6),ANDESITE)
                for k in range((3,2,1)[lod]):
                    m.box((x-1.5+k*1.2,h+.35+k*.14,z+((j+k)%3-1)),(1.5,.7,1.7),(129,126,111))
    # Surviving/restored perwara examples provide the scale of the ruined field.
    for x,z in [(56,-25),(56,-35)]:_javanese_shrine(m,x,z,6.5,12,lod)
    return m


def _dravida_tower(m,x,z,w,d,height,lod,color=OCHRE,painted=False,tiers=9):
    base=height*.20
    # Open ground passage between masonry cheeks.
    for side in (-1,1):m.box((x+side*w*.34,base/2,z),(w*.32,base,d),color)
    m.box((x,base*.9,z),(w,base*.2,d),color)
    pal=[(105,153,166),(157,108,151),(189,140,101),(139,166,129),(212,189,138),(164,78,72)]
    for i in range(tiers):
        ww=w*(1-i*.062);dd=d*(1-i*.046);y=base+i*(height*.69/tiers);th=height*.69/tiers
        _frustum(m,x,z,y,th,ww,dd,ww-w*.061,dd-d*.045,(156,151,133) if painted else color)
        m.box((x,y+.2,z),(ww+.8,.42,dd+.8),(83,133,146) if painted else (181,140,91),17)
        count=max(3,int(ww/(1.25 if lod==0 else 2.4 if lod==1 else 4.5)))
        for side in (-1,1):
            for j in range(count):
                xx=x+(j-(count-1)/2)*(ww-1)/count;zz=z+side*(dd/2-.12)
                cc=pal[(i*3+j)%len(pal)] if painted else (174+(j%3)*4,135+(j%3)*3,92+(j%3)*3)
                _relief(m,xx,y+th*.47,zz,min(th*.6,(ww-1)/count*.8),lod,cc)
                if lod==0:
                    _lathe(m,(xx,zz),[(y+th*.78,.34),(y+th*.9,.42),(y+th*.99,.05)],cc,17,6)
        for side in (-1,1):
            for j in range(max(2,count//2)):
                zz=z+(j-(max(2,count//2)-1)/2)*(dd-1)/max(2,count//2)
                m.ellipsoid((x+side*(ww/2-.2),y+th*.5,zz),(.5,th*.25,.4),pal[(j+i)%6] if painted else color,17,6,3)
    # Barrel-shaped sala ridge and its row of metal kalashas.
    roofy=height*.89;rw=w*.45;rd=d*.59
    for i in range(7):
        t0=math.pi*i/7;t1=math.pi*(i+1)/7
        a=(x-rw/2,roofy+math.sin(t0)*height*.055,z+math.cos(t0)*rd/2)
        b=(x+rw/2,a[1],a[2]);c=(b[0],roofy+math.sin(t1)*height*.055,z+math.cos(t1)*rd/2);dd=(a[0],c[1],c[2])
        _face(m,[a,b,c,dd],(123,160,173) if painted else (175,135,90),12,(0,1,0))
    for j in range(7):_finial(m,x+(j-3)*rw/7,z,height*.945,height*.055,GOLD if painted else (164,123,75),lod)


def _brihadisvara(lod):
    m=Mesh();m.box((0,-2.5,0),(241,5.0,122),(193,177,142),8)
    for side in (-1,1):
        m.box((0,3.0,side*58),(239,6,5),(161,134,96))
        if lod<2:
            for xx in np.arange(-115,116,5):m.box((xx,3.1,side*55.35),(2.1,4.2,.3),(93,85,65),15)
    m.box((-117,3,0),(5,6,117),(161,134,96))
    # The eastern processional axis passes through both gopurams and the
    # enclosure wall; the other three sides remain continuous masonry.
    for side in (-1,1):m.box((117,3,side*31.125),(5,6,54.75),(161,134,96))
    x=-61;w=30.2;m.box((x,5.5,0),(w,11,w),OCHRE)
    for i in range(13):
        y=11+i*3.25;ww=w-i*1.65
        _frustum(m,x,0,y,3.25,ww,ww,ww-1.65,ww-1.65,OCHRE)
        m.box((x,y+.35,0),(ww+.55,.5,ww+.55),(193,151,98),17)
        for side in range(4):
            f=Mesh()
            for j in range(max(3,7-i//3)):
                xx=(j-(max(3,7-i//3)-1)/2)*(ww-2)/max(3,7-i//3)
                _relief(f,xx,y+1.7,-ww/2-.08,1.2,lod,(185,146,100))
            m.merge(_turn(f,side*90,x,0,0))
    _lathe(m,(x,0),[(53.25,5.2),(55,5.9),(57,5.65),(60,4.4),(62.5,2.1),(63.4,1.25)],(180,138,87),12,(32,24,16)[lod])
    _finial(m,x,0,63.4,2.6,(172,135,70),lod)
    # Low halls east of the vimana; the Nandi pavilion is detached.
    for xx,ww,dd,hh in [(-28,36,22,13),(1,22,20,10),(20,16,17,8)]:
        m.box((xx,hh/2,0),(ww,hh,dd),OCHRE)
        _frustum(m,xx,0,hh,3,ww+1,dd+1,ww-4,dd-4,(184,143,92),12)
        for side in (-1,1):
            for ax in np.arange(xx-ww/2+2,xx+ww/2,3):m.box((ax,hh*.5,side*(dd/2+.16)),(.6,hh*.8,.5),(192,151,99),17)
    for xx in [51,61]:
        for zz in [-5,5]:m.box((xx,4,zz),(1,8,1),OCHRE)
    _frustum(m,56,0,8,3,16,15,9,7,(190,145,91),12)
    m.ellipsoid((56,2.6,0),(2.6,2.1,1.3),(87,76,59),17,(16,12,8)[lod],6)
    m.ellipsoid((58.3,3.6,0),(1.0,1.0,.85),(87,76,59),17,10,5)
    for x,w,d,height in ((115,20,13,30),(182,24,14,27)):
        gate=Mesh();_dravida_tower(gate,0,0,w,d,height,lod,tiers=5)
        m.merge(_turn(gate,90,x,0,0))
    m.box((182,-.2,0),(14,.4,24),OCHRE,STONE_WALL)
    m.box((150,-.3,0),(69,.6,34),(193,177,142),8)
    for xx,zz,ww,dd,hh in [(-93,39,18,14,21),(-33,40,40,16,15),(-88,-40,18,14,15)]:
        _dravida_tower(m,xx,zz,ww,dd,hh,lod,tiers=5)
    return m


def _meenakshi(lod):
    m=Mesh();stone=(153,149,130);roof=(194,190,171)
    # The historical plan is oriented west-up. Its cardinal gates agree with
    # current vectors after this metric transformation, so the asymmetry stays.
    def plan_point(px,py):return ((py-500)*.297,(px-526)*.297)
    def gallery(px0,py0,px1,py1,height=9):
        x,z=plan_point((px0+px1)/2,(py0+py1)/2);w=(py1-py0)*.297;d=(px1-px0)*.297
        # Granite corridors have exposed column ranks beneath limewashed roofs.
        # The recessed inner wall leaves real shaded depth behind each rank.
        m.box((x,height/2,z),(max(1,w-3),height,max(1,d-3)),stone,STONE_WALL)
        m.box((x,height+.18,z),(w+.25,.36,d+.25),roof,12)
        nx=max(1,round(w/(3.2 if lod<2 else 6.4)));nz=max(1,round(d/(3.2 if lod<2 else 6.4)))
        for j in range(nx):
            for k in range(nz):
                tone=(-8,-4,0,3,6,-2)[(j*7+k*3+int(x+z))%6]
                col=tuple(v+tone for v in roof)
                xx=x-w/2+j*w/nx;zz=z-d/2+k*d/nz
                _face(m,[(xx+.025,height+.367,zz+.025),(xx+w/nx-.025,height+.367,zz+.025),
                         (xx+w/nx-.025,height+.367,zz+d/nz-.025),(xx+.025,height+.367,zz+d/nz-.025)],col,FLAT_ROOF,(0,1,0))
        for side in (-1,1):
            m.box((x,height+.48,z+side*d/2),(w,.6,.4),(157,139,106),17)
        for side in range(4):
            f=Mesh();fw=w if side%2==0 else d;fd=d if side%2==0 else w
            count=max(2,round(fw/(3.5 if lod<2 else 7)))
            for j in range(count+1):
                xx=-fw/2+j*fw/count
                f.box((xx,.3,-fd/2+.4),(.84,.6,.84),stone,STONE_WALL)
                f.box((xx,(height-.4)/2,-fd/2+.4),(.54,height-1.0,.54),stone,STONE_WALL)
                f.box((xx,height-1.05,-fd/2+.4),(.92,.4,.92),stone,STONE_WALL)
                f.box((xx,height-.75,-fd/2+.4),(1.1,.5,1.0),stone,STONE_WALL)
                if lod==0 and j%3==0:_relief(f,xx,height*.45,-fd/2+.06,.9,lod,(162,159,140))
            f.box((0,height-.3,-fd/2+.4),(fw+.2,.6,1.05),(170,165,143),STONE_WALL)
            m.merge(_turn(f,side*90,x,0,z))
    m.box((0,-.3,-21),(269,.6,235),(181,169,143),8)
    # Perimeter galleries and the northeastern Thousand Pillar Hall.
    # Perimeter galleries stop at the four axial gates so their solid backs
    # cannot seal the open gopuram passages.
    for rect in [(60,50,504,75),(548,50,845,75),
                 (60,925,504,950),(548,925,845,950),
                 (60,75,83,398),(60,442,83,925),
                 (822,75,845,398),(822,442,845,925),
                 (576,697,822,925),(477,689,569,904),(328,686,410,793),
                 (234,840,330,925),(334,906,470,925),(136,160,218,366)]:gallery(*rect)
    # Sundareshwarar's prakara has separate roofed corridors around its court.
    for rect in [(396,173,725,231),(396,231,443,625),(621,231,725,625),
                 (443,507,621,625),(447,240,476,503),(585,240,620,503),
                 (476,240,585,267),(476,477,585,503),(478,302,564,402),
                 (475,409,578,468)]:gallery(*rect,height=10 if rect[0]<447 else 12)
    # Meenakshi's smaller southwestern precinct and its separate sanctum.
    for rect in [(223,156,394,200),(223,200,266,378),(344,200,394,378),
                 (266,342,344,378),(272,209,335,304),(308,434,391,664),
                 (111,399,308,435),(111,435,136,664),(136,638,308,664),
                 (113,664,307,706),(224,735,328,798)]:gallery(*rect)
    # The pool is a true roofless void east of Meenakshi, south of Sundareshwarar.
    px,pz=plan_point(217,539);pw,pd=50.3,36.6
    m.box((px,.4,pz),(pw,.3,pd),(76,123,105),1)
    for i in range(5):
        for sign in (-1,1):
            m.box((px+sign*(pw/2+i*.7),.55+i*.32,pz),(1.0,.42,pd+i*1.4),(188,163,117),8)
            m.box((px,.55+i*.32,pz+sign*(pd/2+i*.7)),(pw+i*1.4,.42,1.0),(188,163,117),8)
    _lathe(m,(px,pz),[(.6,.55),(1.7,.40),(2.0,1.1),(2.35,.65)],GOLD,17,(16,12,8)[lod])
    for j in range(8):
        a=TAU*j/8;m.ellipsoid((px+.7*math.cos(a),2.0,pz+.7*math.sin(a)),(.36,.2,.36),GOLD,17,8,4)
    # Four outer and ten smaller internal gopurams retain distinct positions.
    towers=[(526,50,32,20,48.8,90),(526,950,32,20,49.3,90),
            (60,420,32,20,51.9,0),(850,420,30,19,45.9,0),
            (526,173,20,13,30,90),(722,425,21,13,31,0),
            (526,630,20,13,31,90),(306,152,18,12,25,90),
            (306,387,18,12,26,90),(378,420,19,12,29,0),
            (292,753,22,14,30,90),(441,290,9,7,16,0),
            (393,592,9,7,15,0),(275,322,8,6,14,90)]
    for xp,yp,w,d,h,rot in towers:
        x,z=plan_point(xp,yp);f=Mesh()
        if yp in (50,950) or xp in (60,850):f.box((0,-1.2,0),(w,2.4,d),stone,STONE_WALL)
        _dravida_tower(f,0,0,w,d,h,lod,painted=True,tiers=9 if h>24 else 5);m.merge(_turn(f,rot,x,0,z))
    # Narrow approach ramps join the six sampled street points at the two
    # outer east/west portals without changing the gopuram floor elevations.
    for side,heights in [(-1,[-.500319,-.183959,.097611]),(1,[-.557525,-.338378,-.116714])]:
        for i in range(2):
            z0=-3+i*3;z1=z0+3
            top=[(side*147,heights[i],z0),(side*147,heights[i+1],z1),
                 (side*143.65,0,z1),(side*143.65,0,z0)]
            _face(m,top,(181,169,143),8,(0,1,0))
            for a,b in zip(top,top[1:]+top[:1]):
                _face(m,[(a[0],-2.6,a[2]),(b[0],-2.6,b[2]),b,a],stone,STONE_WALL,(a[0]+b[0]-side*290.65,0,a[2]+b[2]))
    for xp,yp,w in [(524,340,17),(306,246,13)]:
        x,z=plan_point(xp,yp)
        m.box((x,14,z),(w,4,w),(189,160,98))
        for i in range(4):
            _frustum(m,x,z,16+i*1.2,1.2,w-i*2.5,w-i*2.5,w-(i+1)*2.5,w-(i+1)*2.5,GOLD,12)
        _dome(m,(x,z),w*.18,2.2,20.8,GOLD,(20,16,12)[lod],6)
        _finial(m,x,z,23,1.5,GOLD,lod)
    return m


def _konark(lod):
    m=Mesh();stone=(153,123,86);light=(178,145,101)
    # The jagamohana is extant; the western deul remains a ruined foundation.
    for x,z,w,d,h in [(-14,0,65,35,3.5),(14,0,34,34,6),(-21,0,32,32,5),(84,0,28,27,3.8)]:
        m.box((x,h/2,z),(w,h,d),stone)
        m.box((x,h-.25,z),(w+.5,.5,d+.5),light,17)
    m.box((14,12.8,0),(28,13.6,28),stone)
    for face in range(4):
        f=Mesh();f.box((0,12.8,-14.8),(9.5,13.6,1.6),stone)
        _arch(f,(0,6,-15.65),6,10,.42,light,False,(18,12,8)[lod],(47,43,33))
        for xx in [-10,-6,6,10]:
            f.box((xx,12.3,-14.25),(.55,12.2,.65),light,17)
            if lod<2:_relief(f,xx,12,-14.75,1.8,lod,(168,136,94))
        m.merge(_turn(f,face*90,14,0,0))
    # Three distinct pidha groups are separated by tall sculptural recesses.
    for bottom,width,count,rise,inset in [(19.6,31,6,.90,1.3),(26.8,22.6,6,.72,1.2),(32.72,15.4,5,.77,1.3)]:
        for i in range(count):
            y=bottom+i*rise;w=width-i*inset
            _frustum(m,14,0,y,rise,w,w,w-inset,w-inset,stone)
            m.box((14,y+.14,0),(w+.50,.28,w+.50),light,17)
    for y,w in [(25,23.2),(31.12,15.4)]:
        m.box((14,y+.8,0),(w,1.6,w),stone)
        for side in range(4):
            f=Mesh()
            for xx in np.linspace(-w*.38,w*.38,(9,6,3)[lod]):_relief(f,float(xx),y+.75,-w/2-.18,1.25,lod,light)
            m.merge(_turn(f,side*90,14,0,0))
    _lathe(m,(14,0),[(36.76,4.25),(37.6,4.8),(38.2,4.2),(39.0,1.4)],stone,12,(32,24,16)[lod])
    # Twelve wheel positions per long face, explicitly modeled spokes and hubs.
    for side in (-1,1):
        for j in range(12):
            x=-41+j*5.1;z=side*17.85;cy=2.5;r=1.55
            _ring(m,(x,cy,z),r,.16,light,(24,16,12)[lod],17,'xy')
            m.beam((x,cy,z-.2),(x,cy,z+.2),.35,stone,17,(12,8,6)[lod])
            for k in range(8):
                a=TAU*k/8
                _tube(m,(x,cy,z),(x+r*math.cos(a),cy+r*math.sin(a),z),.09,light,17,4)
            if lod==0:
                for k in range(16):
                    a=TAU*k/16;m.ellipsoid((x+r*math.cos(a),cy+r*math.sin(a),z),(.14,.14,.18),(132,109,80),17,6,3)
    # Roofless nata mandira: pillars survive but no conjectural roof is added.
    for x in [75,81,87,93]:
        for z in [-9,-3,3,9]:
            h=7.1+.3*((int(x+z))%3)
            _lathe(m,(x,z),[(3.8,1.0),(4.5,1.0),(4.8,.68),(3.8+h,.68),(4.0+h,.92)],stone,11,8,math.pi/8)
            if lod<2:
                for yy in [5.5,7.5,9.5]:_relief(m,x,yy,z-.78,1.1,lod,light)
    # A short outer landing meets the sampled eastern approach, about0.27m
    # below the temple datum; the existing ten-metre stair remains unchanged.
    for z0,z1,y0,y1 in [(-4.5,0,-.273960,-.273957),(0,4.5,-.273957,-.257289)]:
        top=[(110,y0,z0),(110,y1,z1),(107,0,z1),(107,0,z0)]
        _face(m,top,light,8,(0,1,0))
        for a,b in zip(top,top[1:]+top[:1]):
            _face(m,[(a[0],-1.6,a[2]),(b[0],-1.6,b[2]),b,a],stone,STONE_WALL,(a[0]+b[0]-217,0,a[2]+b[2]))
    m.box((108.5,-.95,0),(3,1.3,9),stone,STONE_WALL)
    s=Mesh();s.box((0,-.8,5),(9,1.6,10),stone,STONE_WALL);_steps(s,0,0,9,10,3.8,0,(15,10,6)[lod],light);m.merge(_turn(s,-90,107,0,0))
    s=Mesh();_steps(s,0,0,8,13,6,0,(18,12,8)[lod],light);m.merge(_turn(s,-90,44,0,0))
    for i in range(14):
        x=-34+(i%4)*7;z=-11+(i//4)*7
        m.box((x,5.4,z),(4.0,1.6+(i%3)*.5,4.0),(135,112,79))
    for side in (-1,1):
        for x in np.arange(-45,29,2.7 if lod<2 else 5.4):
            _relief(m,x,4.5,side*17.65,1.15,lod,light)
    # Surface generation applies each bounded terrain patch datum per vertex.
    # Keep these independent groups relative to their own y=0 terrain plane.
    main=m;m=Mesh()
    # IGNCA inventory records the roofless Mayadevi shrine at 4.55m surviving
    # height. Its two cells and east platform replace only their mapped footprint.
    m.box((-82.6,.45,-30.8),(41,.9,19.4),stone)
    for cx,w in [(-94.5,13),(-78,16.5)]:
        d=13.2
        for side in (-1,1):
            m.box((cx,2.725,-30.8+side*(d/2-.9)),(w,3.65,1.8),stone)
            for xx in np.linspace(cx-w*.38,cx+w*.38,(7,5,3)[lod]):
                _relief(m,float(xx),2.5,-30.8+side*d/2,1.3,lod,light)
        m.box((cx-w/2+.8,2.725,-30.8),(1.6,3.65,d),stone)
        for side in (-1,1):m.box((cx+w/2-.8,2.725,-30.8+side*4.3),(1.6,3.65,4.6),stone)
        for sx in (-1,1):
            for sz in (-1,1):m.box((cx+sx*w*.38,2.7,-30.8+sz*(d/2+.45)),(2.0,3.6,.9),light,17)
    _steps(m,-65.5,-34,6,6,.9,0,(8,6,4)[lod],stone)
    main.merge(m);m=Mesh()
    # The excavated kitchen southeast of the dance hall is roofless, with
    # peripheral support stumps around its open working court (INTACH site plan).
    for side in (-1,1):
        m.box((78.7,.6,-61.2+side*12.8),(32.8,1.2,1.0),stone)
        m.box((78.7+side*15.9,.6,-61.2),(1.0,1.2,26.4),stone)
        for j in range(8):m.box((64.7+j*4.0,1.0,-61.2+side*9.6),(1.0,2.0,1.0),stone)
        for j in range(5):m.box((78.7+side*12.4,1.0,-69.0+j*4.0),(1.0,2.0,1.0),stone)
    main.merge(m)
    # The north elephant and south horse display plinths are sculpture, not
    # multistorey source buildings. Individual limbs keep their silhouettes legible.
    for x,z,kind in [(-7.1,45.4,'elephant'),(10.5,51.4,'elephant'),(10,-69.9,'horse'),(20,-69.9,'horse')]:
        m=Mesh()
        m.box((x,.7,z),(5.8,1.4,6.5),stone)
        m.ellipsoid((x,3.0,z),(1.25,1.3,2.0),light,17,12,6)
        for dx in (-.85,.85):
            for dz in (-1.2,1.2):_tube(m,(x+dx,1.3,z+dz),(x+dx,3,z+dz),.35 if kind=='elephant'else .20,stone,17,8)
        if kind=='elephant':
            m.ellipsoid((x,3.5,z+1.9),(1.0,1.0,.9),light,17,10,5)
            _tube(m,(x,3.4,z+2.55),(x+.3,1.5,z+2.85),.30,light,17,8)
        else:
            _tube(m,(x,3.2,z+1.2),(x,4.9,z+1.9),.48,light,17,10)
            m.ellipsoid((x,5,z+2.25),(.5,.6,.85),light,17,10,5)
        main.merge(m)
    return main


def _akshardham(lod):
    m=Mesh();pink=(197,158,126);light=(222,187,154)
    m.box((0,.95,0),(108.51,2.7,96.32),pink,8)
    for y,w,d in [(2.7,103,92),(3.2,100,89),(4.0,98,87)]:m.box((0,y,0),(w,.6,d),light,17)
    peeth=m;m=Mesh()
    # The mandovar is a7.62m sculpted wall, not several floors of windows.
    # Roof samvarans and column-supported mandapams rise separately above it.
    for x,z,w,d in [(0,0,49,65),(-33,0,27,40),(33,0,27,40),(0,-36,38,21),(0,36,38,21)]:
        m.box((x,7.81,z),(w,7.62,d),pink)
        for side in range(4):
            f=Mesh();fw=w if side%2==0 else d;fd=d if side%2==0 else w
            count=max(3,round(fw/3.15));bay=fw/count
            for j in range(count):
                xx=-fw/2+(j+.5)*bay;depth=-fd/2-.2
                # Sculptural niches contain figures and project from tiered mouldings.
                _arch(f,(xx,6.55,depth),bay*.57,3.65,.19,light,True,(14,10,8)[lod],(160,120,90))
                _relief(f,xx,8.2,depth-.28,1.65,lod,light)
                for dx in (-bay*.44,bay*.44):
                    _lathe(f,(xx+dx,depth),[(6.4,.28),(6.7,.4),(7,.27),(9.8,.23),(10,.4),(10.3,.34)],light,17,8)
                if lod==0:
                    for yy in (4.75,5.55,10.7):
                        _relief(f,xx,yy,depth-.16,.62,lod,(187,146,111))
            for yy,th,projection in [(4.25,.24,.3),(4.8,.16,.42),(5.6,.22,.32),(6.25,.35,.55),(10.3,.28,.48),(10.9,.22,.65),(11.45,.36,.85)]:
                f.box((0,yy,-fd/2-projection/2),(fw+.25,th,projection),light,17)
            m.merge(_turn(f,side*90,x,0,z))
        # A terraced roof mass ends below the separate domes and their drums.
        for j in range(5):
            ww=w+1-j*w*.105;dd=d+1-j*d*.105;y=11.65+j*1.2
            _frustum(m,x,z,y,1.2,ww,dd,ww-w*.105,dd-d*.105,pink)
            m.box((x,y+.15,z),(ww+.4,.25,dd+.4),light,17)
    # Front axial porch has genuine open ranks of carved pillars.
    porch=Mesh()
    for xx in (-8,-2.7,2.7,8):
        for zz in (-47.7,-44.2):
            _lathe(porch,(xx,zz),[(4,.7),(4.5,.8),(5,.52),(6,.46),(10.5,.40),(11.1,.75),(11.6,.72)],light,17,12)
            if lod==0:
                for yy in (5.5,7.2,8.9):
                    for a in range(4):
                        f=Mesh();_relief(f,0,yy,-.48,.85,lod,light);porch.merge(_turn(f,a*90,xx,0,zz))
    porch.box((0,11.9,-46),(19,1.0,6.5),light,17)
    for yy in range(5):
        w=20-yy*2.6;_frustum(porch,0,-46,12.4+yy*.85,.85,w,7.0-yy*.6,w-2.2,6.4-yy*.6,pink)
    m.merge(porch)
    # Nine distinct dome centers: central sanctum, axial halls, corner mandapams.
    domes=[(0,0,12.1,27.6,11.3),(-28,0,8.6,20,9),(28,0,8.6,20,9),(0,-29,8.3,21,9),(0,29,8.3,21,9),(-18,-23,5.4,22,6),(-18,23,5.4,22,6),(18,-23,5.4,22,6),(18,23,5.4,22,6)]
    for x,z,r,y,h in domes:
        _lathe(m,(x,z),[(y-4,r*.67),(y-.55,r*.67)],pink,11,(24,16,12)[lod])
        for j in range(12 if r>8 else 8):
            a=TAU*j/(12 if r>8 else 8);cx=x+r*.84*math.cos(a);cz=z+r*.84*math.sin(a)
            _lathe(m,(cx,cz),[(y-4,.34),(y-3.65,.45),(y-.8,.24),(y-.5,.46)],light,17,8)
        _lathe(m,(x,z),[(y-.55,r*1.03),(y,r*1.03)],light,17,(32,24,16)[lod])
        profile=[(y,r),(y+h*.15,r*.97),(y+h*.40,r*.82),(y+h*.64,r*.58),(y+h*.82,r*.3),(y+h,.12)]
        _lathe(m,(x,z),profile,(211,172,137),12,(40,28,20)[lod])
        for yy in np.linspace(y+.45,y+h*.85,(12,9,6)[lod]):
            rr=float(np.interp(yy,[p[0]for p in profile],[p[1]for p in profile]))
            _lathe(m,(x,z),[(yy-.07,rr+.16),(yy+.13,rr+.16)],light,17,(32,24,16)[lod])
            if lod==0:
                for j in range(24 if r>8 else 16):
                    a=TAU*j/(24 if r>8 else 16);m.ellipsoid((x+(rr+.2)*math.cos(a),yy+.25,z+(rr+.2)*math.sin(a)),(.22,.36,.22),light,17,6,3)
        if lod<2:
            for j in range(16):
                a=TAU*j/16
                for (yy,rr),(yyy,rrr) in zip(profile,profile[1:]):_tube(m,(x+rr*math.cos(a),yy,z+rr*math.sin(a)),(x+rrr*math.cos(a),yyy,z+rrr*math.sin(a)),.14,light,17,5)
        _finial(m,x,z,y+h,4.168 if x==z==0 else 2.0,(178,139,85),lod)
    for j in range(20):
        # Four side runs retain the quadrangular form of the 20 small shikhars.
        side=j//5;k=j%5-2
        x,z=((k*12,-37),(43,k*8),(k*12,37),(-43,k*8))[side]
        for i in range(4):
            y=17+i*1.5;w=5.5-i
            _frustum(m,x,z,y,1.5,w,w,w-.9,w-.9,pink)
            m.box((x,y+.14,z),(w+.3,.25,w+.3),light,17)
        _finial(m,x,z,23,1.3,GOLD,lod)
    # The published overall dimensions include the broad Gajendra Peeth; the
    # narrower roofed mandir follows the independently mapped72.7x63.6m body.
    m.transform([[.78,0,0,0],[0,1,0,0],[0,0,.68,0],[0,0,0,1]]);peeth.merge(m);m=peeth
    # The Gajendra Peeth animal frieze reads as individual elephant forms.
    for face in range(4):
        f=Mesh();width=108.51 if face%2==0 else 96.32;depth=96.32 if face%2==0 else 108.51
        for j in range((24,16,9)[lod]):
            x=(j-((24,16,9)[lod]-1)/2)*(width-6)/(24,16,9)[lod]
            f.ellipsoid((x,1.55,-depth/2-.2),(1.1,.7,.40),light,17,8,4)
            f.ellipsoid((x+.7,1.7,-depth/2-.35),(.5,.5,.35),pink,17,6,3)
            if lod==0:_tube(f,(x+1.0,1.6,-depth/2-.35),(x+1.3,.6,-depth/2-.4),.17,pink,17,5)
        m.merge(_turn(f,face*90))
    _steps(m,0,-48.1,18,14,4.1,0,(22,15,9)[lod],light)
    return m


BIETE_CROSS=[(-2.1,-6.3),(2.1,-6.3),(2.1,-2.1),(6.3,-2.1),(6.3,2.1),(2.1,2.1),(2.1,6.3),(-2.1,6.3),(-2.1,2.1),(-6.3,2.1),(-6.3,-2.1),(-2.1,-2.1)]
BIETE_PIT=[(-13.8,-13.2),(13.0,-13.8),(13.8,12.5),(-13.0,13.6)]


def _biete(lod):
    import json
    from pathlib import Path
    m=Mesh();rock=(151,108,77);light=(177,126,86)
    # y=0 is uncut ground, not the excavated floor. Runtime must cut this pit.
    _polygon_prism(m,BIETE_PIT,-12.6,.12,(121,95,72),8)
    sampled=json.loads((Path(__file__).parent/'references/temples/biete-giyorgis-rim-ground.json').read_text())
    edge_count=12
    rim_inner=[];rim_outer=[]
    for i,a in enumerate(BIETE_PIT):
        b=BIETE_PIT[(i+1)%4]
        for j in range(edge_count):
            t=j/edge_count;x=a[0]*(1-t)+b[0]*t;z=a[1]*(1-t)+b[1]*t
            xx,zz=x*1.045,z*1.045
            rim_inner.append(np.array((xx,.07555537*xx+.20432129*zz-.17042057,zz)))
            outer_y=sampled['ground_samples'][i*12+j*12//edge_count]['ground_m']-2420.681
            rim_outer.append(np.array((x*2.35,outer_y,z*2.35)))
    # The exact court walls hide a wider, lower terrain-grid clearance. The
    # surrounding rock joins independently sampled ground at its outer edge.
    for j,top in enumerate(rim_inner):
        nxt=rim_inner[(j+1)%len(rim_inner)]
        # Neighboring streamed cells can use different LODs. The excavation
        # boundary must therefore retain identical relief vertices at all LODs.
        wall_rows=8
        def wall_point(index,k):
            index%=len(rim_inner)
            high=rim_inner[index]
            bottom=np.array((high[0]/1.045,-12.6,high[2]/1.045))
            t=k/wall_rows
            point=bottom*(1-t)+high*t
            relief=.10*math.sin(math.pi*t)*math.sin(index*2.4+k*1.7)
            point[[0,2]]+=point[[0,2]]/np.linalg.norm(point[[0,2]])*relief
            return point
        for k in range(wall_rows):
            points=[wall_point(j,k),wall_point(j+1,k),wall_point(j+1,k+1),wall_point(j,k+1)]
            tone=tuple(int(c+1.5*math.sin(j*1.7+k*2.3))for c in rock)
            _face(m,points,tone,EARTHEN_PLASTER,(-top[0],0,-top[2]))
        outer=rim_outer[j];outer_next=rim_outer[(j+1)%len(rim_outer)]
        # Boundary anchors are identical at every LOD; the buried skirt closes
        # terrain interpolation gaps between those measured points.
        bottom=outer-np.array((0,3.0,0));bottom_next=outer_next-np.array((0,3.0,0))
        _face(m,[bottom,bottom_next,outer_next,outer],(185,180,151),8,(outer[0],0,outer[2]))
        for k in range(64):
            s,t=k/64,(k+1)/64
            points=[top*(1-s)+outer*s,nxt*(1-s)+outer_next*s,nxt*(1-t)+outer_next*t,top*(1-t)+outer*t]
            blend=(s+t)/2
            tone=tuple(round(a*(1-blend)+b*blend)for a,b in zip(rock,(185,180,151)))
            _face(m,points,tone,EARTHEN_PLASTER)
    for s,y,h in [(1.15,-12.48,.48),(1.10,-12,.45),(1.05,-11.55,.45),(1,-11.1,11.5)]:
        _polygon_prism(m,[(x*s,z*s) for x,z in BIETE_CROSS],y,h,light if s>1 else rock)
    for s,y in [(1.015,-9.8),(1.01,-1.4),(1.02,-.3)]:
        for i,a in enumerate(BIETE_CROSS):
            b=BIETE_CROSS[(i+1)%12];_tube(m,(a[0]*s,y,a[1]*s),(b[0]*s,y,b[1]*s),.14,light,17,4)
    for side in range(4):
        f=Mesh()
        for x in [-1.05,1.05]:
            _arch(f,(x,-4.2,-6.43),.65,1.5,.12,light,True,(12,8,6)[lod],(46,37,28))
            f.box((x,-7.0,-6.35),(.65,1.0,.14),(133,91,62),17)
        if side==3:_arch(f,(0,-11.1,-6.44),1.35,2.9,.17,light,False,(14,10,6)[lod],(42,35,26))
        m.merge(_turn(f,side*90))
    # Three nested cruciform roof mouldings are part of the carved monolith.
    for s,y in [(.80,.45),(.64,.60),(.47,.74)]:
        for i,a in enumerate(BIETE_CROSS):
            b=BIETE_CROSS[(i+1)%12];_tube(m,(a[0]*s,y,a[1]*s),(b[0]*s,y,b[1]*s),.16,light,17,4)
    return m


def _onion(m,x,z,r,y,h,lod,color=MARBLE):
    _lathe(m,(x,z),[(y,r*.83),(y+h*.12,r*.94),(y+h*.30,r),(y+h*.50,r*.91),(y+h*.69,r*.68),(y+h*.84,r*.35),(y+h,.05)],color,12,(48,32,20)[lod])


def _chhatri(m,x,z,y,r,h,lod):
    _lathe(m,(x,z),[(y,r),(y+.45,r)],MARBLE,17,(20,16,12)[lod])
    for j in range(8):
        a=TAU*j/8;m.box((x+r*.77*math.cos(a),y+h*.24,z+r*.77*math.sin(a)),(.35,h*.48,.35),MARBLE,17)
    _onion(m,x,z,r*1.05,y+h*.48,h*.38,lod)
    _finial(m,x,z,y+h*.86,h*.14,GOLD,lod)


def _taj_sidehall(lod):
    m=Mesh();red=(164,76,57)
    m.box((0,9.5,0),(23,19,56),red)
    for z in [-17,0,17]:
        _lathe(m,(0,z),[(19,7),(22.5,7)],MARBLE,11,(24,16,12)[lod])
        _onion(m,0,z,8,22.5,8,lod)
        _finial(m,0,z,30.5,1.5,GOLD,lod)
    for side in range(4):
        f=Mesh();width=23 if side%2==0 else 56;depth=56 if side%2==0 else 23
        for j in range(3 if side%2==0 else 9):
            count=3 if side%2==0 else 9;x=(j-(count-1)/2)*(width-2)/count
            _arch(f,(x,2,-depth/2-.08),width/count*.68,12,.32,MARBLE,True,(14,10,6)[lod],(73,46,38))
        m.merge(_turn(f,side*90))
    for x in [-10.5,10.5]:
        for z in [-27,27]:_chhatri(m,x,z,19,1.5,5,lod)
    return m


def _taj(lod):
    m=Mesh();red=(166,87,65)
    m.box((1,-5,0),(306,10,111),red,8)
    north=Mesh()
    for xx in np.arange(-140,141,10):_arch(north,(xx,-8,-55.57),4.0,5.5,.14,(150,78,60),True,(12,8,6)[lod],(119,72,56))
    m.merge(_turn(north,180))
    m.box((0,3.35,0),(95.4024,6.7,95.4024),MARBLE,8)
    clipped=[(-20,-28.5),(20,-28.5),(28.5,-20),(28.5,20),(20,28.5),(-20,28.5),(-28.5,20),(-28.5,-20)]
    _polygon_prism(m,clipped,6.7,28,MARBLE)
    _polygon_prism(m,[(x*1.016,z*1.016) for x,z in clipped],33.9,.8,(220,217,202),17)
    for side in range(4):
        f=Mesh()
        # Tall central pishtaq and paired stacked arches are individually framed.
        f.box((0,21,-28.66),(19.1,28.6,.35),(231,228,213))
        _arch(f,(0,6.7,-28.92),14.8,24.8,.45,(104,101,91),True,(26,18,12)[lod],(105,104,96))
        _arch(f,(0,7,-29.08),13.2,23.5,.2,MARBLE,True,(24,16,10)[lod],None)
        for x in [-22.5,22.5]:
            for y in [7.2,19.1]:_arch(f,(x,y,-26.1),5.3,10.7,.19,(172,167,151),True,(16,12,8)[lod],(125,125,115))
        for x in [-9.5,9.5]:
            f.box((x,20.7,-29.14),(.34,27.3,.13),(91,89,78),17)
            if lod==0:
                for yy in np.arange(8,33,1.1):f.box((x,yy,-29.23),(.35,.19,.09),(214,209,192),17)
        if lod<2:
            # Pietra dura flowers distinguish panels from blank white walls.
            for x in [-17.5,-14.8,14.8,17.5]:
                for y in [9,12.5,16]:
                    _tube(f,(x,y,-28.69),(x,y+1.25,-28.69),.06,(75,99,75),17,4)
                    for dx,dy in [(-.24,.4),(.24,.7),(0,1.25)]:f.ellipsoid((x+dx,y+dy,-28.74),(.17,.20,.08),(164,80,65),17,6,3)
        m.merge(_turn(f,side*90))
    for j in range(4):
        f=Mesh();_arch(f,(0,7.1,-34.44),6.8,11,.18,(174,170,156),True,(16,12,8)[lod],(119,120,111))
        _arch(f,(0,19.1,-34.44),6.8,10.7,.18,(174,170,156),True,(16,12,8)[lod],(119,120,111))
        m.merge(_turn(f,45+j*90))
    _lathe(m,(0,0),[(34.7,12.0),(41.3,12.0),(41.7,12.6)],MARBLE,11,(48,32,20)[lod])
    _onion(m,0,0,17.5,41.7,25.5,lod)
    _finial(m,0,0,67.2,5.8,(143,118,64),lod)
    for x in [-20,20]:
        for z in [-20,20]:_chhatri(m,x,z,34.7,4.25,12.2,lod)
    for x in [-43.2,43.2]:
        for z in [-43.2,43.2]:
            top=6.7+41.7576
            _lathe(m,(x,z),[(6.7,3.05),(8,3.05),(18,2.63),(30,2.23),(40,1.89)],MARBLE,11,(32,24,16)[lod])
            for y,r in [(18,3.2),(30,2.9),(40,2.65)]:
                _lathe(m,(x,z),[(y-.35,r),(y+.1,r),(y+.75,r*.98)],(220,217,203),17,(32,24,16)[lod])
                if lod<2:
                    for k in range(16):
                        a=TAU*k/16;m.box((x+r*.91*math.cos(a),y+.6,z+r*.91*math.sin(a)),(.16,.9,.16),MARBLE,17)
            _chhatri(m,x,z,40.75,2.2,top-40.75,lod)
    m.merge(_turn(_taj_sidehall(lod),0,-137,0,-2.8));m.merge(_turn(_taj_sidehall(lod),180,141,0,-1.4))
    # River parapet follows the north terrace; south stair meets the garden.
    for x in [-151,153]:m.box((x,1.5,0),(1.5,3,110),red)
    m.box((1,1.5,55),(305,3,1.5),red)
    m.box((0,-1.75,-53),(15,3.5,14),red,STONE_WALL)
    _steps(m,0,-60,15,14,6.7,0,(20,12,8)[lod],MARBLE)
    # Lower garden approach samples span161.594–163.467m across the15m width.
    # The short sloping landing joins those heights before the level risers.
    outer=[(-7.5,-2.806235),(0,-1.944815),(7.5,-.933081)]
    for (x0,y0),(x1,y1) in zip(outer,outer[1:]):
        top=[(x0,y0,-66),(x1,y1,-66),(x1,-1.8,-64.5),(x0,-1.8,-64.5)]
        _face(m,top,red,8,(0,1,0))
        for a,b in zip(top,top[1:]+top[:1]):
            _face(m,[(a[0],-4,a[2]),(b[0],-4,b[2]),b,a],red,STONE_WALL,(a[0]+b[0],0,a[2]+b[2]+130.5))
    for i in range(12):
        top=-1.8+(i+1)*.15
        m.box((0,(-4+top)/2,-64.5+(i+.5)*.375),(15,top+4,.395),red,8)
    return m


def _sagrada_lantern(m,x,z,y0,y1,r0,r1,sides,levels,lod,color):
    # Each double-curved tensioned-stone panel frames a real diamond opening.
    # The paraboloid sides and construction levels follow the foundation booklet.
    levels=max(4,levels//(2 if lod==2 else 1))
    def point(u,y,sector,inset=0):
        t=(y-y0)/(y1-y0);r=r1+(r0-r1)*math.sqrt(max(0,1-t))
        a=TAU*(sector+u)/sides
        return np.array((x+(r-inset)*math.cos(a),y,z+(r-inset)*math.sin(a)))
    for j in range(levels):
        low=y0+(y1-y0)*j/levels;high=y0+(y1-y0)*(j+1)/levels
        for i in range(sides):
            a,b,c,d=point(0,low,i),point(1,low,i),point(1,high,i),point(0,high,i)
            ib=point(.5,low+(high-low)*.14,i);ir=point(.68,(low+high)/2,i)
            it=point(.5,low+(high-low)*.86,i);il=point(.32,(low+high)/2,i)
            normal=(math.cos(TAU*(i+.5)/sides),.15,math.sin(TAU*(i+.5)/sides))
            tone=(-7,-3,2,6,0)[(i+3*j)%5];shade=tuple(v+tone for v in color)
            for tri in [(a,b,ib),(b,ir,ib),(b,c,ir),(c,it,ir),(c,d,it),(d,il,it),(d,a,il),(a,ib,il)]:
                _face(m,tri,shade,11,normal)
            glass=[p-np.array(normal)*.12 for p in [ib,ir,it,il]]
            _face(m,glass,(72,91,91),15,normal)
            if lod<2:
                for p,q in [(a,d),(a,b)]:_tube(m,p,q,.08,(231,221,197),17,4)


def _sagrada_spire(m,x,z,height,lod,color,central=False):
    n=(32,24,16)[lod];base=5.3
    if central:
        if central=='jesus':
            # The crossing room supports the first tower panels at85m.
            _sagrada_lantern(m,x,z,55,85,15.0,9.0,12,6,lod,color)
            _sagrada_lantern(m,x,z,85,143,9.0,3.7,12,12,lod,color)
            _lathe(m,(x,z),[(143,3.7),(147,3.4),(150,2.7),(155.5,2.0)],color,11,12)
            for j in range(12):
                a=TAU*j/12
                _tube(m,(x+3.8*math.cos(a),143,z+3.8*math.sin(a)),(x+2.1*math.cos(a),155,z+2.1*math.sin(a)),.15,(225,221,203),TILE_ROOF,5)
        elif central=='mary':
            _sagrada_lantern(m,x,z,45,108,9.0,3.25,14,19,lod,color)
            _lathe(m,(x,z),[(108,3.25),(111,3.8),(114,2.3)],color,11,12)
            # Eighteen-metre hyperboloid lantern overlaps the six-metre crown.
            _lathe(m,(x,z),[(112.5,2.7),(118,1.7),(124,1.2),(130.5,1.7)],(206,217,216),TILE_ROOF,12)
            for j in range(12):
                a=TAU*j/12
                for yy,rr in [(109,3.3),(112,3.6)]:
                    _tube(m,(x+rr*math.cos(a),yy,z+rr*math.sin(a)),(x+(rr+.3)*math.cos(a),yy+1.7,z+(rr+.3)*math.sin(a)),.10,(81,89,85),SCULPTURE_METAL,5)
            for j in range(3):
                a=TAU*j/3
                _tube(m,(x+1.7*math.cos(a),127,z+1.7*math.sin(a)),(x+.8*math.cos(a),132,z+.8*math.sin(a)),.22,(222,228,219),TILE_ROOF,6)
        else:
            # Official tower division: open eight-rib base45–74, body74–112,
            # terminal22.5m. Continuous solid drums would erase the skylights.
            for j in range(8):
                a=TAU*j/8
                _tube(m,(x+5.5*math.cos(a),45,z+5.5*math.sin(a)),(x+4.8*math.cos(a),74,z+4.8*math.sin(a)),.42,color,11,8)
            for yy,rr in [(45,5.5),(59,5.15),(74,4.8)]:
                _lathe(m,(x,z),[(yy,rr),(yy+.45,rr),(yy+.45,rr-.4),(yy,rr-.4)],color,11,8)
            _sagrada_lantern(m,x,z,74,112,4.8,1.65,8,13,lod,color)
            _lathe(m,(x,z),[(112,1.65),(117,1.3),(121,1.15)],(205,211,199),TILE_ROOF,8)
            m.ellipsoid((x,122.8,z),(2.0,2.3,2.0),(222,223,210),TILE_ROOF,5,3)
        return
    _lathe(m,(x,z),[(0,base),(25,base),(45,base*.91),(height-15,base*.49),(height-7,base*.27)],color,11,n)
    for y in np.arange(31,height-14,3.3 if lod<2 else 6.6):
        r=float(np.interp(y,[25,45,height-15],[base,base*.91,base*.49]))
        for j in range(12):
            a=TAU*j/12
            # Slender dark openings preserve the lattice tower texture.
            rr=r+.10
            f=Mesh();_arch(f,(0,y,-rr),.75,2.5,.08,color,True,(10,8,6)[lod],(75,69,53))
            m.merge(_turn(f,math.degrees(a),x,0,z))
    _lathe(m,(x,z),[(height-7,base*.25),(height-5,base*.40),(height-3,base*.32),(height-1,base*.30),(height,.02)],(198,177,117),17,n//2)
    for k,(yy,rr) in enumerate([(height-5.5,1.9),(height-3.7,1.7),(height-1.8,1.4)]):
        for j in range(6):
            a=TAU*j/6;m.ellipsoid((x+rr*.55*math.cos(a),yy,z+rr*.55*math.sin(a)),(.45,.8,.45),[(162,71,55),(83,148,145),(213,192,93)][(k+j)%3],17,6,3)



def _sagrada(lod):
    m=Mesh();new=(213,193,157);old=(161,135,95)
    # +z apse, +x Nativity; heading -45 places the long axis northwest/southeast.
    m.box((0,-1.6,0),(72,3.2,109),(195,185,161),8)
    m.box((0,15,-9),(45,30,76),new)
    m.box((0,31,-8),(18,32,74),(209,190,156))
    m.box((0,22.5,11),(60,45,30),new)
    for side in (-1,1):
        # Side-nave roofs sit above the30m vaults. A45m inner edge would
        # bury the central clerestory and invent a continuous steep Gothic roof.
        for z0 in np.arange(-47,6,7.5):
            z1=min(z0+7.5,6)
            for k in range((8,5,3)[lod]):
                u=k/(8,5,3)[lod];v=(k+1)/(8,5,3)[lod]
                def p(q,z):return (side*(22.5-13.5*q),30+4*q+1.4*math.sin(math.pi*q),z)
                tone=(-4,0,3,5)[(k+int((z0+47)/7.5))%4]
                _face(m,[p(u,z0),p(u,z1),p(v,z1),p(v,z0)],tuple(c+tone for c in(190,178,153)),STONE_WALL,(side,1,0))
            # Roof lanterns occupy the individual nave bays.
            xx=side*16;zz=(z0+z1)/2
            _lathe(m,(xx,zz),[(33.0,1.6),(35.3,1.45),(37.1,.9),(38,.05)],(195,183,159),11,(12,10,8)[lod])
            for angle in (0,90,180,270):
                f=Mesh();_arch(f,(0,33.6,-1.49),.72,1.65,.10,new,True,(12,10,6)[lod],(80,97,94));m.merge(_turn(f,angle,xx,0,zz))
        # The central attic reaches54m, with stone panels divided into bays.
        for z0 in np.arange(-47,6,3.75):
            z1=min(z0+3.75,6)
            for k in range((5,4,2)[lod]):
                u=k/(5,4,2)[lod];v=(k+1)/(5,4,2)[lod]
                def p(q,z):return (side*9*(1-q),47+7*math.sin(q*math.pi/2),z)
                _face(m,[p(u,z0),p(u,z1),p(v,z1),p(v,z0)],(204+(k%2)*5,192+(k%2)*5,169+(k%2)*5),STONE_WALL,(side,1,0))
        for zz in np.arange(-40,1,7.5):
            f=Mesh();_arch(f,(0,32,-9.12),3.8,12,.23,new,True,(16,12,8)[lod],(88,110,105));m.merge(_turn(f,side*90,0,0,zz))
    _lathe(m,(0,32),[(0,22.5),(31,22.5)],new,11,14,math.pi/14)
    _lathe(m,(0,32),[(31,23),(44,16),(54,8)],new,12,14,math.pi/14)
    for side in (-1,1):
        for z in np.arange(-40,5,7.5):
            f=Mesh();_arch(f,(0,5,-22.7),4.3,22,.37,new,True,(18,12,8)[lod],(78,108,111) if side==1 else (139,89,58))
            m.merge(_turn(f,side*90,0,0,z))
            m.box((side*23.2,17,z),(1.1,34,1.4),new,17)
            # Ceramics on nave pinnacles use fruit clusters, not uniform stone.
            _lathe(m,(side*22.5,z),[(31,1.15),(39,.35),(43,.12)],new,17,8)
            for j in range(3):
                m.ellipsoid((side*22.5+(.4 if j%2 else -.4),41+j*.6,z),(.6,.85,.6),[(185,101,58),(128,161,90),(201,173,87)][j],17,8,4)
    for a in np.linspace(-math.pi*.15,math.pi*1.15,9):
        x=21.8*math.cos(a);z=32+21.8*math.sin(a)
        m.box((x,18,z),(1.3,36,1.3),new)
        _lathe(m,(x,z),[(34,1.3),(42,.5),(48,.05)],new,17,8)
    # Only the existing Nativity and Passion apostle groups are represented.
    for side in (-1,1):
        color=old if side==1 else new
        heights=[98.4,107,107,98.4] if side==1 else [107.4,112.2,112.2,107.4]
        for z,h in zip([-6.5,4.3,17.7,28.5],heights):_sagrada_spire(m,side*28,z,h,lod,color)
        facade=Mesh()
        for x,h in [(-12,25),(0,35),(12,25)]:
            _arch(facade,(x,0,-31),10,20,.65,color,True,(24,16,10)[lod],(62,56,44))
            _frustum(facade,x,-29,20,h-20,11,6,.5,.8,color)
            if side==1:
                for j in range((10,6,3)[lod]):
                    yy=8+j*2.1;span=4.5*(1-j/13)
                    for s in (-1,1):_relief(facade,x+s*span,yy,-32.1,1.5,lod,(173+(j%3)*6,146+(j%3)*5,102+(j%3)*4))
        if side==-1:
            for x in [-18,-11,-4,4,11,18]:_tube(facade,(x,0,-35),(x*.73,22,-31),.65,new,17,8)
            for x in np.linspace(-19,19,18):_tube(facade,(x,22,-31),(x*.6,30,-27),.36,new,17,6)
        else:
            _lathe(facade,(0,-29),[(29,2.0),(40,1.4),(45,.1)],(73,111,67),17,10)
        m.merge(_turn(facade,-90 if side==1 else 90,0,0,11))
    # Central Jesus tower completed in February 2026; the white cross is 17 m.
    _sagrada_spire(m,0,11,155.5,lod,new,'jesus')
    _lathe(m,(0,11),[(149.5,2.5),(155.5,2.0)],MARBLE,17,12)
    m.beam((0,155.5,11),(0,172.5,11),2.35,MARBLE,TILE_ROOF,8)
    m.beam((-6.75,164,11),(6.75,164,11),2.35,MARBLE,TILE_ROOF,8)
    m.beam((0,164,4.25),(0,164,17.75),2.35,MARBLE,TILE_ROOF,8)
    # Mary above apse; glass twelve-point star reaches 138 metres.
    _sagrada_spire(m,0,35,130.5,lod,(220,206,179),'mary')
    center=np.array([0,134.25,35]);phi=(1+math.sqrt(5))/2
    directions=[]
    for a in (-1,1):
        for b in (-1,1):directions.extend([(0,a,b*phi),(a,b*phi,0),(b*phi,0,a)])
    # The glass star has twelve spatial points, not a planar asterisk.
    tilt=math.atan(phi);co,si=math.cos(tilt),math.sin(tilt)
    for direction in directions:
        radial=np.asarray(direction,float);radial/=np.linalg.norm(radial)
        radial=np.array([radial[0],co*radial[1]+si*radial[2],-si*radial[1]+co*radial[2]])
        side=np.cross(radial,[0,1,0] if abs(radial[1])<.9 else [1,0,0]);side/=np.linalg.norm(side);up=np.cross(radial,side)
        base=[center+radial*1.15+.68*(math.cos(TAU*j/5)*side+math.sin(TAU*j/5)*up) for j in range(5)]
        tip=center+radial*3.75
        for j in range(5):_face(m,[base[j],base[(j+1)%5],tip],(220,232,226) if j%2 else (197,216,218),15,radial)
    for x,z in [(-12.5,-1.5),(12.5,-1.5),(-12.5,23.5),(12.5,23.5)]:
        _sagrada_spire(m,x,z,131,lod,new,'evangelist')
        _lathe(m,(x,z),[(125,.8),(130,1.3)],MARBLE,17,10)
        # Wings + compact symbolic figure occupy the 135 m tetramorph terminals.
        m.ellipsoid((x,133.7,z),(.8,1.3,.7),MARBLE,17,10,5)
        _face(m,[(x,125.2,z),(x-3,134.2,z),(x-2.4,130.2,z-.6)],MARBLE,17,(0,0,-1))
        _face(m,[(x,125.2,z),(x+3,134.2,z),(x+2.4,130.2,z-.6)],MARBLE,17,(0,0,-1))
    # Glory facade remains under construction; no four future towers or steps
    # extending across Carrer de Mallorca are invented.
    # Existing inner enclosure and bronze doors precede the future narthex.
    # The foundation gives the central door as5m wide and4.7m tall.
    for x in (-17.5,-12,-6.5,0,6.5,12,17.5):
        w=5.0 if x==0 else 3.4
        m.box((x,2.35,-47.16),(w,4.7,.16),(78,77,57),SCULPTURE_METAL)
        for dx in (-w/2,w/2):m.box((x+dx,2.65,-47.24),(.24,5.3,.35),new,WEATHERED_LIMESTONE)
        for yy in (10,20):
            _arch(m,(x,yy,-47.23),3.4,6.3,.20,new,True,(16,12,8)[lod],(83,110,105))
    for yy in (5.5,16.9,28.0):m.box((0,yy,-47.20),(45,.4,.6),new,WEATHERED_LIMESTONE)
    # Eight branching support columns are under construction. The eventual
    # bell towers and16 hanging narthex lanterns are deliberately absent.
    for x in (-19,-15,-9,-5,5,9,15,19):
        _lathe(m,(x,-51.7),[(0,1.1),(1.2,1.2),(21,.72),(24,1.0)],(193,183,159),11,8)
        for dx in (-1.15,1.15):_tube(m,(x,22,-51.7),(x+dx,29.4,-50.4),.45,new,11,8)
    for x in (-14.5,14.5):
        for z in np.arange(-40,3,7.5):
            _tube(m,(x,35,z),(x,35,z+5.8),.08,(119,128,126),METAL_ROOF,5)
    return m


_BUILDERS={
    'borobudur':_borobudur,'shwedagon-pagoda':_shwedagon,'wat-arun':_wat_arun,
    'potala-palace':_potala,'paro-taktsang':_paro,'prambanan':_prambanan,
    'brihadisvara-temple':_brihadisvara,'meenakshi-amman-temple':_meenakshi,
    'konark-sun-temple':_konark,'swaminarayan-akshardham':_akshardham,
    'biete-giyorgis':_biete,'taj-mahal':_taj,'sagrada-familia':_sagrada,
}



def _physical_materials(name,m):
    masonry={11:STONE_WALL,12:STONE_WALL,15:4,17:STONE_WALL}
    palette={}
    if name in ('borobudur','prambanan','brihadisvara-temple','konark-sun-temple'):
        mapping=masonry
    elif name=='biete-giyorgis':
        # The church is carved from continuous volcanic tuff, with no ashlar joints.
        mapping={8:4,11:EARTHEN_PLASTER,12:EARTHEN_PLASTER,15:4,17:EARTHEN_PLASTER}
    elif name=='shwedagon-pagoda':
        mapping={11:PLASTER_WALL,12:METAL_ROOF,17:STONE_WALL}
        palette={c:SCULPTURE_METAL for c in [GOLD,(237,193,77),(236,189,72),(222,188,91)]}
        palette[RED]=TIMBER_WALL
    elif name=='wat-arun':
        mapping={11:PLASTER_WALL,12:STONE_WALL,15:4,17:TILE_ROOF}
        palette={GOLD:SCULPTURE_METAL}
    elif name in ('potala-palace','paro-taktsang'):
        mapping={11:PLASTER_WALL,12:EARTHEN_PLASTER,17:TIMBER_WALL}
        palette={GOLD:SCULPTURE_METAL,(234,193,78):SCULPTURE_METAL,
                 IVORY:STONE_WALL,(179,159,111):EARTHEN_PLASTER}
    elif name=='meenakshi-amman-temple':
        mapping={11:PLASTER_WALL,12:FLAT_ROOF,15:4,17:PLASTER_WALL}
        palette={(166,147,116):STONE_WALL,(153,149,130):STONE_WALL,(162,159,140):STONE_WALL,(157,139,106):STONE_WALL,
                 (123,160,173):PLASTER_WALL,GOLD:SCULPTURE_METAL}
    elif name=='swaminarayan-akshardham':
        # The visible exterior is Rajasthan pink sandstone. White Carrara marble
        # belongs primarily to the interior and is not invented on exterior domes.
        mapping=masonry
        palette={GOLD:SCULPTURE_METAL}
    elif name=='taj-mahal':
        mapping={11:STONE_WALL,12:VEINED_MARBLE,15:4,17:VEINED_MARBLE}
        palette={c:VEINED_MARBLE for c in [MARBLE,(231,228,213),(220,217,202),(220,217,203)]}
        palette.update({GOLD:SCULPTURE_METAL,(143,118,64):SCULPTURE_METAL,
                        (166,87,65):STONE_WALL,(164,76,57):STONE_WALL,(150,78,60):STONE_WALL})
    elif name=='sagrada-familia':
        mapping={11:WEATHERED_LIMESTONE,12:TILE_ROOF,17:WEATHERED_LIMESTONE}
        palette={c:TILE_ROOF for c in [(198,177,117),(162,71,55),(83,148,145),(213,192,93),
            (185,101,58),(128,161,90),(201,173,87),(73,111,67)]}
        palette[MARBLE]=VEINED_MARBLE
    else:raise KeyError(name)
    m.triangles=[(p,c,palette.get(c,mapping.get(mat,mat)) if mat in (8,11,12,15,17) else mat)
                 for p,c,mat in m.triangles]
    return m

def build(name,lod):
    if lod not in (0,1,2):raise ValueError('LOD must be 0, 1 or 2')
    m=_BUILDERS[name](lod)
    if name=='potala-palace':m.transform([[1,0,0,0],[0,117.2/113.0,0,0],[0,0,1,0],[0,0,0,1]])
    if name=='paro-taktsang':m.transform([[.70,0,0,0],[0,1,0,0],[0,0,.70,0],[0,0,0,1]])
    if name=='wat-arun':m.transform([[.72,0,0,0],[0,1,0,0],[0,0,.72,0],[0,0,0,1]])
    if name=='shwedagon-pagoda':m.transform([[.86,0,0,0],[0,1,0,0],[0,0,.86,0],[0,0,0,1]])
    return _physical_materials(name,m)


ENTRIES=[{'name': 'borobudur',
  'city': 'Magelang',
  'lat': -7.607940007352901,
  'lon': 110.20381806175165,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://whc.unesco.org/en/list/592/', 'https://whc.unesco.org/document/186919'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Five stepped galleries,72 perforated stupas on three round terraces,cardinal stairs,and a '
                 'central stupa.',
  'source_exclusion_polygons': [[[-63, -63], [63, -63], [63, 63], [-63, 63]]],
  'road_exclusion_polygons': [[[-63, -63], [63, -63], [63, 63], [-63, 63]]],
  'activation_bounds_xz': [[-65, -65], [65, 65]],
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [62, 62],
                       'feather_m': 7,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 267.572}],
  'terrain_cuts': [],
  'max_source_building_span_m': 168,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/borobudur-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/borobudur-ground.json',
            'ground_datum_m': 267.572,
            'architectural_dimensions_m': [123, 35.4, 123],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 221.4,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 12.389999999999999,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'shwedagon-pagoda',
  'city': 'Yangon',
  'lat': 16.7983388552758,
  'lon': 96.14959587152097,
  'heading': -15,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://shwedagonpagoda.com/architecture.php',
                 'https://openjicareport.jica.go.jp/pdf/12122537_02.pdf'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Gilded bell profile,individually banded spire and hti,satellite stupas and tiered Burmese '
                 'pavilions.',
  'source_exclusion_polygons': [[[-85, -83], [85, -83], [85, 83], [-85, 83]]],
  'road_exclusion_polygons': [[[-85, -83], [85, -83], [85, 83], [-85, 83]]],
  'activation_bounds_xz': [[-87, -85], [87, 85]],
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [84.3, 81.7],
                       'feather_m': 5,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 65.732}],
  'terrain_cuts': [{'polygon_xz': [[-98, -95], [98, -95], [98, 95], [-98, 95]],
                    'plane': [0, 0, -0.3], 'feather_m': 1.5}],
  'max_source_building_span_m': 213.6,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/shwedagon-pagoda-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/shwedagon-pagoda-ground.json',
            'ground_datum_m': 65.732,
            'architectural_dimensions_m': [168.6, 99.365, 163.4],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 303.48,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 34.77775,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'wat-arun',
  'city': 'Bangkok',
  'lat': 13.74369813471238,
  'lon': 100.48890308581204,
  'heading': -18.3,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://api.tat.or.th/upload/live/multimedia/8969/E-Brochure_Bangkok_%28Th%29.pdf'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Central and four satellite prangs,steep cardinal stairs,and individually colored ceramic '
                 'ornament.',
  'source_exclusion_polygons': [[[-36.4, -36.4], [36.4, -36.4], [36.4, 36.4], [-36.4, 36.4]]],
  'road_exclusion_polygons': [[[-36.4, -36.4], [36.4, -36.4], [36.4, 36.4], [-36.4, 36.4]]],
  'activation_bounds_xz': [[-38.4, -38.4], [38.4, 38.4]],
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [36.36, 36.36],
                       'feather_m': 4,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 9.271}],
  'terrain_cuts': [{'polygon_xz': [[-50.5, -50.5], [50.5, -50.5], [50.5, 50.5], [-50.5, 50.5]],
                    'plane': [0, 0, -0.3], 'feather_m': 1.5}],
  'max_source_building_span_m': 117.72,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/wat-arun-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/wat-arun-ground.json',
            'ground_datum_m': 9.271,
            'architectural_dimensions_m': [72.72, 82, 72.72],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 130.89600000000002,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 28.7,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'potala-palace',
  'city': 'Lhasa',
  'lat': 29.65795,
  'lon': 91.11705,
  'heading': 0,
  'base': 3653.12,
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://whc.unesco.org/en/list/707/',
                 'https://m.tibet.cn/eng/travel/destination/201512/t20151202_5717373.html',
                 'https://doi.org/10.1186/s43238-026-00260-0',
                 'https://www.chinastory.cn/ywdbk/english/v1/detail/20190708/1012700000042741562587081859903262_1.html'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Asymmetric terraced White and Red Palaces,seven gilded roof pavilions,window surrounds,and '
                 'immediate rock outcrop.',
  'source_exclusion_polygons': [[[-142, -63], [-58, -63], [-58, 15], [-142, 15]],
                                [[-77, -49], [62, -49], [62, 69], [-77, 69]],
                                [[49, -44], [148, -44], [148, 59], [49, 59]],
                                [[132, 12], [223, 12], [223, 38], [132, 38]]],
  'road_exclusion_polygons': [[[-142, -63], [-58, -63], [-58, 15], [-142, 15]],
                              [[-77, -49], [62, -49], [62, 69], [-77, 69]],
                              [[49, -44], [148, -44], [148, 59], [49, 59]],
                              [[132, 12], [223, 12], [223, 38], [132, 38]]],
  'activation_bounds_xz': [[-144, -65], [225, 71]],
  'terrain_flatten': [],
  'terrain_cuts': [{'polygon_xz': [[-153, -68], [-53, -68], [-53, 22], [-153, 22]],
                    'plane': [0, 0, -0.25],
                    'feather_m': 18},
                   {'polygon_xz': [[-77, -49], [62, -49], [62, 69], [-77, 69]],
                    'plane': [0, 0, 15],
                    'feather_m': 4},
                   {'polygon_xz': [[49, -44], [148, -44], [148, 59], [49, 59]],
                    'plane': [0, 0, 0],
                    'feather_m': 4},
                   {'polygon_xz': [[132, 12], [223, 12], [223, 38], [132, 38]],
                    'plane': [0, 0, 18],
                    'feather_m': 3}],
  'max_source_building_span_m': 417,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/potala-palace-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/potala-palace-ground.json',
            'ground_datum_m': 3653.12,
            'architectural_dimensions_m': [372, 117.2, 140],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.',
            'grounding_note': 'Datum median of three level southeast-valley samples; cuts apply only beneath '
                              'authored masonry. Southwest foundation cleanup extends11m beyond its former '
                              'cut with18m feather: production samples reach3709.7m at(-125,-62), above the '
                              'lowest range roof. This bounded DSM intrusion correction preserves surrounding '
                              'Red Mountain. Facade/material evidence and approximation bounds are in '
                              'references/temples/potala-palace-facade.json.'},
  'capture': {'distance': 410,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 41.019999999999996,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'paro-taktsang',
  'city': 'Paro',
  'lat': 27.491926829553478,
  'lon': 89.36351880518266,
  'heading': 0,
  'base': 3060.623,
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://a.storyblok.com/f/171618/x/b4748f2dc4/visitor-travel-booklet-thimphu-and-paro.pdf'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Separate cliff ledges supporting white monastery blocks,layered red-and-gold Bhutanese '
                 'roofs,and the immediate cliff face.',
  'source_exclusion_polygons': [[[-23.799999999999997, -3.15],
                                 [-8.399999999999999, -3.15],
                                 [-8.399999999999999, 18.549999999999997],
                                 [-23.799999999999997, 18.549999999999997]],
                                [[-15.749999999999998, -7.699999999999999],
                                 [4.55, -7.699999999999999],
                                 [4.55, 14.7],
                                 [-15.749999999999998, 14.7]],
                                [[-4.199999999999999, 7.0],
                                 [12.6, 7.0],
                                 [12.6, 22.4],
                                 [-4.199999999999999, 22.4]],
                                [[8.399999999999999, -2.8],
                                 [22.4, -2.8],
                                 [22.4, 9.799999999999999],
                                 [8.399999999999999, 9.799999999999999]],
                                [[-25.2, 6.6499999999999995],
                                 [-14.0, 6.6499999999999995],
                                 [-14.0, 17.15],
                                 [-25.2, 17.15]],
                                [[4.199999999999999, -24.15],
                                 [23.799999999999997, -24.15],
                                 [23.799999999999997, -10.85],
                                 [4.199999999999999, -10.85]],
                                [[-7.0, -19.25],
                                 [4.199999999999999, -19.25],
                                 [4.199999999999999, -10.149999999999999],
                                 [-7.0, -10.149999999999999]]],
  'road_exclusion_polygons': [[[-23.799999999999997, -3.15],
                               [-8.399999999999999, -3.15],
                               [-8.399999999999999, 18.549999999999997],
                               [-23.799999999999997, 18.549999999999997]],
                              [[-15.749999999999998, -7.699999999999999],
                               [4.55, -7.699999999999999],
                               [4.55, 14.7],
                               [-15.749999999999998, 14.7]],
                              [[-4.199999999999999, 7.0],
                               [12.6, 7.0],
                               [12.6, 22.4],
                               [-4.199999999999999, 22.4]],
                              [[8.399999999999999, -2.8],
                               [22.4, -2.8],
                               [22.4, 9.799999999999999],
                               [8.399999999999999, 9.799999999999999]],
                              [[-25.2, 6.6499999999999995],
                               [-14.0, 6.6499999999999995],
                               [-14.0, 17.15],
                               [-25.2, 17.15]],
                              [[4.199999999999999, -24.15],
                               [23.799999999999997, -24.15],
                               [23.799999999999997, -10.85],
                               [4.199999999999999, -10.85]],
                              [[-7.0, -19.25],
                               [4.199999999999999, -19.25],
                               [4.199999999999999, -10.149999999999999],
                               [-7.0, -10.149999999999999]]],
  'activation_bounds_xz': [[-27.2, -26.15], [25.799999999999997, 24.4]],
  'terrain_flatten': [],
  'terrain_cuts': [{'polygon_xz': [[-23.799999999999997, -3.15],
                                   [-8.399999999999999, -3.15],
                                   [-8.399999999999999, 18.549999999999997],
                                   [-23.799999999999997, 18.549999999999997]],
                    'plane': [0, 0, -1],
                    'feather_m': 1.5},
                   {'polygon_xz': [[-15.749999999999998, -7.699999999999999],
                                   [4.55, -7.699999999999999],
                                   [4.55, 14.7],
                                   [-15.749999999999998, 14.7]],
                    'plane': [0, 0, 5],
                    'feather_m': 1.5},
                   {'polygon_xz': [[-4.199999999999999, 7.0],
                                   [12.6, 7.0],
                                   [12.6, 22.4],
                                   [-4.199999999999999, 22.4]],
                    'plane': [0, 0, 8],
                    'feather_m': 1.5},
                   {'polygon_xz': [[8.399999999999999, -2.8],
                                   [22.4, -2.8],
                                   [22.4, 9.799999999999999],
                                   [8.399999999999999, 9.799999999999999]],
                    'plane': [0, 0, 13],
                    'feather_m': 1.5},
                   {'polygon_xz': [[-25.2, 6.6499999999999995],
                                   [-14.0, 6.6499999999999995],
                                   [-14.0, 17.15],
                                   [-25.2, 17.15]],
                    'plane': [0, 0, 9],
                    'feather_m': 1.5},
                   {'polygon_xz': [[4.199999999999999, -24.15],
                                   [23.799999999999997, -24.15],
                                   [23.799999999999997, -10.85],
                                   [4.199999999999999, -10.85]],
                    'plane': [0, 0, -6],
                    'feather_m': 1.5},
                   {'polygon_xz': [[-7.0, -19.25],
                                   [4.199999999999999, -19.25],
                                   [4.199999999999999, -10.149999999999999],
                                   [-7.0, -10.149999999999999]],
                    'plane': [0, 0, -5],
                    'feather_m': 1.5}],
  'max_source_building_span_m': 100,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/paro-taktsang-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/paro-taktsang-ground.json',
            'ground_datum_m': 3060.623,
            'architectural_dimensions_m': [55, 42, 40],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.',
            'grounding_note': 'Local cliff-anchor raster datum; separate ledge cuts replace DEM smoothing '
                              'under masonry. No whole-hill flatten. In-engine cliff clearance still requires '
                              'visual confirmation.'},
  'capture': {'distance': 99.0,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 14.7,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'prambanan',
  'city': 'Yogyakarta',
  'lat': -7.75200191591475,
  'lon': 110.49140186909271,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://whc.unesco.org/en/list/642/'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Main Shiva,Vishnu and Brahma temples;three vehicle shrines;apit and boundary shrines;224 '
                 'predominantly ruined perwara sites.',
  'source_exclusion_polygons': [[[-93, -93], [93, -93], [93, 93], [-93, 93]]],
  'road_exclusion_polygons': [[[-93, -93], [93, -93], [93, 93], [-93, 93]]],
  'activation_bounds_xz': [[-95, -95], [95, 95]],
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [93, 93],
                       'feather_m': 6,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 149.971}],
  'terrain_cuts': [{'polygon_xz': [[-56, -56], [56, -56], [56, 56], [-56, 56]],
                    'plane': [0, 0, -0.3], 'feather_m': 1.5}],
  'max_source_building_span_m': 231,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/prambanan-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/prambanan-ground.json',
            'ground_datum_m': 149.971,
            'architectural_dimensions_m': [186, 47, 186],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 334.8,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 16.45,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'brihadisvara-temple',
  'city': 'Thanjavur',
  'lat': 10.782698063895197,
  'lon': 79.13166747649804,
  'heading': -20.3,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://whc.unesco.org/en/list/250/',
                 'https://www.tamilnadutourism.com/attractions/temples/brihadeeswarar-temple-tanjore'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Thirteen-tier vimana,low axial halls,separate Nandi pavilion,subsidiary shrines,enclosure '
                 'and two eastern gateways.',
  'source_exclusion_polygons': [[[-121, -62], [121, -62], [121, 62], [-121, 62]],
                                [[115, -18], [195, -18], [195, 18], [115, 18]]],
  'road_exclusion_polygons': [[[-121, -62], [121, -62], [121, 62], [-121, 62]],
                              [[115, -18], [195, -18], [195, 18], [115, 18]]],
  'activation_bounds_xz': [[-123, -64], [197, 64]],
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [120.5, 61],
                       'feather_m': 5,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 62.983},
                      {'center_xz': [157, 0],
                       'half_extent': [37, 18],
                       'feather_m': 4,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 62.983}],
  'terrain_cuts': [{'polygon_xz': [[-120.5, -61], [120.5, -61], [120.5, 61], [-120.5, 61]],
                    'plane': [0, 0, -0.3], 'feather_m': 1.5}],
  'max_source_building_span_m': 359,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/brihadisvara-temple-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/brihadisvara-temple-ground.json',
            'ground_datum_m': 62.983,
            'architectural_dimensions_m': [314, 66, 122],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 565.2,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 23.099999999999998,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'meenakshi-amman-temple',
  'city': 'Madurai',
  'lat': 9.919653696998244,
  'lon': 78.11930911857635,
  'heading': -13.8,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://madurai.nic.in/tourist-place/sri-meenakshi-sundareswarar-temple/',
                 'https://maduraimeenakshi.hrce.tn.gov.in/',
                 'https://commons.wikimedia.org/wiki/File:Plan_of_Meenakshi_Amman_Temple_Madurai_India.jpg'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Four outer and ten inner painted gopurams,two gold sanctum roofs,asymmetric roofed '
                 'galleries,and a roofless Golden Lotus Tank.',
  'source_exclusion_polygons': [[[-145, -150], [147, -150], [147, 108], [-145, 108]]],
  'road_exclusion_polygons': [[[-145, -150], [147, -150], [147, 108], [-145, 108]]],
  'activation_bounds_xz': [[-147, -152], [149, 110]],
  'terrain_flatten': [{'center_xz': [0, -21],
                       'half_extent': [135, 118],
                       'feather_m': 5,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 143.401}],
  'terrain_cuts': [],
  'max_source_building_span_m': 325,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/meenakshi-amman-temple-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/meenakshi-amman-temple-ground.json',
            'ground_datum_m': 143.401,
            'architectural_dimensions_m': [280, 51.9, 255],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.',
            'layout_note': '1911plan west-up conversion x=(pixelY-500)*.297,z=(pixelX-526)*.297 independently '
                           'matches modern four gate centers; current source vectors determine heading and '
                           'anchor.'},
  'capture': {'distance': 504.0,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 18.165,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'konark-sun-temple',
  'city': 'Konark',
  'lat': 19.887563217904233,
  'lon': 86.09454,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://culture.gov.in/sun-temple-konarak',
                 'https://www.works.odisha.gov.in/en/odisha-tourism/important-tourist-pointskonark',
                 'https://ignca.gov.in/asi_reports/orpuri093.pdf',
                 'https://youngintach.org/files/activity_ideas56.pdf'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Surviving jagamohana and24 spoked chariot wheels,ruined western deul foundations,and the '
                 'roofless detached nata mandira, Mayadevi remains, kitchen and sculpture plinths.',
  'source_exclusion_polygons': [[[-49, -36], [44, -36], [44, 36], [-49, 36]],
                                [[69, -17], [108, -17], [108, 17], [69, 17]]] + [[[-104, -41], [-61, -41], [-61, -19], [-104, -19]], [[61, -75], [97, -75], [97, -46], [61, -46]], [[-14, 37], [0, 37], [0, 55], [-14, 55]], [[6, 46], [15, 46], [15, 57], [6, 57]], [[3, -77], [26, -77], [26, -63], [3, -63]]],
  'road_exclusion_polygons': [[[-49, -36], [44, -36], [44, 36], [-49, 36]],
                              [[69, -17], [108, -17], [108, 17], [69, 17]]] + [[[-104, -41], [-61, -41], [-61, -19], [-104, -19]], [[61, -75], [97, -75], [97, -46], [61, -46]], [[-14, 37], [0, 37], [0, 55], [-14, 55]], [[6, 46], [15, 46], [15, 57], [6, 57]], [[3, -77], [26, -77], [26, -63], [3, -63]]],
  'activation_bounds_xz': [[-107, -80], [110, 60]],
  'terrain_flatten': [{'center_xz': [-5, 0],
                       'half_extent': [46, 36],
                       'feather_m': 4,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 14.82},
                      {'center_xz': [84, 0],
                       'half_extent': [16, 16],
                       'feather_m': 4,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 14.82},
                      {'center_xz': [-82.6, -30.8], 'half_extent': [21.5, 11],
                       'feather_m': 4, 'sample_clearance_m': 8, 'fixed_base_geo': 9.9468},
                      {'center_xz': [78.7, -61.2], 'half_extent': [17, 14],
                       'feather_m': 4, 'sample_clearance_m': 8, 'fixed_base_geo': 15.2816},
                      {'center_xz': [-7.1, 45.4], 'half_extent': [3.5, 4],
                       'feather_m': 3, 'sample_clearance_m': 8, 'fixed_base_geo': 14.5822},
                      {'center_xz': [10.5, 51.4], 'half_extent': [3.5, 4],
                       'feather_m': 3, 'sample_clearance_m': 8, 'fixed_base_geo': 15.7320},
                      {'center_xz': [15, -69.9], 'half_extent': [8.5, 4],
                       'feather_m': 3, 'sample_clearance_m': 8, 'fixed_base_geo': 18.0655}],
  'terrain_cuts': [],
  'max_source_building_span_m': 198,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/konark-sun-temple-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/konark-sun-temple-ground.json',
            'ground_datum_m': 14.82,
            'architectural_dimensions_m': [212, 39, 135],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 340.0,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 13.649999999999999,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'swaminarayan-akshardham',
  'city': 'Delhi',
  'lat': 28.612537438555602,
  'lon': 77.2773589571242,
  'heading': -30,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://akshardham.com/explore/Mandir/'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Pink-sandstone mandir,nine ribbed domes,20 small square spires,carved column rhythm,and '
                 'elephant-frieze peeth.',
  'source_exclusion_polygons': [[[-55, -49], [55, -49], [55, 49], [-55, 49]]],
  'road_exclusion_polygons': [[[-55, -49], [55, -49], [55, 49], [-55, 49]]],
  'activation_bounds_xz': [[-57, -51], [57, 51]],
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [54.8, 48.8],
                       'feather_m': 4,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 199.637}],
  'terrain_cuts': [],
  'max_source_building_span_m': 153.51,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/swaminarayan-akshardham-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/swaminarayan-akshardham-ground.json',
            'ground_datum_m': 199.637,
            'architectural_dimensions_m': [108.51, 43.068, 96.32],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 195.318,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 15.073799999999999,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'biete-giyorgis',
  'city': 'Lalibela',
  'lat': 12.031626552928785,
  'lon': 39.04114122419252,
  'heading': 10,
  'base': 2420.681,
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://whc.unesco.org/en/list/18/'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Cruciform monolithic church below natural ground,three nested roof crosses,carved openings '
                 'and a12.6m-deep excavated court.',
  'source_exclusion_polygons': [[[-14, -14], [14, -14], [14, 14], [-14, 14]]],
  'road_exclusion_polygons': [[[-32.43,-31.02],[30.55,-32.43],[32.43,29.375],[-30.55,31.96]]],
  'activation_bounds_xz': [[-34, -34], [34, 34]],
  'terrain_flatten': [],
  'terrain_cuts': [{'polygon_xz': [[-25.53, -24.42], [24.05, -25.53], [25.53, 23.125], [-24.05, 25.16]],
                    'plane': [0, 0, -12.6],
                    'feather_m': 8}],
  'max_source_building_span_m': 57.6,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/biete-giyorgis-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/biete-giyorgis-ground.json',
            'ground_datum_m': 2420.681,
            'architectural_dimensions_m': [12.6, 13.5, 12.6],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.',
            'grounding_note': 'Court walls retain the architectural pit footprint. A wider hidden lower-only '
                              'terrain clearance prevents coarse triangles crossing those walls. An authored '
                              'rock rim joins 48 sampled outer ground points at every LOD with a buried skirt; native paths beneath this authored rim are replaced by its walkable rock surface. The clearance is a rendering '
                              'allowance, not an enlarged archaeological excavation.',
            'rim_ground_samples_file': 'tools/heritage_catalog/references/temples/biete-giyorgis-rim-ground.json'},
  'capture': {'distance': 45,
              'azimuth': 210,
              'elevation': 38,
              'target_height': -3,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'taj-mahal',
  'city': 'Agra',
  'lat': 27.175036215274172,
  'lon': 78.0420804645743,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://www.tajmahal.gov.in/outlying-building.aspx',
                 'https://www.tajmahal.gov.in/the-taj-exteriors.aspx'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'Clipped-square marble mausoleum,bulbous dome,four minarets,chhatris,pietra-dura color '
                 'details,and both red-sandstone flanking halls.',
  'source_exclusion_polygons': [[[-153, -60], [155, -60], [155, 56], [-153, 56]], [[-8,-66.3],[8,-66.3],[8,-59.8],[-8,-59.8]]],
  'road_exclusion_polygons': [[[-153, -60], [155, -60], [155, 56], [-153, 56]], [[-8,-66.3],[8,-66.3],[8,-59.8],[-8,-59.8]]],
  'activation_bounds_xz': [[-155, -68], [157, 58]],
  'terrain_flatten': [{'center_xz': [1, -3],
                       'half_extent': [152, 53],
                       'feather_m': 3,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 164.4}],
  'terrain_cuts': [{'polygon_xz': [[-152, -55.5], [154, -55.5], [154, 55.5], [-152, 55.5]],
                    'plane': [0, 0, -0.3], 'feather_m': 1.5}],
  'max_source_building_span_m': 351,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/taj-mahal-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/taj-mahal-ground.json',
            'ground_datum_m': 164.4,
            'architectural_dimensions_m': [306, 73, 116],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.',
            'grounding_note': '164.4m datum follows lower unobstructed southern garden approach samples; lower '
                              'northern river samples excluded.10m terrace retaining walls meet lower '
                              'riverbank terrain.'},
  'capture': {'distance': 550.8000000000001,
              'azimuth': 210,
              'elevation': 30,
              'target_height': 25.549999999999997,
              'time_utc': '2026-03-20T10:00:00Z'}},
 {'name': 'sagrada-familia',
  'city': 'Barcelona',
  'lat': 41.40351943085036,
  'lon': 2.1744406483531376,
  'heading': -45,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': True,
  'suppress_trees': True,
  'references': ['https://sagradafamilia.org/en/-/la-grua-central-de-la-sagrada-familia-inicia-una-nova-etapa-despres-de-completar-l-exterior-de-la-torre-de-jesucrist?redirect=%2Fen%2Fnews',
                 'https://sagradafamilia.org/documents/20142/1205286/SF_Booklet_10_20240415_digital_AF.pdf/504e0081-e3b7-43ff-be2c-0efb05e2ebaf',
                 'https://sagradafamilia.org/en/-/culminen-les-torres-dels-evangelistes-mateu-i-joan?redirect=%2Fen%2Fhome'],
  'attribution': 'Geometry originally authored for Low Poly Earth. Placement and footprint evidence: © OpenStreetMap '
                 'contributors (ODbL), via Protomaps.',
  'description': 'September2026 state:14 existing towers,172.5m Jesus cross,138m Mary star,135m '
                 'Evangelists,contrasting Nativity and Passion facades,and nave vault roofs.',
  'source_exclusion_polygons': [[[-36, -54.5], [36, -54.5], [36, 55.2], [-36, 55.2]]],
  'road_exclusion_polygons': [[[-36, -54.5], [36, -54.5], [36, 55.2], [-36, 55.2]]],
  'activation_bounds_xz': [[-38, -56.5], [38, 57.2]],
  'terrain_flatten': [{'center_xz': [0, 0.35],
                       'half_extent': [36, 54.85],
                       'feather_m': 6,
                       'sample_clearance_m': 8,
                       'fixed_base_geo': 45.579}],
  'terrain_cuts': [{'polygon_xz': [[-36, -54.5], [36, -54.5], [36, 54.5], [-36, 54.5]],
                    'plane': [0, 0, -0.3], 'feather_m': 1.5}],
  'max_source_building_span_m': 155,
  'audit': {'survey_file': 'tools/heritage_catalog/references/temples/sagrada-familia-survey.json',
            'ground_samples_file': 'tools/heritage_catalog/references/temples/sagrada-familia-ground.json',
            'ground_datum_m': 45.579,
            'architectural_dimensions_m': [72, 172.5, 110],
            'height_authority': 'Published architectural dimension; vector tile heights are not architectural '
                                'authority.'},
  'capture': {'distance': 240,
              'azimuth': 145,
              'elevation': 30,
              'target_height': 80,
              'time_utc': '2026-03-20T10:00:00Z'}}]
