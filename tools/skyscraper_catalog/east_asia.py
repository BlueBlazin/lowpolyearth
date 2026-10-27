"""Original metric East Asian skyscrapers, fitted to retained public map rings."""
import json
import math
from pathlib import Path
import numpy as np
from heritage_catalog.geometry import Mesh, face, plan_solid, plan_triangles, lathe
from heritage_catalog.materials import (
    WINDOW, SCULPTURE_METAL, STONE_WALL, PANEL_WALL, FLAT_ROOF, VEINED_MARBLE,
)
from skyscraper_catalog.east_asia_geometry import (
    ccw, resample, rounded_square, chamfer, rotated, roof, loft, ribs, band,
    diagonals, grid_prism,
)

REF=Path(__file__).parent/'references/east_asia'
FITS={p.name.removesuffix('-placement.json'):json.loads(p.read_text())
      for p in REF.glob('*-placement.json')}
GLASS=(91,124,140)
SILVER=(168,177,177)


def _p(name,index=None):
    fit=FITS[name]
    key=str(fit['main_source_index'] if index is None else index)
    return ccw(fit.get('fitted_plans_xz_m',{}).get(key,fit['parts'][key]['model_rings_xz_m'][0]))


def _foundation(m,name,plan=None,top=.12):
    p=_p(name) if plan is None else ccw(plan)
    fit=FITS[name]
    bottom=fit.get('foundation_bottom_m',-5)
    plan_solid(m,p,bottom,top,(151,148,139),STONE_WALL)
    roof(m,p,top+.012,(161,158,150),STONE_WALL)


def _podium(m,name,index,height,lod):
    p=_p(name,index);_foundation(m,name,p)
    grid_prism(m,p,.12,height,lod,(91,112,115),(178,175,165),4.5,2.4,1.15)


def _smooth(points,count):
    p=ccw(points);out=[]
    for i in range(len(p)):
        a,b,c,d=p[(i-1)%len(p)],p[i],p[(i+1)%len(p)],p[(i+2)%len(p)]
        for t in np.linspace(0,1,8,endpoint=False):
            out.append(.5*((2*b)+(-a+c)*t+(2*a-5*b+4*c-d)*t*t+(-a+3*b-3*c+d)*t*t*t))
    return resample(out,count)


def _shanghai(lod):
    n='shanghai-tower';m=Mesh();_foundation(m,n)
    # Published envelope: a curved triangle, exponential taper and 120° twist.
    # The mapped entrance notch is kept in the fitted ground plan.
    p=_smooth(_p(n),(144,96,48)[lod])
    def section(y):
        scale=math.exp(math.log(.55686)*max(0,y-45)/560)
        return rotated(p*scale,120*y/632)
    zones=[.12,20,90,155,225,295,365,445,520,580,605]
    loft(m,section,zones,lod,(110,145,154),(166,182,184),4.5,.18,.10)
    for y in [90,155,225,295,365,445,520,580]:
        band(m,section,y,2.4,(73,101,105))
        # The recessed ring is the double-skin sky-garden floor, not a facade brace.
        q=section(y)*.87;roof(m,q,y+.2,(108,120,112))
    # The wind-screen crown has an inclined open rim, rather than a flat cone.
    q=section(605);upper=section(632)
    for j in range(len(q)):
        k=(j+1)%len(q);a=2*math.pi*j/len(q);b=2*math.pi*k/len(q)
        ya=619+13*math.sin(a);yb=619+13*math.sin(b)
        normal=(q[k,1]-q[j,1],0,q[j,0]-q[k,0])
        face(m,[(q[j,0],605,q[j,1]),(q[k,0],605,q[k,1]),(upper[k,0],yb,upper[k,1]),(upper[j,0],ya,upper[j,1])],(129,157,162),WINDOW,normal)
        if j%(2 if lod<2 else 3)==0:m.beam((q[j,0],605,q[j,1]),(upper[j,0],ya,upper[j,1]),.14,SILVER,SCULPTURE_METAL,4)
        m.beam((upper[j,0],ya,upper[j,1]),(upper[k,0],yb,upper[k,1]),.22,SILVER,SCULPTURE_METAL,4)
    roof(m,section(605)*.71,604.6,(71,84,87))
    return m


