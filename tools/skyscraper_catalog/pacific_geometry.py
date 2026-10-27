"""Plan-preserving envelopes and measured support toes for the Pacific collection."""
import json
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.geometry import Mesh, face, plan_solid, plan_triangles, curtain_wall
from heritage_catalog.geometry import lathe
from heritage_catalog.materials import WINDOW, SCULPTURE_METAL, FLAT_ROOF, STONE_WALL, PANEL_WALL


def site(group, name):
    return json.loads((Path(__file__).parent/'references'/group/(name+'-site.json')).read_text())


def clean(poly):
    p=np.asarray(poly,float)
    if np.linalg.norm(p[0]-p[-1])<1e-5:p=p[:-1]
    p=np.asarray([a for i,a in enumerate(p) if np.linalg.norm(a-p[i-1])>1e-5])
    if sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
    return p


def part(data, identifier):
    return clean(next(w['polygon_xz_m'] for w in data['osm_parts'] if w['id']==identifier))


def production_part(data, seed):
    return clean(next(b['rings'][0]for b in data['production_buildings']if str(b['seed'])==str(seed)))


def rect(cx,cz,width,depth,angle=0):
    p=np.array([[-width/2,-depth/2],[width/2,-depth/2],[width/2,depth/2],[-width/2,depth/2]])
    t=math.radians(angle);r=np.array([[math.cos(t),-math.sin(t)],[math.sin(t),math.cos(t)]])
    return p@r.T+[cx,cz]


def roof(m,p,y,color=(132,137,134)):
    for tri in plan_triangles(p):face(m,[(x,y,z)for x,z in tri],color,FLAT_ROOF)


def envelope(m,p,lo,hi,lod,glass=(91,129,145),metal=(104,117,123),floor=4.0,bay=4.0,spandrel=.55,material=103):
    p=clean(p);roof(m,p,hi)
    tint=tuple(round(.62*g+.38*w)for g,w in zip(glass,metal))
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        length=np.linalg.norm(b-a)
        if length<.1:continue
        d=(b-a)/length;n=np.array([d[1],-d[0]])
        point=lambda u,y,relief=0:(a[0]+d[0]*u+n[0]*relief,y,a[1]+d[1]*u+n[1]*relief)
        count=max(1,min((12,6,1)[lod],int(length/max(4,bay))))
        pier=.65 if material in (108,109,110) else .32
        for i in range(count):
            l=i*length/count;r=(i+1)*length/count
            intervals=[(l,l+pier/2,metal,PANEL_WALL),(l+pier/2,r-pier/2,tint,material),(r-pier/2,r,metal,PANEL_WALL)] if count>1 else [(l,r,tint,material)]
            for u,v,color,mat in intervals:
                face(m,[point(u,lo),point(v,lo),point(v,hi),point(u,hi)],color,mat,(n[0],0,n[1]))


def ground_at(data,x,z):
    if data.get('footing_ground_samples'):
        samples=data['footing_ground_samples']
        distances=[(v['east_m']-x)**2+(v['north_m']-z)**2 for v in samples]
        i=int(np.argmin(distances))
        if distances[i]>1e-10:raise ValueError(f'unmeasured foundation point: {data["name"]} {(x,z)}')
        return samples[i]['ground_m']
    # The provider's 8m pitch has already been sampled into the retained grid.
    # Bilinear interpolation is bounded by those measured samples, not a roof datum.
    s=data['ground_samples'];xs=sorted(set(v['east_m']for v in s));zs=sorted(set(v['north_m']for v in s))
    x=max(xs[0],min(xs[-1],x));z=max(zs[0],min(zs[-1],z))
    i=min(len(xs)-2,max(0,int(np.searchsorted(xs,x)-1)));j=min(len(zs)-2,max(0,int(np.searchsorted(zs,z)-1)))
    h={(v['east_m'],v['north_m']):v['ground_m']for v in s}
    u=(x-xs[i])/(xs[i+1]-xs[i]);v=(z-zs[j])/(zs[j+1]-zs[j])
    return (1-u)*(1-v)*h[xs[i],zs[j]]+u*(1-v)*h[xs[i+1],zs[j]]+(1-u)*v*h[xs[i],zs[j+1]]+u*v*h[xs[i+1],zs[j+1]]


