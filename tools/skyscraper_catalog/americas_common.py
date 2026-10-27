"""Facade construction and measured placement for the American additions."""
import json, math
from pathlib import Path
import numpy as np
from skyscraper_catalog.geometry import Mesh, face, plan_solid, plan_triangles
from heritage_catalog.geometry import lathe
from skyscraper_catalog.appearance import muted_frame, panel_tint, panel_count
from heritage_catalog.materials import WINDOW, SCULPTURE_METAL, FLAT_ROOF, STONE_WALL, PANEL_WALL, WEATHERED_LIMESTONE

REF=Path(__file__).parent/'references/americas'
STONE=(204,199,184); DARK=(40,46,47); GLASS=(89,121,129); METAL=(146,155,153)

def ccw(p):
    p=np.asarray(p,float)
    return p if sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))>0 else p[::-1]

def rect(w,d,x=0,z=0):
    return np.array([(x-w/2,z-d/2),(x+w/2,z-d/2),(x+w/2,z+d/2),(x-w/2,z+d/2)])

def fit(name):return json.loads((REF/(name+'-placement.json')).read_text())
def part(name,index):return ccw(next(p['polygon_xz']for p in fit(name)['parts']if p['source_index']==index))

def roof(m,p,y,color=(125,131,127),material=FLAT_ROOF):
    for tri in plan_triangles(p):face(m,[(x,y,z)for x,z in tri],color,material)

def loft(m,p0,p1,y0,y1,color,mat=SCULPTURE_METAL,caps=True):
    p0,p1=ccw(p0),ccw(p1)
    assert len(p0)==len(p1)
    for i in range(len(p0)):
        j=(i+1)%len(p0);a,b,c,d=p0[i],p0[j],p1[j],p1[i];v=b-a
        face(m,[(a[0],y0,a[1]),(b[0],y0,b[1]),(c[0],y1,c[1]),(d[0],y1,d[1])],color,mat,(v[1],0,-v[0]))
    if caps:
        roof(m,p1,y1,color,mat)
        for tri in plan_triangles(p0):face(m,[(x,y0,z)for x,z in tri],color,mat,(0,-1,0))

def facade(m,p0,p1,y0,y1,floor,bay,lod,stone=False,glass=GLASS,metal=METAL,spandrel=.65,mullion=.14,wall_material=None,native_material=None,explicit_grid=False):
    p0,p1=ccw(p0),ccw(p1);mat=wall_material or (WEATHERED_LIMESTONE if stone else PANEL_WALL)
    metal=muted_frame(metal)
    roof(m,p1,y1,metal,mat)
    for tri in plan_triangles(p0):face(m,[(x,y0,z)for x,z in tri],metal,mat,(0,-1,0))
    if not explicit_grid:
        native_material=native_material or (106 if stone else 103)
        tint=panel_tint(native_material,metal)
        for i,(a,b)in enumerate(zip(p0,np.roll(p0,-1,axis=0))):
            c,d=p1[(i+1)%len(p1)],p1[i];width=np.linalg.norm(b-a)
            n=np.array([(b-a)[1],-(b-a)[0]])/width
            def point(u,y):
                t=(y-y0)/(y1-y0);v=(a+(b-a)*u)*(1-t)+(d+(c-d)*u)*t
                return(v[0],y,v[1])
            count=panel_count(width,bay,lod);pier=max(.65 if stone else .32,mullion)
            inset=min(.25,pier/(width/count))
            def strip(u0,u1,color,material):
                face(m,[point(u0,y0),point(u1,y0),point(u1,y1),point(u0,y1)],color,material,(n[0],0,n[1]))
            for k in range(count):
                left,right=k/count,(k+1)/count;u0=(k+inset)/count;u1=(k+1-inset)/count
                strip(left,u0,metal,mat);strip(u0,u1,tint,native_material);strip(u1,right,metal,mat)
        return
    glass=panel_tint(native_material or (106 if stone else 103),metal)
    pitch=floor*(2 if lod==2 else 1)
    for i,(a,b)in enumerate(zip(p0,np.roll(p0,-1,axis=0))):
        c,d=p1[(i+1)%len(p1)],p1[i];n=np.array([(b-a)[1],-(b-a)[0]]);n/=np.linalg.norm(n)
        rows=max(1,round((y1-y0)/pitch));cols=max(1,round(np.linalg.norm(b-a)/(bay*(2 if lod==2 else 1))))
        def point(u,y):
            t=(y-y0)/(y1-y0);v=(a+(b-a)*u)*(1-t)+(d+(c-d)*u)*t
            return(v[0],y,v[1])
        inset=min(.36,mullion/max(1,np.linalg.norm(b-a)/cols))
        def cell(u0,u1,lo,hi,color,material):
            if hi>lo and u1>u0:
                face(m,[point(u0,lo),point(u1,lo),point(u1,hi),point(u0,hi)],color,material,(n[0],0,n[1]))
        for row in range(rows):
            start=y0+row*(y1-y0)/rows;end=y0+(row+1)*(y1-y0)/rows
            lo=min(end,start+spandrel);hi=max(lo,end-.15)
            cell(0,1,start,lo,metal,mat);cell(0,1,hi,end,metal,mat)
            for col in range(cols):
                left,right=col/cols,(col+1)/cols;u0=(col+inset)/cols;u1=(col+1-inset)/cols
                cell(left,u0,lo,hi,metal,mat);cell(u0,u1,lo,hi,glass,WINDOW);cell(u1,right,lo,hi,metal,mat)


