"""Seven original architectural meshes, reconstructed from public form references.

Metric envelopes and manually authored profiles preserve the recognizable forms;
no downloaded geometry, image texture, branding, or animated state is used.
"""
from __future__ import annotations
import math
import numpy as np
from common import Mesh

WHITE = (235, 234, 222)
PALE = (207, 211, 208)
SILVER = (180, 185, 184)
TITANIUM = (185, 180, 162)
LIMESTONE = (206, 191, 162)
GLASS = (44, 72, 83)
JOINT = (118, 131, 137)
WARM = (239, 192, 117)
STONE, METAL, GLOW = 11, 16, 94


def quad(mesh, points, color, material=STONE, normal=(0, 1, 0)):
    p = np.asarray(points, dtype=float)
    if np.dot(np.cross(p[1]-p[0], p[2]-p[0]), normal) < 0:
        p = p[::-1]
    mesh.quad(*p, color, material)


def fan(mesh, points, color, material=STONE, normal=(0, 1, 0)):
    p = np.asarray(points, dtype=float)
    for i in range(1, len(p)-1):
        a, b, c = p[0], p[i], p[i+1]
        if np.dot(np.cross(b-a, c-a), normal) < 0:
            b, c = c, b
        mesh.triangle(a, b, c, color, material)


def prism(mesh, outline, bottom, top, color, material=STONE):
    """Convex horizontal slab; outline is authored in x,z metres."""
    center = np.mean(outline, axis=0)
    for a, b in zip(outline, outline[1:]+outline[:1]):
        normal = ((a[0]+b[0])/2-center[0], 0, (a[1]+b[1])/2-center[1])
        quad(mesh, [(a[0],bottom,a[1]),(b[0],bottom,b[1]),
                    (b[0],top,b[1]),(a[0],top,a[1])], color, material, normal)
    fan(mesh,[(x,top,z) for x,z in outline],color,material)
    fan(mesh,[(x,bottom,z) for x,z in outline],color,material,(0,-1,0))


def ellipse(mesh, center, radii, bottom, top, color, segments=64, material=STONE):
    outline=[(center[0]+radii[0]*math.cos(i*math.tau/segments),
              center[1]+radii[1]*math.sin(i*math.tau/segments)) for i in range(segments)]
    prism(mesh,outline,bottom,top,color,material)


def line(mesh, points, radius, color, material=METAL, segments=5):
    """Closed continuous tube; shared joint rings avoid per-segment end caps."""
    pts=np.asarray(points,dtype=float)
    pts=pts[np.r_[True,np.linalg.norm(np.diff(pts,axis=0),axis=1)>1e-7]]
    if len(pts)<2:return
    rings=[]
    for i,p in enumerate(pts):
        direction=pts[min(i+1,len(pts)-1)]-pts[max(0,i-1)]
        direction/=np.linalg.norm(direction)
        ref=np.array((0.,1.,0.)) if abs(direction[1])<.9 else np.array((1.,0.,0.))
        u=np.cross(direction,ref);u/=np.linalg.norm(u);v=np.cross(direction,u)
        rings.append([p+radius*(u*math.cos(j*math.tau/segments)+v*math.sin(j*math.tau/segments)) for j in range(segments)])
    for i in range(len(rings)-1):
        for j in range(segments):
            k=(j+1)%segments;p=[rings[i][j],rings[i+1][j],rings[i+1][k],rings[i][k]]
            quad(mesh,p,color,material,np.mean(p,axis=0)-(pts[i]+pts[i+1])/2)
    fan(mesh,rings[0],color,material,pts[0]-pts[1])
    fan(mesh,rings[-1],color,material,pts[-1]-pts[-2])


def panel_grid(mesh, point, nu, nv, color, material=STONE, normal=(0,1,0), seams=False, thickness=0):
    """Independent subtly toned panels express scale without bitmap textures."""
    for i in range(nu):
        for j in range(nv):
            u0,u1,v0,v1=i/nu,(i+1)/nu,j/nv,(j+1)/nv
            shade=1+0.025*math.sin(i*2.17+j*1.73)
            tint=tuple(int(max(0,min(255,c*shade))) for c in color)
            p=[point(u0,v0),point(u1,v0),point(u1,v1),point(u0,v1)]
            quad(mesh,p,tint,material,normal)
            if thickness:
                quad(mesh,[np.asarray(q)-[0,thickness,0] for q in p],color,material,-np.asarray(normal))
    if thickness:
        for seq in [[point(i/nu,v) for i in range(nu+1)] for v in [0,1]]+[[point(u,j/nv) for j in range(nv+1)] for u in [0,1]]:
            for a,b in zip(seq,seq[1:]):
                a,b=np.asarray(a),np.asarray(b)
                direction=b-a;outward=np.cross(direction,[0,-1,0])
                quad(mesh,[a,b,b-[0,thickness,0],a-[0,thickness,0]],color,material,outward)
    if seams:
        for i in range(0,nu+1,4):
            line(mesh,[np.asarray(point(i/nu,j/nv))+np.asarray(normal)*.035 for j in range(nv+1)],.035,JOINT,METAL,4)
        for j in range(0,nv+1,4):
            line(mesh,[np.asarray(point(i/nu,j/nv))+np.asarray(normal)*.035 for i in range(nu+1)],.035,JOINT,METAL,4)


