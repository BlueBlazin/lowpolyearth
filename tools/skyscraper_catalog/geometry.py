"""Metric facade panels over individually authored tower envelopes."""
import math
import numpy as np
from heritage_catalog.geometry import Mesh, face, plan_solid, plan_triangles
from heritage_catalog.materials import WINDOW, SCULPTURE_METAL, FLAT_ROOF, PANEL_WALL
from skyscraper_catalog.appearance import muted_frame, panel_tint, panel_count


def curtain_wall(mesh, a, b, bottom, top, floor_m, bay_m, glass, metal,
                 lod, spandrel=0.8, mullion=0.14, facade_material=105):
    a, b = np.asarray(a, float), np.asarray(b, float)
    direction = b-a
    width = np.linalg.norm(direction)
    direction /= width
    outward = np.array([direction[1], -direction[0]])
    point = lambda u, y, relief=0: (a[0]+direction[0]*u+outward[0]*relief,
                                    y, a[1]+direction[1]*u+outward[1]*relief)
    normal = (outward[0], 0, outward[1])
    frame=muted_frame(metal);tint=panel_tint(facade_material,frame)
    def strip(lo,hi,color,material):
        face(mesh,[point(lo,bottom),point(hi,bottom),point(hi,top),point(lo,top)],color,material,normal)
    if lod==2:
        strip(0,width,tint,facade_material)
        return
    bays=panel_count(width,bay_m,lod);pitch=width/bays
    pier=min(max(.32,mullion),pitch*.2)
    for bay in range(bays):
        left,right=bay*pitch,(bay+1)*pitch
        strip(left,left+pier,frame,PANEL_WALL)
        strip(left+pier,right-pier,tint,facade_material)
        strip(right-pier,right,frame,PANEL_WALL)


def exposed_grid_towers(mesh, tubes, module, floor_m, glass, metal, lod, bands=()):
    heights={(x,z):height for x,z,height in tubes}
    for x,z,height in tubes:
        cx,cz=x*module,z*module;r=module/2
        corners=[(cx-r,cz-r),(cx+r,cz-r),(cx+r,cz+r),(cx-r,cz+r)]
        for side,(dx,dz) in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
            bottom=heights.get((x+dx,z+dz),-.6)
            if bottom>=height:
                continue
            a,b=corners[side],corners[(side+1)%4]
            levels=sorted({bottom,height,*[max(bottom,min(height,y))for band in bands for y in band]})
            for low,high in zip(levels,levels[1:]):
                if any(lo<=(low+high)/2<=hi for lo,hi in bands):
                    face(mesh,[(a[0],low,a[1]),(b[0],low,b[1]),(b[0],high,b[1]),(a[0],high,a[1])],
                         (103,105,98),PANEL_WALL,(dx,0,dz))
                else:
                    curtain_wall(mesh,a,b,low,high,floor_m,module/5,glass,metal,lod,1.07,.14,103)
        face(mesh,[(a,height,b)for a,b in corners],(77,78,74),FLAT_ROOF)