def _swfc(lod):
    n='shanghai-world-financial-center';m=Mesh();_podium(m,n,402,15,lod)
    # A 58m square is shaved by two curved planes toward a diagonal crest.
    def ring_uv(y):
        d=58/math.sqrt(2);v=d*(1-.90*(y/492)**1.5)
        return np.array([[-d+v,-v],[d-v,-v],[d,0],[d-v,v],[-d+v,v],[-d,0]])
    def section(y):return rotated(resample(ring_uv(max(y,0)),(88,56,28)[lod]),45)
    _foundation(m,n,rotated(chamfer(58,58,.01),0))
    loft(m,section,[.12,40,180,320,420,438],lod,(96,131,151),(163,177,181),4.35,.36,.11)
    for y in [85,175,263,351,426]:band(m,section,y,2.0,(66,87,98))
    # The aperture is genuinely empty. Its sloping jambs carry the upper skywalk.
    # Published 97th-floor aperture / 474m skywalk; jamb widths are photo estimates.
    def jamb(sign,y):
        d=58/math.sqrt(2);v=d*(1-.90*(y/492)**1.5)
        outer=d-v;inner=np.interp(y,[438,474,478],[19,26,26])
        q=np.array([[inner,-v],[outer,-v],[d,0],[outer,v],[inner,v]])
        if sign<0:q[:,0]*=-1
        return rotated(ccw(q),45)
    for sign in [-1,1]:loft(m,lambda y,s=sign:resample(jamb(s,y),(24,16,10)[lod]),[438,474,478],lod,GLASS,SILVER,4.2,.25,.10)
    loft(m,section,[478,487,492],lod,(99,135,155),(186,192,191),4.2,.35,.12)
    for y in [438,478]:
        roof(m,section(y),y,(80,105,115))
    for v in [-1,1]:
        a=rotated([[-26,4.7*v],[26,4.7*v]],45)
        m.beam((a[0,0],477.5,a[0,1]),(a[1,0],477.5,a[1,1]),.33,(180,190,188),SCULPTURE_METAL,4)
    return m


def _jin_mao(lod):
    n='jin-mao-tower';m=Mesh();_podium(m,n,524,10,lod)
    # Eight perimeter lobes and successive 16,14,12,10,8,7,6,5,4,3,2,1-floor groups.
    # The axial recesses are retained from the mapped plan rather than an octagon proxy.
    p=_p(n,503);floors=[16,14,12,10,8,7,6,5,4,3,2,1]
    y=10.;hfloor=365/88
    for k,nfloor in enumerate(floors):
        top=y+nfloor*hfloor;scale=1-.055*k
        q=p*scale;fn=lambda yy,q=q:q
        loft(m,lambda yy,q=q:resample(q,(100,64,32)[lod]),[y,top],lod,(105,127,134),(188,192,182),hfloor,.8,.12)
        band(m,fn,top-1.2,1.15,(126,135,132))
        # Projecting pagoda cornices and prominent eight stainless-steel ribs.
        roof(m,q*1.045,top+.03,(168,176,170),SCULPTURE_METAL)
        for a in np.arange(8)*math.pi/4:
            r=32*scale;x,z=r*math.cos(a),r*math.sin(a)
            m.beam((x,y,z),(x,top+2,z),.62,(197,199,184),SCULPTURE_METAL,6 if lod==0 else 4)
        y=top
    # The lattice lantern and stepped metal pinnacle are distinct from occupied floors.
    for bottom,top,r0,r1 in [(375,386,15,12),(386,398,12,7.5),(398,408,7.5,3.2)]:
        for a in np.arange(8)*math.pi/4:
            m.beam((r0*math.cos(a),bottom,r0*math.sin(a)),(r1*math.cos(a),top,r1*math.sin(a)),.40,(185,191,178),SCULPTURE_METAL,4)
        lathe(m,(0,0),[(bottom,r0),(bottom+.7,r0),(bottom+1.1,r0-.6)],(143,155,151),SCULPTURE_METAL,(24,16,12)[lod])
    lathe(m,(0,0),[(398,4),(408,3),(412,1.6),(419.5,.40),(420.5,.05)],(190,194,180),SCULPTURE_METAL,(16,12,8)[lod])
    return m


def _ping_an(lod):
    n='ping-an-finance-centre';m=Mesh();_podium(m,n,236,27,lod)
    # Successively recessed retail terraces face the southern amphitheatre.
    p=_p(n,236)
    for i in range(1,4):
        q=(p-p.mean(0))*(1-.13*i)+p.mean(0)+[0,4*i]
        grid_prism(m,q,27+(i-1)*5,27+i*5,lod,(107,126,125),(174,170,156),4.5,2.4,1.1)
    def section(y):
        width=np.interp(y,[0,100,460,510,555,584,599.1],[68,66,57,51,38,17,1.0])
        return resample(chamfer(width,cut=width*.19),(80,48,24)[lod])
    _foundation(m,n,_p(n));loft(m,section,[.12,100,230,370,460,510,555,584,599.1],lod,(125,144,149),(182,187,180),4.45,.42,.18)
    # Eight uninterrupted stainless-steel mega-column faces taper into the crown.
    for i in range(8):
        a=2*math.pi*i/8+math.pi/8
        for y0,y1 in zip([0,100,230,370,460,510,555,584],[100,230,370,460,510,555,584,599.1]):
            p0=resample(section(y0),8)[i];p1=resample(section(y1),8)[i]
            m.beam((p0[0],y0,p0[1]),(p1[0],y1,p1[1]),1.35 if y1<510 else .95,(181,185,173),SCULPTURE_METAL,6 if lod==0 else 4)
    for y in [108,210,311,410,502,548]:band(m,section,y,3.8,(89,107,112))
    return m