def _bilbao(lod):
    mesh=Mesh();n=(36,24,14)[lod];m=(16,10,6)[lod]
    prism(mesh,[(-78,-96),(20,-96),(121,-2),(111,32),(-38,61),(-78,30)],-9,1,(171,165,148))
    # Limestone galleries remain rectilinear around the curling titanium core.
    for x,z,w,d,h in [(-47,-22,37,36,24),(-42,-54,31,31,21),(-5,-60,33,27,19),
                       (18,-39,27,28,24),(-52,15,31,29,18),(-10,-80,25,27,17)]:
        mesh.box((x,1+h/2,z),(w,h,d),LIMESTONE)
        mesh.box((x,h+1.2,z),(w+.8,.5,d+.8),(177,173,156))
        if lod<2:
            for yy in np.arange(4,h,3):
                for side in [-1,1]:
                    mesh.box((x,yy,z+side*(d/2+.025)),(w,.055,.045),(160,151,132))
            for xx in np.arange(x-w/2+4,x+w/2,6):
                mesh.box((xx,h*.55,z-d/2-.06),(1.2,6,.08),GLASS,METAL)
    # Each closed loft is a distinct bent gallery volume, not an ellipsoid.
    def sail(cx,cz,length,width,height,angle,lean,phase):
        ca,sa=math.cos(angle),math.sin(angle)
        def point(u,v):
            t=math.tau*u
            z=(v-.5)*length
            taper=.64+.36*math.sin(math.pi*v)**.5
            x=width*taper*math.cos(t)
            y=1+height*(.5+.5*math.sin(t))*(.80+.20*math.sin(math.pi*v))
            x+=lean*(y/height)**2+2*math.sin(v*math.pi+phase)
            return np.array((cx+x*ca+z*sa,y,cz-x*sa+z*ca))
        for i in range(n):
            for j in range(m):
                p=[point(i/n,j/m),point((i+1)/n,j/m),point((i+1)/n,(j+1)/m),point(i/n,(j+1)/m)]
                normal=np.mean(p,axis=0)-np.array((cx,height*.4,cz))
                tint=tuple(int(c*(.955+.045*math.sin(i*1.77+j*.89)**2)) for c in TITANIUM)
                quad(mesh,p,tint,METAL,normal)
        for end in [0,1]:
            fan(mesh,[point(i/n,end) for i in range(n)],TITANIUM,METAL,
                (math.sin(angle)*(end*2-1),0,math.cos(angle)*(end*2-1)))
        if lod<2:
            for j in range(1,m,2):
                pts=[point(i/n,j/m) for i in range(n+1)]
                line(mesh,pts,.045,(132,134,126),METAL,4)
    sail(-16,12,56,16,43,.18,9,.5)
    sail(20,12,59,17,49,-.48,-6,.8)
    sail(4,36,38,13,38,.55,7,.3)
    sail(-32,25,38,12,33,-.70,-4,1.4)
    sail(44,-1,43,15,35,-.76,5,2.1)
    # The long eastern fish gallery tracks the river and reaches La Salve.
    sail(99,5,116,17,27,math.pi/2-.10,-2,.2)
    prism(mesh,[(43,-18),(161,-10),(166,23),(43,35)],-9,1,(171,165,148))
    prism(mesh,[(155,7),(166,10),(163,25),(157,20)],1,42,TITANIUM,METAL)
    # Recessed blue-glass atrium and its thin vertical mullions.
    mesh.box((0,19,-6),(25,36,23),GLASS,METAL)
    for x in np.linspace(-12,12,(13,9,5)[lod]):
        mesh.beam((x,1,-17.6),(x,37,-17.6),.12,SILVER,METAL,5)
    for y in range(6,37,6):mesh.box((0,y,-17.7),(24,.12,.10),SILVER,METAL)
    for x,z in [(-13,-7),(12,-7)]:
        mesh.box((x,8,z),(2,14,2),SILVER,METAL)
    mesh.box((0,2.8,-18),(19,.18,.3),WARM,GLOW)
    return mesh


