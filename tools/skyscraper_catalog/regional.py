"""Metric facade and measured-plan utilities for the Europe/Middle East models."""
import json
import math
from pathlib import Path
import numpy as np
from skyscraper_catalog.geometry import Mesh, face, plan_solid, plan_triangles, curtain_wall
from heritage_catalog.geometry import lathe
from heritage_catalog.materials import WINDOW, SCULPTURE_METAL, FLAT_ROOF, STONE_WALL, WEATHERED_LIMESTONE, PANEL_WALL

ROOT=Path(__file__).parent
METAL=(174,184,186)
GLASS=(87,130,143)


def read(group,name,suffix):
    return json.loads((ROOT/'references'/group/f'{name}-{suffix}.json').read_text())


def ccw(p):
    p=np.asarray(p,float)
    if np.linalg.norm(p[0]-p[-1])<1e-6:p=p[:-1]
    if sum(a[0]*b[1]-a[1]*b[0]for a,b in zip(p,np.roll(p,-1,axis=0)))<0:p=p[::-1]
    return p


def rect(w,d,c=(0,0)):
    return np.asarray([[-w/2,-d/2],[w/2,-d/2],[w/2,d/2],[-w/2,d/2]])+c


def ellipse(rx,rz,n=64,c=(0,0)):
    return np.array([(c[0]+rx*math.cos(t),c[1]+rz*math.sin(t))for t in np.linspace(0,2*math.pi,n,endpoint=False)])


def parts(group,name):
    return read(group,name,'plans')['parts']


def main_parts(group,name):
    ids=read(group,name,'placement')['main_seeds']
    return [q for q in parts(group,name)if q['seed']in ids]


def cap(m,p,y,color=(116,123,122),material=FLAT_ROOF,normal=(0,1,0)):
    for t in plan_triangles(p):face(m,[(x,y,z)for x,z in t],color,material,normal)


def prism_part(m,q,lo,hi,color,material=STONE_WALL,sides=True):
    # Stored triangulation deliberately excludes all source courtyard holes.
    for t in q['triangles']:
        face(m,[(x,hi,z)for x,z in t],color,material)
        face(m,[(x,lo,z)for x,z in t],color,material,(0,-1,0))
    if not sides:return
    for ri,ring in enumerate([q['outer']]+q['holes']):
        p=ccw(ring)
        if ri:p=p[::-1]
        for a,b in zip(p,np.roll(p,-1,axis=0)):
            e=b-a;face(m,[(a[0],lo,a[1]),(b[0],lo,b[1]),(b[0],hi,b[1]),(a[0],hi,a[1])],color,material,(e[1],0,-e[0]))


def divided(p,pitch):
    p=ccw(p);out=[]
    for a,b in zip(p,np.roll(p,-1,axis=0)):
        n=max(1,math.ceil(np.linalg.norm(b-a)/pitch))
        out.extend(a+(b-a)*j/n for j in range(n))
    return np.asarray(out)


PROFILES={
    103:((104,117,123),(91,129,145)),104:((165,170,169),(137,157,165)),
    105:((92,83,70),(129,113,91)),106:((179,166,143),(78,99,108)),
    107:((163,170,167),(85,114,126)),108:((188,184,172),(77,104,118)),
    109:((164,158,143),(77,104,118)),110:((151,152,145),(73,96,106))}


def loft(m,shape,levels,lod,glass=GLASS,metal=METAL,bay=2.1,spandrel=.7,mullion=.10,material=SCULPTURE_METAL,cap_top=True):
    """Native facade materials supply filtered floors; mesh strips follow shape only."""
    source_levels=list(levels);rings=[ccw(shape(y))for y in source_levels]
    if len({len(p)for p in rings})!=1:raise ValueError('loft rings must have matching topology')
    selected={0,len(rings)-1};tolerance=(.12,.35,.85)[lod]
    def simplify(a,b):
        if b<=a+1:return
        errors=[]
        for i in range(a+1,b):
            t=(source_levels[i]-source_levels[a])/(source_levels[b]-source_levels[a])
            errors.append(float(np.max(np.linalg.norm(rings[i]-(rings[a]*(1-t)+rings[b]*t),axis=1))))
        largest=max(errors);i=a+1+errors.index(largest)
        if largest>tolerance or (callable(glass) and source_levels[b]-source_levels[a]>20):
            selected.add(i);simplify(a,i);simplify(i,b)
    simplify(0,len(rings)-1);indices=sorted(selected)
    facade=getattr(m,'facade_material',103);wall,profile_glass=PROFILES[facade]
    tint=tuple(round(g*.62+w*.38)for g,w in zip(profile_glass,wall))
    pier_material=PANEL_WALL if material==SCULPTURE_METAL else material
    for floor,(ia,ib)in enumerate(zip(indices,indices[1:])):
        lo,hi=source_levels[ia],source_levels[ib];p,q=rings[ia],rings[ib]
        if hi<=lo:continue
        for i,(a,b,c,d)in enumerate(zip(p,np.roll(p,-1,axis=0),np.roll(q,-1,axis=0),q)):
            edge=(b-a+c-d)/2;normal=np.array([edge[1],0,-edge[0]])
            if np.linalg.norm(normal)<1e-8:continue
            normal/=np.linalg.norm(normal)
            pts=np.array([[a[0],lo,a[1]],[b[0],lo,b[1]],[c[0],hi,c[1]],[d[0],hi,d[1]]])
            if i in getattr(m,'facade_omit_edges',set()):continue
            width=(np.linalg.norm(b-a)+np.linalg.norm(c-d))/2
            n=max(1,min((12,8,4)[lod],int(width/(4,6,10)[lod])))
            pier=.65 if facade>=108 else .32
            def pt(u,v):return pts[0]*(1-u)*(1-v)+pts[1]*u*(1-v)+pts[2]*u*v+pts[3]*(1-u)*v
            def panel(u0,u1,color,mat):
                if u1-u0>1e-7:face(m,[pt(u0,0),pt(u1,0),pt(u1,1),pt(u0,1)],color,mat,normal)
            last=0
            for k in range(n):
                gap=min(.15,pier/max(.01,width));u0=k/n+gap;u1=(k+1)/n-gap
                if u1<=u0:continue
                panel(last,u0,wall,pier_material)
                tone=tint
                if callable(glass):tone=tuple(round(v*.55+w*.45)for v,w in zip(glass(int(lo/3.9),i,k),wall))
                panel(u0,u1,tone,facade);last=u1
            panel(last,1,wall,pier_material)
    if cap_top:cap(m,rings[-1],source_levels[-1])