def _ctf(lod):
    n='guangzhou-ctf-finance-centre';m=Mesh();_podium(m,n,431,15,lod)
    _podium(m,n,549,38,lod)
    stages=[(0,273,64,61,0,0),(273,355,60,55,-2,0),(355,440,54,48,-5,-2),(440,498,46,40,-6,-5)]
    for bottom,top,w,d,x,z in stages:
        p=rounded_square(w,d,4,5)+[x,z];fn=lambda y,p=p:resample(p,(96,64,32)[lod])
        loft(m,fn,[max(.12,bottom),top],lod,(119,151,158),(221,219,203),4.2,.65,.38,PANEL_WALL)
        # White glazed terracotta is an opaque ceramic surface, not metal.
        ribs(m,fn,[max(.12,bottom),top],range(0,len(fn(0)),(2,2,2)[lod]),.25,(221,219,202),PANEL_WALL,lod)
        band(m,fn,top-3,3,(83,103,109))
    # KPF's completed exterior has stepped horizontal crown parapets.
    for x,width,top in [(-17.5,23,530),(5.5,23,518)]:
        p=chamfer(width,40,2)+[x,-5]
        loft(m,lambda y,p=p:resample(p,(40,24,16)[lod]),[498,top],lod,
             (135,157,161),(195,195,181),frame_material=PANEL_WALL,glass_material=104)
    return m


def _gz_ifc(lod):
    n='guangzhou-international-finance-center';m=Mesh();_foundation(m,n)
    p=_smooth(_p(n),(96,64,40)[lod]);center=np.mean(p,axis=0);p-=center
    def section(y):
        scale=np.interp(y,[0,120,240,340,438.6],[1,1.08,1.01,.87,.62])
        return p*scale+center
    loft(m,section,[.12,90,180,270,360,428,438.6],lod,(65,102,117),(106,127,131),4.15,.30,.08)
    # The externally legible 12-node diagrid follows the rounded triangular tube.
    diagonals(m,section,list(np.arange(0,421,30))+[438.6],12,.68,(167,181,180),lod,curved=True)
    for y in [120,240,360,427]:band(m,section,y,1.25,(86,115,120))
    roof(m,section(438.6)*.86,438.3,(69,88,91))
    return m


def _citic(lod):
    n='citic-tower';m=Mesh();_foundation(m,n)
    def section(y):
        # KPF publishes 78m at the foot, 54m at the waist, and 69m at the crown.
        t=y/527.7;w=54+(24*((.53-t)/.53)**2 if t<.53 else 15*((t-.53)/.47)**2)
        return resample(rounded_square(w,radius=5.2,steps=8),(112,72,40)[lod])
    levels=[.12,18,60,130,210,280,350,420,490,515]
    loft(m,section,levels,lod,(112,146,158),(178,193,196),4.25,.30,.13)
    for y in [98,205,302,396,490]:band(m,section,y,2.7,(87,112,121))
    # The four lifted crown corners recall the flared rim of a zun vessel.
    p=section(515);q=section(527.7)
    for i in range(len(p)):
        j=(i+1)%len(p);h=lambda v:519.7+8*(min(abs(v[0]),abs(v[1]))/30)**2
        hi=min(527.7,h(q[i]));hj=min(527.7,h(q[j]))
        face(m,[(p[i,0],515,p[i,1]),(p[j,0],515,p[j,1]),(q[j,0],hj,q[j,1]),(q[i,0],hi,q[i,1])],(144,167,175),WINDOW,(p[j,1]-p[i,1],0,p[i,0]-p[j,0]))
        if i%(2 if lod<2 else 3)==0:m.beam((p[i,0],515,p[i,1]),(q[i,0],hi,q[i,1]),.19,SILVER,SCULPTURE_METAL,4)
    # Curved splayed entrance canopies emerge from the four side centres.
    for angle in [0,90,180,270]:
        for x in np.linspace(-17,17,(15,9,5)[lod]):
            points=rotated([(x,-39),(x,-45),(x,-52)],angle)
            ys=[18,8,5]
            for a,b,y0,y1 in zip(points,points[1:],ys,ys[1:]):m.beam((a[0],y0,a[1]),(b[0],y1,b[1]),.22,SILVER,SCULPTURE_METAL,4)
    return m