def _hemisferic(lod):
    mesh=Mesh();nu=(64,40,24)[lod];nv=(30,20,12)[lod]
    ellipse(mesh,(0,0),(69,43),-5,.45,(209,212,197),(72,48,32)[lod])
    ellipse(mesh,(0,0),(65,39),.46,.53,(47,137,164),(72,48,32)[lod],16)
    ellipse(mesh,(0,0),(56,28.3),.54,1.1,WHITE,(72,48,32)[lod])
    def p(u,v):
        x=55*(2*u-1);r=math.sqrt(max(0,1-(x/55)**2));a=-math.pi/2+math.pi*v
        return np.array((x,1.1+25*r*math.cos(a),27.75*r*math.sin(a)))
    for i in range(nu):
        for j in range(nv):
            color=GLASS if j/nv<.37 else WHITE
            pts=[p(i/nu,j/nv),p((i+1)/nu,j/nv),p((i+1)/nu,(j+1)/nv),p(i/nu,(j+1)/nv)]
            quad(mesh,pts,color,METAL if color==GLASS else STONE,np.mean(pts,axis=0)-[0,1,0])
    # Long bright eyelid edge and separate structural ribs retain the eye shape.
    for v in [0,.37,1]:line(mesh,[p(i/nu,v) for i in range(nu+1)],.4,WHITE,STONE,6)
    for i in range(1,nu,2 if lod==0 else 1):
        u=i/nu
        line(mesh,[p(u,j/nv)*[1,1.002,1.002] for j in range(nv+1)],.18 if lod<2 else .22,WHITE,STONE,5)
    # Central planetarium iris is a recessed spherical cap behind the glazing.
    def iris(u,v):
        a=math.pi*u;r=17*math.sin(a)
        return (17*math.cos(a),1.2+r*math.sin(math.pi*v),-25.3+3.2*math.sin(a)*math.cos(math.pi*v))
    panel_grid(mesh,iris,(32,22,14)[lod],(16,12,8)[lod],(177,194,200),METAL,(0,0,-1))
    for side in [-1,1]:
        mesh.box((side*53,1.8,0),(15,1.4,5),WHITE)
        if lod<2:
            for z in [-2.6,2.6]:mesh.beam((side*47,2.7,z),(side*62,2.7,z),.07,SILVER,METAL,4)
    return mesh


def _oslo(lod):
    mesh=Mesh();prism(mesh,[(-115,-67),(104,-67),(104,67),(-112,67)],-2,.7,WHITE)
    # The public marble carpet splits into two broad sloping ramps around foyer.
    def roof_quad(points):
        quad(mesh,points,WHITE)
        quad(mesh,[(x,y-.55,z) for x,y,z in points],PALE,STONE,(0,-1,0))
        for a,b in zip(points,points[1:]+points[:1]):
            quad(mesh,[a,b,(b[0],b[1]-.55,b[2]),(a[0],a[1]-.55,a[2])],PALE,STONE,(a[0]+b[0],0,a[2]+b[2]))
        if lod<2:
            a,b,c,d=map(np.asarray,points)
            count=(22,12)[lod]
            for i in range(1,count):
                t=i/count;line(mesh,[a*(1-t)+b*t+[0,.025,0],d*(1-t)+c*t+[0,.025,0]],.025,(188,192,185),STONE,4)
    roof_quad([(-114,1,-67),(-3,17,-67),(-3,17,-37),(-101,1,-37)])
    roof_quad([(-113,1,35),(25,20,35),(25,20,66),(-113,1,66)])
    roof_quad([(-114,1,-38),(-86,5,-38),(-86,5,35),(-114,1,35)])
    # Broad glass foyer is exposed between the accessible roof planes.
    mesh.box((-40,7.6,-3),(91,13.8,72),GLASS,METAL)
    roof_quad([(-85,14.5,-39),(6,19.2,-39),(6,19.2,35),(-85,14.5,35)])
    for z in np.linspace(-38,33,(26,16,9)[lod]):
        mesh.beam((-85.6,1,z),(-85.6,14.5,z),.085,SILVER,METAL,4)
    for x in np.linspace(-82,4,(30,18,10)[lod]):
        h=14.5+(x+85)/91*4.7
        mesh.beam((x,1,-39.3),(x,h,-39.3),.085,SILVER,METAL,4)
    # Warm oak silhouette is visible as a narrow ribbon within the main lobby.
    if lod<2:
        for i in range(30):
            x=-80+i*2.8;y=4.7+.8*math.sin(i*.19)
            mesh.box((x,y,-39.45),(2.65,5,.07),(133,96,57),STONE)
    mesh.box((43,7.8,0),(122,14.2,126),(194,197,193))
    mesh.box((43,15.15,0),(123,.65,127),WHITE)
    mesh.box((14,24,11),(46,33,45),(212,216,213),METAL)
    mesh.box((14,40.7,11),(46.8,.7,45.8),PALE,METAL)
    for side in [-1,1]:
        for x in np.arange(-15,102,(5,8,14)[lod]):
            mesh.box((x,7.4,side*63.05),(3.2,4,.08),GLASS,METAL)
    for x in np.arange(-8,36,(2.5,5,9)[lod]):
        mesh.box((x,25,-11.6),(.14,30,.08),(165,174,178),METAL)
    if lod==0:
        for z in np.arange(-58,60,7):mesh.box((80,15.8,z),(18,.5,2.5),(156,167,172),METAL)
    mesh.box((-84.5,2.6,-1),(.15,.15,30),WARM,GLOW)
    return mesh