def support(m,data,p,base,top=0,color=(155,153,145)):
    p=clean(p);roof(m,p,top,color)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        n=max(1,math.ceil(np.linalg.norm(b-a)/6))
        for t0,t1 in zip(np.linspace(0,1,n+1)[:-1],np.linspace(0,1,n+1)[1:]):
            c=a+(b-a)*t0;d=a+(b-a)*t1
            # A lower-only footprint cut caps high DSM samples; the support toe
            # also follows lower terrain, with half a metre of burial.
            yc=min(top-.5,ground_at(data,*c)-base-.5);yd=min(top-.5,ground_at(data,*d)-base-.5)
            face(m,[(c[0],yc,c[1]),(d[0],yd,d[1]),(d[0],top,d[1]),(c[0],top,c[1])],color,STONE_WALL,(d[1]-c[1],0,c[0]-d[0]))


def slab(m,p,y,thickness=.28,color=(176,181,179),material=PANEL_WALL):
    plan_solid(m,clean(p),y-thickness,y,color,material)


def ribs(m,p,lo,hi,pitch=6,width=.2,color=(179,185,183),lod=0,material=PANEL_WALL):
    if lod==2:return
    seen=set()
    for index,(a,b) in enumerate(zip(p,np.roll(p,-1,axis=0))):
        if lod==1 and index%2:continue
        edge=b-a;length=np.linalg.norm(edge)
        if length<1:continue
        tangent=edge/length;normal=np.array([tangent[1],-tangent[0]])
        n=max(1,round(length/pitch))
        for t in np.linspace(0,1,n+1):
            q=a+t*edge
            key=tuple(np.round(q,3))
            if key in seen:continue
            seen.add(key)
            # Embedded rear edges and a distinct 0.45m front face avoid the
            # near-coplanar cylinder/glass intersections visible at world scale.
            v0=q-tangent*width/2-normal*.08;v1=q+tangent*width/2-normal*.08
            v2=q+tangent*width/2+normal*.45;v3=q-tangent*width/2+normal*.45
            bottom=lo-.5 if lo==0 else lo
            plan_solid(m,[v0,v1,v2,v3],bottom,hi,color,material)


def vegetation(m,x,y,z,rx=1.4,ry=1.1,rz=1.4,lod=0,tone=0):
    # Discrete planted crowns, used only at documented garden terraces.
    m.ellipsoid((x,y,z),(rx,ry,rz),[(66,91,53),(78,103,61),(86,108,69)][tone%3],14,segments=(6,5,4)[lod],rings=(4,3,3)[lod])


def make_entry(data,city,base,owned,height,description,refs,audit,capture=None):
    polygons=[tri.tolist()for p in owned for tri in plan_triangles(clean(p))]
    return {'name':data['name'],'city':city,'lat':data['lat'],'lon':data['lon'],'heading':0,'base':base,
        'suppress_buildings':True,'suppress_roads':True,'suppress_trees':True,
        'source_exclusion_polygons':polygons,'road_exclusion_polygons':polygons,
        'max_source_building_span_m':max(150,max(np.ptp(p,axis=0).max()for p in owned)+30),
        'terrain_flatten':[],'terrain_cuts':[{'polygon_xz':p,'plane':[0,0,-.1],'feather_m':1.5}for p in polygons],
        'description':description,'attribution':'Original Low Poly Earth geometry; plan measurements © OpenStreetMap contributors (ODbL), production vectors via Protomaps.',
        'references':refs,'audit':{'architectural_height_m':height,'ground_datum_m':base,**audit},
        'capture':capture or {'distance':height*2.3,'azimuth':225,'elevation':23,'target_height':height*.43}}


def halfplane(p,normal,offset):
    """Clip a simple plan to dot(point, normal) >= offset."""
    out=[];p=clean(p);normal=np.asarray(normal,float)
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        da=np.dot(a,normal)-offset;db=np.dot(b,normal)-offset
        if da>=-1e-8:out.append(a)
        if (da<0)!=(db<0):out.append(a+(b-a)*da/(da-db))
    return clean(out)