def _cctv(lod):
    n='cctv-headquarters';m=Mesh();pbase=_p(n,742)
    _foundation(m,n,pbase);grid_prism(m,pbase,.12,38,lod,(96,123,124),(107,126,124),4.1,2.3,.42)
    shift=math.tan(math.radians(6))
    def braced_wall(v,w,heights_fn,rows,lean=0):
        edge=w-v;normal=np.array([edge[1],0.,-edge[0]])
        normal[1]=-lean*shift*(normal[0]-normal[2])
        normal/=np.linalg.norm(normal)
        bays=max(1,round(np.linalg.norm(edge)/20))
        def point(t,u):
            p=v+edge*t;lo,hi=heights_fn(p);y=lo+(hi-lo)*u
            return np.array([p[0]+lean*y*shift,y,p[1]-lean*y*shift])+normal*.42
        # Members stay on one actual wall face; perimeter chords through a
        # corner would disappear behind the glazing and leave detached ends.
        for j in range(bays):
            for k in range(rows):
                a,b,c,d=point(j/bays,k/rows),point((j+1)/bays,k/rows),point((j+1)/bays,(k+1)/rows),point(j/bays,(k+1)/rows)
                for start,end in [(a,c),(b,d)]:
                    if lod:
                        side=np.cross(normal,end-start);side*=.25/np.linalg.norm(side)
                        face(m,[start-side,start+side,end+side,end-side],(145,160,158),SCULPTURE_METAL,normal)
                    else:m.beam(start,end,.25,(145,160,158),SCULPTURE_METAL,4)
    top_rings=[]
    for index,sign,height in [(744,1,234),(745,-1,210)]:
        p=_p(n,index);_foundation(m,n,p)
        def section(y,p=p,s=sign):return resample(p+[s*y*shift,-s*y*shift],(72,48,24)[lod])
        loft(m,section,[.12,38,80,130,170,height],lod,(100,132,132),(128,148,146),4.1,.30,.085)
        for v,w in zip(p,np.roll(p,-1,axis=0)):
            braced_wall(v,w,lambda q,h=height:(.12,h),round(height/18),sign)
        top_rings.append((p+[sign*height*shift,-sign*height*shift],height))
    # Opposite L-links form a three-dimensional continuous tube. The upper link
    # is assembled as two swept arms with a shared corner, leaving the void open.
    a,ha=top_rings[0];b,hb=top_rings[1]
    amin,amax=a.min(0),a.max(0);bmin,bmax=b.min(0),b.max(0)
    p=ccw([(amin[0],bmin[1]),(bmax[0],bmin[1]),(bmax[0],bmax[1]),
           (amax[0],bmax[1]),(amax[0],amax[1]),(amin[0],amax[1])])
    ca=a.mean(0);cb=b.mean(0);axis=cb-ca
    def heights(v):
        t=np.clip(np.dot(v-ca,axis)/np.dot(axis,axis),0,1)
        return 180-18*t,234-24*t
    q=resample(p,(120,72,36)[lod])
    for v,w in zip(q,np.roll(q,-1,axis=0)):
        bottom,top=heights(v);bottom2,top2=heights(w)
        face(m,[(v[0],bottom,v[1]),(w[0],bottom2,w[1]),(w[0],top2,w[1]),(v[0],top,v[1])],(112,139,138),103,(w[1]-v[1],0,v[0]-w[0]))
    for tri in plan_triangles(p):
        for side,normal in [(0,(0,-1,0)),(1,(0,1,0))]:
            face(m,[(v[0],heights(v)[side],v[1])for v in tri],(107,132,132),103,normal)
    for v,w in zip(p,np.roll(p,-1,axis=0)):
        braced_wall(v,w,heights,3)
    return m