def _milwaukee(lod):
    mesh=Mesh();n=(36,24,16)[lod]
    prism(mesh,[(-39,-82),(16,-82),(30,-60),(30,56),(-36,66),(-49,12)],-1,4.5,WHITE)
    # Ship-like white prow, with the glazed Windhover Hall above its keel.
    prism(mesh,[(-18,-77),(18,-77),(26,-29),(23,43),(-23,43),(-26,-29)],4.5,7,WHITE)
    def hall(u,v):
        z=-73+77*u;r=math.sin(math.pi*u)**.35;a=math.pi*v
        return (19*r*math.cos(a),7+20.4*r*math.sin(a),z)
    panel_grid(mesh,hall,(48,32,20)[lod],(24,16,10)[lod],GLASS,METAL,(0,1,0))
    for j in range(n+1):
        u=j/n
        line(mesh,[hall(u,k/20) for k in range(21)],.20,WHITE,STONE,5)
    # Seventy-two tapered steel fins are held in their fixed open position.
    for side in [-1,1]:
        for j in range(n):
            u=(j+.5)/n;z=-57+52*u
            reach=8+25.05*math.sin(math.pi*u)**.85
            root=np.array((side*1.1,25.8,z))
            tip=np.array((side*reach,25.8+reach*.50,z-5*math.sin(math.pi*u)))
            tangent=np.array((0,0,.43 if lod==0 else .62))
            middle=root*.45+tip*.55+np.array((0,2.0,0))
            for a,b,width in [(root,middle,1),(middle,tip,.72)]:
                p=[a-tangent*width,b-tangent*.12,b+tangent*.12,a+tangent*width]
                quad(mesh,p,WHITE,STONE,(0,1,0));quad(mesh,[v+[0,-.35,0] for v in p],PALE,STONE,(0,-1,0))
                for k in [0,2]:quad(mesh,[p[k],p[(k+1)%4],p[(k+1)%4]+[0,-.35,0],p[k]+[0,-.35,0]],WHITE,STONE,(0,0,1 if k==2 else -1))
        mesh.beam((side*1.1,25.8,-58),(side*1.1,25.8,-4),.62,WHITE,STONE,8)
    # Landward Reiman Bridge and its inclined cable mast are static structure.
    mesh.box((-55,8,-22),(74,1.3,5.6),WHITE)
    mesh.beam((-20,5,-22),(-33,51,-22),.85,WHITE,STONE,8)
    for x in np.linspace(-89,-39,(12,8,5)[lod]):
        mesh.beam((x,8.7,-22),(-32,48,-22),.065,SILVER,METAL,4)
    for z in [-24.9,-19.1]:mesh.beam((-91,10,z),(-18,10,z),.08,WHITE,STONE,5)
    mesh.box((-17,6,53),(38,11,25),(196,200,196))
    mesh.box((-17,11.8,53),(38,.5,25),WHITE)
    mesh.box((0,7.15,-62),(20,.12,.20),WARM,GLOW)
    return mesh