def floor_levels(lo,hi,pitch,lod=0):
    step=pitch*(1,1,2)[lod]
    return [lo]+list(np.arange(math.floor(lo/step)*step+step,hi-.01,step))+[hi]


def vertical(m,p,lo,hi,lod,glass=GLASS,metal=METAL,pitch=4,bay=2,spandrel=.7,material=SCULPTURE_METAL):
    loft(m,lambda y:p,floor_levels(lo,hi,pitch,lod),lod,glass,metal,bay,spandrel,material=material)


def curtain_wall(m,a,b,bottom,top,floor_m,bay_m,glass,metal,lod,spandrel=.8,mullion=.14,facade_material=105):
    a,b=np.asarray(a),np.asarray(b);edge=b-a;width=np.linalg.norm(edge)
    if width<1e-7:return
    normal=np.array([edge[1],0,-edge[0]])/width
    facade=getattr(m,'facade_material',facade_material);wall,glazing=PROFILES[facade]
    tint=tuple(round(g*.62+w*.38)for g,w in zip(glazing,wall))
    def pt(u,y,relief=0):
        p=a+(b-a)*u;return np.array([p[0],y,p[1]])+normal*relief
    n=max(1,min((12,8,4)[lod],int(width/(4,6,10)[lod])));gap=min(.15,.32/width)
    last=0
    def panel(a,b,color,mat):
        if b-a>1e-7:face(m,[pt(a,bottom),pt(b,bottom),pt(b,top),pt(a,top)],color,mat,normal)
    for j in range(n):
        u,v=j/n+gap,(j+1)/n-gap
        if v>u:panel(last,u,wall,PANEL_WALL);panel(u,v,tint,facade);last=v
    panel(last,1,wall,PANEL_WALL)


def strip(m,a,b,y,width,height,color=METAL,material=SCULPTURE_METAL):
    a,b=np.array(a),np.array(b);e=b-a;n=np.array([e[1],-e[0]])/np.linalg.norm(e)*width/2
    plan_solid(m,[a+n,b+n,b-n,a-n],y,y+height,color,material)


def footings(m,group,name,base,color=(111,115,111),top=.2):
    samples=read(group,name,'ground')['samples']['toe']
    # A buried common lower cap avoids interpolation cracks between mesh LODs;
    # only mapped building solids receive this support, never adjoining streets.
    bottom=min(s['ground_m']for s in samples)-base-.65
    for q in main_parts(group,name):prism_part(m,q,min(bottom,-.65),top,color,STONE_WALL)


def entry(group,name,city,base,height,tip,description,urls,estimates,extra_owned=(),ground_grade=False):
    fit=read(group,name,'placement');qs=main_parts(group,name)
    polygons=[t for q in qs for t in q['triangles']]+list(extra_owned)
    e={'name':name,'city':city,'lat':fit['lat'],'lon':fit['lon'],'heading':fit['heading'],'base':base,
       'suppress_buildings':True,'suppress_roads':False,'suppress_trees':True,
       'source_exclusion_polygons':polygons,'max_source_building_span_m':400 if name=='palace-of-culture-and-science'else 230,
       'terrain_flatten':[],'terrain_cuts':[{'polygon_xz':p,'plane':[0,0,-.15],'feather_m':1.2}for p in polygons],
       'description':description,'attribution':'Original Low Poly Earth architectural mesh. Mapped plans © OpenStreetMap contributors, ODbL 1.0, via Protomaps; primary dimensions cited separately.',
       'references':urls,'audit':{'architectural_height_m':height,'tip_height_m':tip,'ground_datum_m':base,
       'placement_file':str((ROOT/'references'/group/f'{name}-placement.json').relative_to(ROOT.parent.parent)),
       'ground_file':str((ROOT/'references'/group/f'{name}-ground.json').relative_to(ROOT.parent.parent)),
       'estimates':estimates,'ground_method':'Selected perimeter/street elevation control, fixed metric datum; footprint-only lower cuts and buried support. Provider elevations are not a surveyed building datum. Actual-engine ground-contact acceptance is recorded separately under output/skyscrapers.'},
       'capture':{'distance':max(300,tip*2.1),'azimuth':225,'elevation':21,'target_height':tip*.48}}
    if ground_grade:
        e['terrain_grades']=[{'polygon_xz':p,'plane':[0,0,-.15],'feather_m':1.2}for p in polygons]
    vertices=np.array([v for p in polygons for v in p]);e['activation_bounds_xz']=[vertices.min(axis=0).tolist(),vertices.max(axis=0).tolist()]
    return e