def _ifc2(lod):
    n='two-international-finance-centre';m=Mesh();_foundation(m,n)
    p=_p(n)
    def section(y):
        scale=np.interp(y,[0,85,180,265,330,378,388],[1,1,.96,.88,.79,.68,.65])
        return resample(p*scale,(96,64,32)[lod])
    levels=[.12,30,85,180,265,330,378,388]
    loft(m,section,levels,lod,(119,143,151),(162,177,181),4.3,.5,.13,glass_material=104)
    # Paired pearl-coloured ribs and broad gently bowed faces ascend into the crown.
    for y in [85,180,265,330,378]:band(m,section,y,1.3,(122,141,148))
    # The construction report shows a comb of slender inward-curving crown fins.
    # Their gaps expose recessed plant, without an invented saw-tooth roof.
    q=resample(section(388),24)
    for i,a in enumerate(q):
        b=q[(i+1)%len(q)];mid=(a+b)/2
        lo=a*.45+mid*.55;hi=b*.45+mid*.55
        top=410+2*(.5+.5*math.cos(2*math.pi*i/6))
        for y0,y1,s0,s1 in [(388,401,1,1),(401,top,1,.86)]:
            normal=(b[1]-a[1],0,a[0]-b[0])
            face(m,[(lo[0]*s0,y0,lo[1]*s0),(hi[0]*s0,y0,hi[1]*s0),
                    (hi[0]*s1,y1,hi[1]*s1),(lo[0]*s1,y1,lo[1]*s1)],(169,183,185),SCULPTURE_METAL,normal)
            if lod<2:m.beam((lo[0]*s0,y0,lo[1]*s0),(lo[0]*s1,y1,lo[1]*s1),.17,SILVER,SCULPTURE_METAL,4)
    roof(m,q*.70,389,(98,120,127))
    return m


def _icc(lod):
    n='international-commerce-centre';m=Mesh();_foundation(m,n)
    p=_p(n)
    def section(y):
        # Re-entrant corner channels remain full-height, with subtle upper taper.
        s=np.interp(y,[0,110,380,470,484],[1,1,.96,.91,.91])
        return resample(p*s,(100,64,40)[lod])
    levels=[.12,35,110,220,340,420,470,484]
    loft(m,section,levels,lod,(127,154,163),(174,188,191),4.05,.35,.12,glass_material=104)
    for y in [110,210,312,415,470]:band(m,section,y,2.0,(119,144,153))
    # Three sheltering skirts and the extended north atrium follow mapped parts.
    for index in [66,727,728,729,730,735,736]:
        q=_p(n,index);h=FITS[n]['parts'][str(index)]['height']
        _foundation(m,n,q)
        center=q.mean(0);distance=np.linalg.norm(center)
        for j,(a,b)in enumerate(zip(q,np.roll(q,-1,axis=0))):
            ya=12+max(0,48-np.linalg.norm(a))*.9
            yb=12+max(0,48-np.linalg.norm(b))*.9
            face(m,[(a[0],.12,a[1]),(b[0],.12,b[1]),(b[0],yb,b[1]),(a[0],ya,a[1])],(143,163,167),104,(b[1]-a[1],0,a[0]-b[0]))
            if lod<2:m.beam((a[0],.2,a[1]),(a[0],ya,a[1]),.18,SILVER,SCULPTURE_METAL,4)
        for tri in plan_triangles(q):face(m,[(x,12+max(0,48-math.hypot(x,z))*.9,z)for x,z in tri],(142,165,171),WINDOW)
    return m


def _boc(lod):
    n='bank-of-china-tower';m=Mesh();_foundation(m,n)
    # Four triangular shafts occupy quadrants of Pei's published52m square.
    # Map triangles determine which compass-facing shaft terminates first.
    for index,low,high in [(936,110,160),(682,160,210),(938,210,260),(937,260,315)]:
        p=_p(n,index);center=np.array([0.,0.])
        # Each roof plane rises toward the shared core and its far corner.
        corner=np.argmax(np.linalg.norm(p,axis=1));opposite=np.argmin(np.linalg.norm(p,axis=1))
        heights=np.full(len(p),low,dtype=float);heights[opposite]=high
        if index==937:heights[corner]=315
        for j,a in enumerate(p):
            k=(j+1)%len(p);b=p[k];normal=(b[1]-a[1],0,a[0]-b[0])
            face(m,[(a[0],.12,a[1]),(b[0],.12,b[1]),(b[0],heights[k],b[1]),(a[0],heights[j],a[1])],(127,146,156),104,normal)
            # Broad triangulated steel members are the actual visible tube structure.
            for y in np.arange(.12,min(heights[j],heights[k]),50):
                top=min(y+50,heights[j],heights[k]);mid=(a+b)/2
                if top-y<1:continue
                m.beam((a[0],y,a[1]),(b[0],top,b[1]),.52,(184,192,187),SCULPTURE_METAL,4 if lod else 6)
                m.beam((b[0],y,b[1]),(a[0],top,a[1]),.52,(184,192,187),SCULPTURE_METAL,4 if lod else 6)
            m.beam((a[0],.12,a[1]),(a[0],heights[j],a[1]),.58,(188,196,192),SCULPTURE_METAL,4 if lod else 6)
        face(m,[(v[0],y,v[1])for v,y in zip(p,heights)],(150,170,178),WINDOW)
        for j,a in enumerate(p):
            k=(j+1)%len(p);b=p[k]
            m.beam((a[0],heights[j],a[1]),(b[0],heights[k],b[1]),.55,(193,200,194),SCULPTURE_METAL,4)
        if lod<2:
            # Sparse full-height secondary mullions use the existing engine bands.
            for a,b in zip(p,np.roll(p,-1,axis=0)):
                if np.linalg.norm(a-b)<10:continue
                for t in np.linspace(0,1,7)[1:-1]:
                    v=a+(b-a)*t;m.beam((v[0],1,v[1]),(v[0],low,v[1]),.11,(155,169,173),SCULPTURE_METAL,4)
    for x,z,tip in [(-12,-20,367.4),(-4,-22,360.8)]:
        lathe(m,(x,z),[(305,.78),(325,.65),(346,.45),(tip,.08)],(183,194,191),SCULPTURE_METAL,(12,8,6)[lod])
    return m