def _heydar(lod):
    mesh=Mesh();nu=(88,56,32)[lod];nv=(38,26,16)[lod]
    ellipse(mesh,(0,-5),(124,91),-1,.5,(224,220,205),(96,64,40)[lod])
    # An asymmetrical continuous shell grows out of the plaza. The southern
    # edge peels upward to expose a recessed, curved glass entrance wall.
    def profile(u):
        x=-116+232*u
        h=1+28*math.exp(-((x+55)/40)**2)+72*math.exp(-((x-37)/42)**2)
        w=15+45*math.sin(math.pi*u)**.62
        center=7+13*math.sin(u*math.pi*1.25)
        return x,h,w,center
    def roof(u,v):
        x,h,w,center=profile(u)
        front=-1.35+.50*math.sin(math.pi*u)**.5
        a=front+(math.pi/2-front)*v
        return np.array((x,.5+h*math.cos(a),center+w*math.sin(a)))
    panel_grid(mesh,roof,nu,nv,WHITE,STONE,(0,1,0),lod==0,thickness=.30)
    for end in [0,1]:
        edge=[roof(end,j/nv) for j in range(nv+1)]
        fan(mesh,edge+[(edge[-1][0],.5,edge[-1][2]),(edge[0][0],.5,edge[0][2])],WHITE,STONE,(end*2-1,0,0))
    for i in range(nu):
        u0,u1=i/nu,(i+1)/nu;a,b=roof(u0,0),roof(u1,0)
        quad(mesh,[a,b,(b[0],.5,b[2]),(a[0],.5,a[2])],GLASS,METAL,(0,0,-1))
        if i%((2,2,3)[lod])==0:
            mesh.beam((a[0],.5,a[2]-.04),a+[0,0,-.04],.09,(139,151,154),METAL,4)
    for fraction in [.25,.50,.75]:
        line(mesh,[(p[0],.5+(p[1]-.5)*fraction,p[2]-.04)
                   for p in [roof(i/nu,0) for i in range(nu+1)]],.065,SILVER,METAL,4)
    # Low western fold returns into a vaulted opening, separating the hall
    # from the tall eastern crest while retaining a single white composition.
    def fold(u,v):
        x=-112+95*u
        rise=25*math.sin(math.pi*u)**1.2
        a=math.pi*v
        return (x,.55+rise*math.sin(a),-53-27*math.cos(a))
    panel_grid(mesh,fold,(40,26,16)[lod],(24,16,10)[lod],WHITE,STONE,(0,1,0),lod==0,thickness=.30)
    for i in range((32,20,12)[lod]):
        u0,u1=i/(32,20,12)[lod],(i+1)/(32,20,12)[lod]
        x0,x1=-106+81*u0,-106+81*u1
        y0,y1=.6+15*math.sin(math.pi*u0),.6+15*math.sin(math.pi*u1)
        quad(mesh,[(x0,.6,-78),(x1,.6,-78),(x1,y1,-78),(x0,y0,-78)],GLASS,METAL,(0,0,-1))
    # Gentle broad terrace bands express the deliberate topography of the site.
    for i in range(4):
        z=-84-i*2.5
        mesh.box((18,.18+i*.12,z),(112,.20,2.2),(225-i*2,220-i*2,204-i*2))
    mesh.box((37,1.6,-44),(43,.16,.25),WARM,GLOW)
    return mesh


def _khan(lod):
    mesh=Mesh();nu=(144,96,56)[lod];nv=(40,26,16)[lod]
    ellipse(mesh,(0,0),(103,100.5),-1,1.1,(179,178,164),(96,64,40)[lod])
    ellipse(mesh,(0,0),(99,96.5),1.1,8.0,GLASS,(96,64,40)[lod],METAL)
    def roof(u,v):
        a=math.tau*u;r=1-v
        y=8+126*v**1.55
        return np.array((100*r*math.cos(a)-23*v,y,97.5*r*math.sin(a)+19*v))
    for i in range(nu):
        for j in range(nv):
            p=[roof(i/nu,j/nv),roof((i+1)/nu,j/nv),roof((i+1)/nu,(j+1)/nv),roof(i/nu,(j+1)/nv)]
            shade=.95+.05*((i+j)%3)/2
            tint=tuple(int(v*shade) for v in (162,181,187))
            quad(mesh,p,tint,METAL,(math.cos(math.tau*(i+.5)/nu),0,math.sin(math.tau*(i+.5)/nu)))
    # Visible radial and circumferential cable mesh follows the tent membrane.
    for i in range(0,nu,(3,3,4)[lod]):
        line(mesh,[roof(i/nu,j/nv)+[0,.13,0] for j in range(nv+1)],.105,(220,221,209),METAL,5)
    for v in np.linspace(.05,.94,(16,12,8)[lod]):
        line(mesh,[roof(i/nu,v)+[0,.13,0] for i in range(nu+1)],.075,(114,131,140),METAL,4)
    for a in np.linspace(0,math.tau,(72,48,28)[lod],endpoint=False):
        x,z=99*math.cos(a),96.5*math.sin(a)
        mesh.beam((x,1.1,z),(x,8,z),.16,PALE,METAL,5)
    for a in [0,2.1,4.2]:
        mesh.beam((14*math.cos(a),1.2,14*math.sin(a)),(-23,134,19),1.4,PALE,METAL,8)
    mesh.beam((-23,130,19),(-27,150,22),.72,WHITE,METAL,8)
    ellipse(mesh,(-23,19),(4,4),132.5,134.5,PALE,(24,18,12)[lod],METAL)
    mesh.box((0,3.4,-98.1),(30,5.6,2.8),GLASS,METAL)
    mesh.box((0,6.3,-100),(34,.7,6),PALE)
    mesh.box((0,6,-103.1),(31,.15,.15),WARM,GLOW)
    return mesh