def measured_entry(name,city,height,description,refs,owned=None,base=None):
    f=fit(name);polys=[ccw(p['polygon_xz'])for p in f['parts']]if owned is None else owned
    points=np.concatenate(polys);lo,hi=points.min(0),points.max(0);h=math.radians(f['heading_deg']);r=np.array([[math.cos(h),-math.sin(h)],[math.sin(h),math.cos(h)]])
    d=json.loads((REF/(name+'-survey.json')).read_text());near=[]
    for s in d['ground_samples']:
        p=(np.array([s['east_m'],s['north_m']])-f['center_en_m'])@r.T
        if np.all(p>=lo-12)and np.all(p<=hi+12):near.append(s['ground_m'])
    if not near:near=[d['ground_m']]
    base=round(float(np.median(near)),2)if base is None else base
    bottom=min(-.75,min(near)-base-.75)
    tris=[t.tolist()for p in polys for t in plan_triangles(p)]
    return dict(name=name,city=city,lat=f['lat'],lon=f['lon'],heading=f['heading_deg'],base=base,
      suppress_buildings=True,suppress_roads=True,suppress_trees=True,source_exclusion_polygons=tris,road_exclusion_polygons=tris,
      max_source_building_span_m=max(150,float(np.linalg.norm(hi-lo))+25),terrain_flatten=[],
      terrain_cuts=[dict(polygon_xz=t,plane=[0,0,-.12],feather_m=1.5)for t in tris],
      description=description,attribution='Original Low Poly Earth geometry; mapped placement © OpenStreetMap contributors (ODbL), via Protomaps.',references=refs,
      audit=dict(architectural_height_m=height,ground_datum_m=base,foundation_bottom_m=bottom,perimeter_ground_range_m=[min(near),max(near)],survey_file=str(REF.relative_to(Path(__file__).resolve().parents[2])/(name+'-survey.json')),placement_file=str(REF.relative_to(Path(__file__).resolve().parents[2])/(name+'-placement.json')),estimates='Mapped footprints and source terrain determine placement; facade bay spacing, subsidiary roof levels and ornament are modeled approximations. Rendered contact inspection accompanies the final assets.'),
      capture=dict(distance=max(250,height*1.8),azimuth=220,elevation=22,target_height=height*.45))

def foundation(m,e,polys,color=STONE,top=.05,mat=STONE_WALL):
    for p in polys:plan_solid(m,p,e['audit']['foundation_bottom_m'],top,color,mat)

def pin(m,x,z,y,h,r,color=STONE,lod=0):
    lathe(m,(x,z),[(y,r),(y+h*.60,r*.8),(y+h,.05)],color,WEATHERED_LIMESTONE,(8,6,4)[lod])