def _hsbc(lod):
    n='hsbc-main-building';m=Mesh();_foundation(m,n)
    metal=(163,172,170);dark=(112,130,136)
    # Eight masts, each a cluster of four 1.4m-diameter tubular steel columns.
    # 3.9m links and 33.6m transverse truss span are published structural sizes.
    mast_z=[-23,-5,13,31];tops=[110,176,176,135];center_x=4.2
    for x in [center_x-16.8,center_x+16.8]:
        for z,top in zip(mast_z,tops):
            for dx,dz in [(-1.05,-1.05),(1.05,-1.05),(-1.05,1.05),(1.05,1.05)]:
                lathe(m,(x+dx,z+dz),[(FITS[n].get('foundation_bottom_m',-8),.7),(5,.7),(top,.4)],metal,SCULPTURE_METAL,(10,8,6)[lod])
            for y in np.arange(4,top,3.9*(1,2,4)[lod]):
                m.box((x,y,z),(3.5,.45,3.5),(154,165,164),SCULPTURE_METAL)
    # The three depth bays step independently; the ground plaza stays open.
    for z,depth,top in [(-14,18,104),(4,18,168),(22,18,129)]:
        p=chamfer(54,depth,.15)+[center_x,z]
        grid_prism(m,p,13,top,lod,(122,145,151),metal,3.9,4.2,.8)
        roof(m,p,top+.08,(111,128,129))
    # Double-height suspension trusses and north/south X-braces carry the floors.
    for z,top in zip(mast_z,tops):
        for y in [34,66,98,130,168]:
            if y+7>top:continue
            x0=center_x-27;x1=center_x+27
            m.beam((x0,y,z),(x1,y,z),.55,metal,SCULPTURE_METAL,4)
            m.beam((x0,y+7,z),(x1,y+7,z),.55,metal,SCULPTURE_METAL,4)
            for a,b in [(x0,center_x),(center_x,x1)]:
                m.beam((a,y,z),(b,y+7,z),.68,metal,SCULPTURE_METAL,4)
                m.beam((a,y+7,z),(b,y,z),.68,metal,SCULPTURE_METAL,4)
    for x in [center_x-28,center_x+28]:
        for z0,z1,top in zip(mast_z,mast_z[1:],[104,168,129]):
            for y in [34,66,98,130,160]:
                if y+7>top:continue
                m.beam((x,y,z0),(x,y+7,z1),.52,metal,SCULPTURE_METAL,4)
                m.beam((x,y+7,z0),(x,y,z1),.52,metal,SCULPTURE_METAL,4)
    # East service modules, roof maintenance booms, and the paired plaza escalators.
    for z,top in [(-14,105),(4,165),(22,130)]:
        for y in np.arange(16,top,3.9*(1,2,4)[lod]):m.box((40.2,y,z),(4.3,2.7,12.5),(172,177,167),PANEL_WALL)
    for x in [center_x-16.8,center_x+16.8]:
        for z in [-5,13]:
            m.box((x,177,z),(3,2.2,3),(170,178,174),SCULPTURE_METAL)
            m.beam((x,179,z),(x+9,179.5,z),.45,(183,188,179),SCULPTURE_METAL,4)
    for side in [-1,1]:
        a=(center_x+side*9,.3,-31);b=(center_x+side*4,13,-2)
        m.beam(a,b,1.45,(139,151,153),SCULPTURE_METAL,4)
        if lod<2:
            for t in np.linspace(0,1,35):
                p=np.array(a)*(1-t)+np.array(b)*t;m.box(p,(2.0,.16,.75),(105,122,127),SCULPTURE_METAL)
    return m