def _gate(lod):
    mesh=Mesh();rows=(96,64,36)[lod];cols=(16,10,6)[lod]
    # The two legs become a continuous arch, leaving a real 238m-high opening.
    def outer(y):return 75 if y<=200 else 75*math.sqrt(max(0,1-((y-200)/78)**2))
    def inner(y):return 25 if y<=200 else (25*math.sqrt(max(0,1-((y-200)/38)**2)) if y<238 else 0)
    def p(y,u,side,face):
        x=side*(inner(y)+(outer(y)-inner(y))*u)
        z=face*(23+4*math.sin(math.pi*u)*outer(y)/75)
        return np.array((x,y,z))
    for side in [-1,1]:
        for face in [-1,1]:
            for i in range(rows):
                y0,y1=278*i/rows,278*(i+1)/rows
                for j in range(cols):
                    u0,u1=j/cols,(j+1)/cols
                    color=(76,109,123) if (i+j)%7 else (99,128,138)
                    quad(mesh,[p(y0,u0,side,face),p(y1,u0,side,face),p(y1,u1,side,face),p(y0,u1,side,face)],color,METAL,(0,0,face))
            for y in np.linspace(3,275,(66,44,26)[lod]):
                for j in range(cols):
                    ps=[p(y+dy,u,side,face)+[0,0,face*.10] for dy,u in [(-.13,j/cols),(.13,j/cols),(.13,(j+1)/cols),(-.13,(j+1)/cols)]]
                    quad(mesh,ps,(174,187,190),METAL,(0,0,face))
            for u in np.linspace(0,1,(11,7,4)[lod]):
                for i in range(rows):
                    ps=[p(y,du,side,face)+[0,0,face*.12] for y,du in [(278*i/rows,max(0,u-.002)),(278*(i+1)/rows,max(0,u-.002)),(278*(i+1)/rows,min(1,u+.002)),(278*i/rows,min(1,u+.002))]]
                    quad(mesh,ps,(154,172,179),METAL,(0,0,face))
        for i in range(rows):
            y0,y1=278*i/rows,278*(i+1)/rows
            for edge in [0,1]:
                if edge==0 and y0>=238:continue
                quad(mesh,[p(y0,edge,side,-1),p(y1,edge,side,-1),p(y1,edge,side,1),p(y0,edge,side,1)],
                     (49,82,95) if edge==0 else (120,146,155),METAL,(side*(1 if edge else -1),0,0))
        prism(mesh,[(side*21,-32),(side*78,-32),(side*82,32),(side*21,32)],-1,5,(105,126,135),METAL)
        # Twin slender architectural spires continue the curved outer profile.
        root=np.array((side*23,273,0));tip=np.array((side*18,301.8,0))
        fan(mesh,[root+[-4,0,-1],root+[4,0,-1],tip+[0,0,-.6]],PALE,METAL,(0,0,-1))
        fan(mesh,[root+[-4,0,1],root+[4,0,1],tip+[0,0,.6]],PALE,METAL,(0,0,1))
        mesh.beam(root,tip,.42,SILVER,METAL,6)
    return mesh.transform(np.array([[1.17,0,0,8],[0,1,0,0],[0,0,1.66,0],[0,0,0,1]]))


_BUILDERS={"guggenheim-bilbao":_bilbao,"hemisferic":_hemisferic,"oslo-opera-house":_oslo,
           "milwaukee-art-museum":_milwaukee,"heydar-aliyev-center":_heydar,
           "khan-shatyr":_khan,"gate-of-the-orient":_gate}


def build(name,lod):
    if lod not in (0,1,2):raise ValueError("city model LOD must be 0, 1, or 2")
    return _BUILDERS[name](lod)


