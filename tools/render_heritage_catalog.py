"""Blender inspection of the actual encoded LMS assets, preserving face colors.

Blender --background --factory-startup --python tools/render_heritage_catalog.py
  -- --only st-pauls-cathedral --views 210,30 30,30
"""
import argparse
import json
import math
from pathlib import Path
import struct
import sys
import zlib

import bpy
from mathutils import Vector
import numpy as np

ROOT=Path(__file__).resolve().parents[1]


def decode(path,lod):
    encoded=path.read_bytes()
    raw=zlib.decompress(encoded[4:])
    assert len(raw)==struct.unpack_from('<I',encoded)[0] and raw[:4]==b'LMS1'
    at=8
    vertex=np.dtype([('position','<f4',(3,)),('normal','<f4',(3,)),('color','u1',(4,))])
    for level in range(struct.unpack_from('<I',raw,4)[0]):
        nv,ni=struct.unpack_from('<II',raw,at);at+=8
        vertices=np.frombuffer(raw,dtype=vertex,count=nv,offset=at);at+=nv*28
        faces=np.frombuffer(raw,dtype='<u4',count=ni,offset=at).reshape(-1,3);at+=ni*4
        if level==lod:
            return vertices,faces
    raise ValueError('missing LOD')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--only',nargs='+',required=True)
    parser.add_argument('--catalog', type=Path,
                        default=ROOT/'crates/vearth-gen/assets/landmarks/heritage-surfaces/catalog.json')
    parser.add_argument('--lod',type=int,choices=(0,1,2),default=0)
    parser.add_argument('--views',nargs='+',default=['210,28','30,28','0,89'])
    parser.add_argument('--output',type=Path,default=ROOT/'output/heritage/previews')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    args.output.mkdir(parents=True,exist_ok=True)
    bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
    scene=bpy.context.scene
    scene.render.engine='BLENDER_EEVEE'
    scene.render.resolution_x=1400;scene.render.resolution_y=1050
    scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG'
    scene.view_settings.view_transform='Standard'
    scene.view_settings.look='None'
    scene.world.use_nodes=True
    background=scene.world.node_tree.nodes['Background']
    background.inputs[0].default_value=(.57,.63,.70,1)
    background.inputs[1].default_value=.65
    bpy.ops.object.light_add(type='SUN',location=(80,-120,180))
    bpy.context.object.rotation_euler=(math.radians(28),math.radians(-20),math.radians(-30))
    bpy.context.object.data.energy=2.4;bpy.context.object.data.angle=.12
    bpy.ops.object.camera_add();camera=bpy.context.object;scene.camera=camera
    camera.data.type='ORTHO';camera.data.clip_end=50000
    linear=lambda c:c/12.92 if c<=.04045 else ((c+.055)/1.055)**2.4
    for name in args.only:
        entries = {entry['name'] for entry in json.loads(args.catalog.read_text())}
        if name not in entries:
            raise ValueError('name is absent from selected catalog: '+name)
        source=args.catalog.parent/(name+'.lms')
        vertex,faces=decode(source,args.lod)
        positions=vertex['position'][:,[0,2,1]].astype(float)
        positions[:,1]*=-1
        colors,face_colors=np.unique(vertex['color'][faces[:,0]],axis=0,return_inverse=True)
        data=bpy.data.meshes.new(name);data.from_pydata(positions.tolist(),[],faces.tolist());data.update()
        obj=bpy.data.objects.new(name,data);scene.collection.objects.link(obj)
        for color in colors:
            mat=bpy.data.materials.new(str(color.tolist()));mat.use_nodes=True
            bs=mat.node_tree.nodes['Principled BSDF']
            bs.inputs['Base Color'].default_value=tuple(linear(float(c)/255) for c in color[:3])+(1,)
            bs.inputs['Roughness'].default_value=.63
            bs.inputs['Metallic'].default_value=.20 if color[3]==16 else 0
            data.materials.append(mat)
        for polygon,index in zip(data.polygons,face_colors):polygon.material_index=int(index)
        low=positions.min(0);high=positions.max(0);span=float(max(high-low))
        center=(low+high)/2
        target=Vector(center)
        bpy.ops.mesh.primitive_plane_add(size=span*8,location=(center[0],center[1],low[2]-.3))
        floor=bpy.context.object
        mat=bpy.data.materials.new('neutral-ground');mat.diffuse_color=(.24,.27,.23,1);floor.data.materials.append(mat)
        for view in args.views:
            az,el=map(float,view.split(','));a,e=math.radians(az),math.radians(el)
            camera.location=target+Vector((math.sin(a)*math.cos(e),-math.cos(a)*math.cos(e),math.sin(e)))*span*2.7
            camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler()
            camera.data.ortho_scale=span*1.4
            scene.render.filepath=str(args.output/f'{name}-lod{args.lod}-{int(az)}-{int(el)}.png')
            bpy.ops.render.render(write_still=True)
        bpy.data.objects.remove(obj,do_unlink=True);bpy.data.objects.remove(floor,do_unlink=True)
        print(json.dumps({'name':name,'lod':args.lod,'triangles':len(faces),'colors':len(colors)}),flush=True)


if __name__=='__main__':main()