def _lotte(lod):
    n='lotte-world-tower';m=Mesh();_foundation(m,n)
    p=_smooth(_p(n),(112,72,40)[lod]);p-=p.mean(0)
    def section(y):
        # Measured mapped lower/upper rings constrain the convex taper.
        scale=np.interp(y,[0,120,240,320,360,450,500,520],[1,.95,.75,.55,.43,.32,.27,.255])
        return p*scale
    levels=[.12,24,120,240,320,360,450,500,520]
    loft(m,section,levels,lod,(135,164,177),(177,190,194),4.35,.32,.13,glass_material=104)
    for y in [120,235,355,450,499]:band(m,section,y,1.2,(119,145,155))
    # Two separated crown ears continue the main shell around a genuine open cleft.
    count=(40,28,18)[lod]
    for sign in [-1,1]:
        angles=np.linspace(-math.pi*.40,math.pi*.40,count)
        for a,b in zip(angles,angles[1:]):
            point=lambda t,y:(sign*(9.8-.032*(y-520))*math.cos(t),y,10.7*math.sin(t))
            ya=535+19.5*(math.cos(a)**2);yb=535+19.5*(math.cos(b)**2)
            face(m,[point(a,520),point(b,520),point(b,yb),point(a,ya)],(150,176,185),104,(sign,0,0))
            if lod<2:m.beam(point(a,520),point(a,ya),.14,SILVER,SCULPTURE_METAL,4)
        a=point(0,541);b=(-a[0],541,a[2]);m.beam(a,b,.5,(144,158,161),SCULPTURE_METAL,4)
    for index,height in [(680,10.2),(720,12.59)]:_podium(m,n,index,height,lod)
    return m


def _sky85(lod):
    n='85-sky-tower';m=Mesh()
    # Source parts are split at a vector tile seam; retained union rings remove it.
    fit=FITS[n]
    def combined(key):return np.asarray(fit['unions_xz_m'][key])
    podium=combined('podium');_foundation(m,n,podium)
    grid_prism(m,podium,.12,50,lod,(115,139,137),(151,166,158),4.0,4.0,.8)
    for key in ['north_wing','south_wing']:
        p=combined(key);grid_prism(m,p,50,190,lod,(89,126,130),(142,163,158),4.0,4.0,.75)
        for y in [82,114,146,178]:band(m,lambda yy,p=p:p,y,1.3,(130,153,148))
    # The lower crossbar ends at70m; central shaft starts145m, leaving a75m void.
    p=combined('crossbar');grid_prism(m,p,50,70,lod,(104,137,140),(146,165,159),4.0,4.0,.8)
    p=combined('shaft')
    def section(y):
        scale=np.interp(y,[145,245,295,320,330],[1,1,.94,.79,.68])
        return (p-p.mean(0))*scale+p.mean(0)
    loft(m,section,[145,190,245,295,320,330],lod,(92,133,138),(155,176,169),4.0,.6,.18,glass_material=107)
    for y in [190,245,295,320]:band(m,section,y,2,(138,162,154))
    # Pagoda-like fourfold crown and the independently measured378m mast tip.
    p=combined('crown');c=p.mean(0)
    for bottom,top,s0,s1 in [(330,337,1,.84),(337,343,.84,.50),(343,347.5,.50,.16)]:
        loft(m,lambda y,p=p,c=c,bot=bottom,top=top,s0=s0,s1=s1:(p-c)*np.interp(y,[bot,top],[s0,s1])+c,[bottom,top],lod,(127,158,158),(177,194,177),glass_material=104)
        roof(m,(p-c)*s0+c,bottom,(139,166,158),SCULPTURE_METAL)
    lathe(m,(0,0),[(340,1.5),(347.5,1.2),(363,.68),(378,.08)],(180,191,177),SCULPTURE_METAL,(16,10,6)[lod])
    return m


BUILDERS={
    'shanghai-tower':_shanghai,'shanghai-world-financial-center':_swfc,
    'jin-mao-tower':_jin_mao,'ping-an-finance-centre':_ping_an,
    'guangzhou-ctf-finance-centre':_ctf,
    'guangzhou-international-finance-center':_gz_ifc,'citic-tower':_citic,
    'cctv-headquarters':_cctv,'two-international-finance-centre':_ifc2,
    'international-commerce-centre':_icc,'bank-of-china-tower':_boc,
    'hsbc-main-building':_hsbc,'lotte-world-tower':_lotte,'85-sky-tower':_sky85,
}
ARCHITECTURE=json.loads((REF/'architecture.json').read_text())