ENTRIES = [{'name': 'guggenheim-bilbao',
  'city': 'Bilbao',
  'lat': 43.26864,
  'lon': -2.934,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.guggenheim-bilbao.eus/en/the-building',
                 'https://www.guggenheim-bilbao.eus/el-edificio/la-construccion',
                 'https://www.openstreetmap.org/way/118499485'],
  'description': 'Curving titanium galleries, limestone wings, recessed glass atrium and the long gallery '
                 'beside La Salve.',
  'terrain_flatten': [{'center_xz': [-12, -17],
                       'half_extent': [67, 80],
                       'feather_m': 4,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 8.0},
                      {'center_xz': [106, 7],
                       'half_extent': [61, 31],
                       'feather_m': 4,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 8.0}],
  'capture': {'distance': 380,
              'azimuth': 25,
              'elevation': 25,
              'target_height': 20,
              'time_utc': '2026-03-20T15:30:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': 9.6485,
            'perimeter_m': [10.5891, 16.9416, 19.9594, 6.169, 9.11, 5.8328, 6.0028, 2.2278],
            'source_file': 'output/city-catalog-expansion/modern/guggenheim-bilbao-final.json',
            'placement_note': 'River-side samples are 2.23–9.11 m; southern road samples rise to '
                              '16.94–19.96 m. An 8 m river-facing datum avoids lifting the museum to the '
                              'elevated street. Two bounded cleanup rectangles follow the main galleries '
                              'and eastern gallery separately.'}},
 {'name': 'hemisferic',
  'city': 'Valencia',
  'lat': 39.4566823,
  'lon': -0.3538811,
  'heading': 42.2,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://cac.es/hemisferic/', 'https://www.openstreetmap.org/way/23433774'],
  'description': 'A white ribbed eye-shaped planetarium with dark glazing, central iris and a still '
                 'turquoise pool.',
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [69, 43],
                       'feather_m': 6,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 3.7}],
  'capture': {'distance': 240,
              'azimuth': 212,
              'elevation': 23,
              'target_height': 12,
              'time_utc': '2026-03-20T15:00:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': 3.4367,
            'perimeter_m': [6.2405, 3.4842, 4.4284, 6.937, 5.5577, 6.5639, 3.8872, 7.6633],
            'source_file': 'output/city-catalog-expansion/modern/hemisferic-final.json',
            'placement_note': 'The pool and sunken promenade samples are 3.44–4.43 m; surrounding raised '
                              'approaches reach 7.66 m. The selected 3.7 m datum follows the pool level.'}},
 {'name': 'oslo-opera-house',
  'city': 'Oslo',
  'lat': 59.9074842,
  'lon': 10.7526811,
  'heading': 22,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://old.snohetta.com/projects/42-norwegian-national-opera-and-ballet',
                 'https://www.operaen.no/en/about-us-oslo-operahouse/about-the-oslo-opera-house/',
                 'https://www.openstreetmap.org/way/810259696'],
  'description': 'Broad walkable marble roof ramps around a blue-glass foyer and restrained metallic stage '
                 'volumes.',
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [116, 69],
                       'feather_m': 6,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 0.5}],
  'capture': {'distance': 350,
              'azimuth': 225,
              'elevation': 26,
              'target_height': 15,
              'time_utc': '2026-03-20T12:30:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': 0.0036,
            'perimeter_m': [0.0, 0.0, 0.7595, 0.0, 0.7151, 0.6453, 0.3357, 2.5008],
            'source_file': 'output/city-catalog-expansion/modern/oslo-opera-house-final.json',
            'placement_note': 'Coastal perimeter median is 0.49 m, with verified sea-level zero samples. A '
                              '0.5 m datum keeps the marble waterfront carpet close to the quay.'}},
 {'name': 'milwaukee-art-museum',
  'city': 'Milwaukee',
  'lat': 43.039529,
  'lon': -87.8969658,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://mam.org/info/pressroom/2015/09/fact-sheet-for-the-reimagined-milwaukee-art-museum/',
                 'https://mam.org/info/pressroom/2010/10/anniversary/',
                 'https://www.openstreetmap.org/way/403894584'],
  'description': 'The white Quadracci Pavilion with seventy-two fixed-open wing fins, glazed Windhover '
                 'Hall and cable-stayed pedestrian bridge.',
  'terrain_flatten': [{'center_xz': [0, -8],
                       'half_extent': [51, 78],
                       'feather_m': 4,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 179.5}],
  'capture': {'distance': 270,
              'azimuth': 125,
              'elevation': 20,
              'target_height': 19,
              'time_utc': '2026-03-20T20:00:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': 181.2637,
            'perimeter_m': [179.572, 177.8221, 176.307, 179.8689, 176.1066, 180.6205, 180.0906, 179.3823],
            'source_file': 'output/city-catalog-expansion/modern/milwaukee-art-museum-final.json',
            'placement_note': 'Lake-side samples are 176.11–176.31 m, landward paths 179.57–180.62 m. The '
                              '179.5 m landward datum retains the authored podium; cleanup northern edge '
                              'plus feather stops at z74 before the existing galleries begin near z76.'}},
 {'name': 'heydar-aliyev-center',
  'city': 'Baku',
  'lat': 40.39613925,
  'lon': 49.8675167,
  'heading': -36.36,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.zha.com/projects/architecture/heydar-aliyev-centre?disclaimer=true',
                 'https://tyrensakt.com/projects/heydar-aliyev-centre/',
                 'https://www.openstreetmap.org/relation/19380164'],
  'description': 'A continuous white architectural wave, curved curtain wall, lower vaulted fold and '
                 'softly terraced plaza.',
  'terrain_flatten': [{'center_xz': [0, -5],
                       'half_extent': [124, 91],
                       'feather_m': 14,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 14.0}],
  'capture': {'distance': 385,
              'azimuth': 174,
              'elevation': 22,
              'target_height': 27,
              'time_utc': '2026-03-20T11:00:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': 14.0318,
            'perimeter_m': [5.0289, 16.7556, 16.8168, 6.943, 15.9676, 10.8573, 17.846, 21.8783],
            'source_file': 'output/city-catalog-expansion/modern/heydar-aliyev-center-final.json',
            'placement_note': 'The terrain retains the former site slope: 5.03–21.88 m around the '
                              'perimeter, with center 14.03 m. A 14 m central datum and 14 m feather '
                              'support the deliberately terraced architectural plaza.'}},
 {'name': 'khan-shatyr',
  'city': 'Astana',
  'lat': 51.1325089,
  'lon': 71.4038726,
  'heading': 0,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://www.fosterandpartners.com/projects/khan-shatyr-entertainment-center/',
                 'https://www.khanshatyr.com/index.php/en/about',
                 'https://www.openstreetmap.org/way/460703779'],
  'description': 'An asymmetrically tilted translucent tent, radial cable lattice, inclined tripod mast '
                 'and glazed entrance ring.',
  'terrain_flatten': [{'center_xz': [0, 0],
                       'half_extent': [105, 104],
                       'feather_m': 6,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': None}],
  'capture': {'distance': 415,
              'azimuth': 190,
              'elevation': 19,
              'target_height': 49,
              'time_utc': '2026-03-20T10:00:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': 344.0,
            'perimeter_m': [341.1318, 344.0, 343.0, 340.3163, 345.9787, 345.0, 345.0, 344.9655],
            'source_file': 'output/city-catalog-expansion/modern/khan-shatyr-final.json',
            'placement_note': 'Perimeter-derived terrain base; measured samples span 340.32–345.98 m, with '
                              'median 344.48 m. The mast and membrane share this datum.'}},
 {'name': 'gate-of-the-orient',
  'max_source_building_span_m': 300,
  'city': 'Suzhou',
  'lat': 31.3189418,
  'lon': 120.6747334,
  'heading': 78,
  'base': 'terrain',
  'suppress_buildings': True,
  'suppress_roads': False,
  'suppress_trees': True,
  'references': ['https://rmjm.com/portfolio/gate-to-the-east-china/',
                 'https://rmjm.com/the-10-tallest-rmjm-buildings/',
                 'https://www.skyscrapercenter.com/suzhou/gate-to-the-east/499/',
                 'https://www.openstreetmap.org/way/933819303'],
  'description': 'Two rounded glazed towers join into a monumental open arch below twin slender '
                 'architectural spires.',
  'terrain_flatten': [{'center_xz': [8, 0],
                       'half_extent': [97, 54],
                       'feather_m': 6,
                       'sample_clearance_m': 12,
                       'fixed_base_geo': 4.0}],
  'capture': {'distance': 620,
              'azimuth': 92,
              'elevation': 14,
              'target_height': 135,
              'time_utc': '2026-03-20T07:15:00Z'},
  'audit': {'source_dataset': '20260907-terrain-ff72be71951c0f92-bridges-b1c01b2e5d670c2a-regional-landcover-selective-v1-water-polygons-typed-v1-watercourses-rivers-canals-v1-bridge-roads-adaptive-v1-surface-pack-v2-monthly-regional-opt-in-v1-m09',
            'ground_center_m': -0.9823,
            'perimeter_m': [-2.6111, 2.7985, 4.6132, 5.7696, -4.5935, 3.4493, 7.5553, 5.9794],
            'source_file': 'output/city-catalog-expansion/modern/gate-of-the-orient-final.json',
            'placement_note': 'Source DEM is depressed under the building (center −0.98 m); landward '
                              'perimeter samples are mainly 2.80–7.56 m. A 4 m datum avoids subgrade '
                              'placement. Preserve the adjacent large Suzhou Center mall footprint.',
            'preserve_source_seed': '8991428007006568193',
            'preserve_source_bounds_xz': [[-251, -216], [214, -18]],
            'target_source_seeds': ['16645017429774736005',
                                    '5244302600832414237',
                                    '10234328233259004770',
                                    '17280357480106158784'],
            'target_source_bounds_xz': [[-88, -52], [104, 50]]}}]