def _owned(name):
    parts={
        'shanghai-world-financial-center':[319,402],
        'jin-mao-tower':[524],
        'ping-an-finance-centre':[254,236],
        'guangzhou-ctf-finance-centre':[431,549,550],
        'cctv-headquarters':[742,744,745],
        'international-commerce-centre':[31,66,727,728,729,730,735,736],
        'lotte-world-tower':[713,680,720],
    }.get(name)
    if name=='85-sky-tower':return [np.asarray(FITS[name]['unions_xz_m']['podium'])]
    return [_p(name,i)for i in parts]if parts else [_p(name)]


ENTRIES=[]
for _name in BUILDERS:
    _f=FITS[_name];_a=ARCHITECTURE[_name]
    _polys=[t.tolist()for p in _owned(_name)for t in plan_triangles(p)]
    # Suspended source parts need replacement without grading the courtyard
    # beneath them. Ground ownership follows only the actual foot-bearing parts.
    _source_polys=_polys+([t.tolist()for t in plan_triangles(_p(_name,743))]
                         if _name=='cctv-headquarters'else [])
    ENTRIES.append({
        'name':_name,'city':_a['city'],'lat':_f['lat'],'lon':_f['lon'],
        'heading':_f['heading_deg'],'base':_f['base_geo_m'],
        'suppress_buildings':True,'suppress_roads':True,'suppress_trees':True,
        'source_exclusion_polygons':_source_polys,'road_exclusion_polygons':_polys,
        'max_source_building_span_m':260,
        'terrain_flatten':[],
        'terrain_grades':[{'polygon_xz':p,'plane':[0,0,-.08],'feather_m':7.5}for p in _polys],
        'description':_a['description'],
        'attribution':'Original Low Poly Earth geometry; mapped placement and source rings © OpenStreetMap contributors (ODbL), via Protomaps.',
        'references':_a['references'],
        'audit':{
            'architectural_height_m':_a['architectural_height_m'],
            'tip_height_m':_a['tip_height_m'],
            'published_controls':_a['published_controls'],
            'estimates':_a['estimated_details'],
            'ground_datum_m':_f['base_geo_m'],
            'ground_method':_f['datum_method'],
            'foundation_bottom_m':_f['foundation_bottom_m'],
            'terrain_ownership':'Grades are confined to modeled ground footprints with7.5m transition;fixed architecture datum is independent of those grades. Below-ground foundation depth includes0.6m burial beneath the lowest retained perimeter sample.',
            'placement_file':str(REF/(_name+'-placement.json')),
            'engine_review':'Pending actual engine overview and low-angle entrance/support inspection.',
        },
        'capture':{'distance':_a['tip_height_m']*2.05,'azimuth':225,'elevation':22,
                   'target_height':_a['architectural_height_m']*.48},
    })

_CONTACT={
    'shanghai-tower':([0,-43],180,115),
    'shanghai-world-financial-center':([-63,-5],264.63,120),
    'jin-mao-tower':([0,-45],180,115),
    'ping-an-finance-centre':([-36,0],270,95),
    'guangzhou-ctf-finance-centre':([0,-34],180,100),
    'guangzhou-international-finance-center':([27,0],90,105),
    'citic-tower':([0,40],0,120),
    'cctv-headquarters':([72,-50],90,140),
    'two-international-finance-centre':([0,28],14.7,105),
    'international-commerce-centre':([-31,0],222.25,105),
    'bank-of-china-tower':([0,26],16,100),
    'hsbc-main-building':([4,32],14.7,105),
    'lotte-world-tower':([0,-40],160,110),
    '85-sky-tower':([-39,0],251.7,130),
}
for _e in ENTRIES:
    _target,_az,_distance=_CONTACT[_e['name']]
    _e['audit']['contact_capture']={'target_xz':_target,'target_height':1.7,
                                  'distance':_distance,'azimuth':_az,'elevation':7}
    if _e['name']=='shanghai-world-financial-center':_e['capture']['azimuth']=129.63
    if _e['name']=='85-sky-tower':_e['capture']['azimuth']=251.7
    if _e['name']=='hsbc-main-building':_e['capture']['azimuth']=344.7

DEPENDENCIES=[Path(__file__).parent/'east_asia_geometry.py',
              Path(__file__).parent/'appearance.py',
              Path(__file__).parent/'geometry.py',Path(__file__).parent/'inventory.py',
              Path(__file__).resolve().parents[2]/'crates/vearth-gen/src/highrise.rs',
              Path(__file__).resolve().parents[2]/'crates/vearth-gen/src/building_surface.rs',
              *REF.glob('*.json')]


def build(name,lod):
    if lod not in (0,1,2):raise ValueError('expected LOD0,1 or2')
    return BUILDERS[name](lod)
