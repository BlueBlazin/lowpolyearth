"""Editable metric Blender inspection of generator geometry.

Run with Blender, not the system Python::

    blender -b --factory-startup --python tools/skyscraper_catalog/crown_blender.py \
      -- --input geometry.json --output output/skyscrapers/crown-accurate

JSON schema: ``vertices`` are engine [x,y,z] metres (Y up); ``faces`` are
index lists; ``materials`` contains {name, engine_id, color_rgb:[0..255]};
``face_materials`` contains a material index per face. Optional ``face_tags``
names editable objects, and ``heading_deg`` places model +Z clockwise from
north. Blender axes are +X east, -Y north, +Z up, preserving source winding.
The plain-Python export_mesh helper accepts the catalog's Mesh.triangles.
Materials are Blender inspection proxies, not the engine's facade atlas.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import sys


def export_mesh(mesh, path, face_tags=None, heading_deg=0):
    """Export the actual generator mesh without changing its triangles."""
    vertices, faces, materials, indices = [], [], [], []
    lookup = {}
    for points, color, engine_id in mesh.triangles:
        key = (int(engine_id), tuple(int(c) for c in color[:3]))
        if key not in lookup:
            lookup[key] = len(materials)
            materials.append(dict(name=f'material-{key[0]}-{len(materials)}',
                                  engine_id=key[0], color_rgb=list(key[1])))
        faces.append(list(range(len(vertices), len(vertices)+3)))
        vertices.extend([[float(c) for c in point] for point in points])
        indices.append(lookup[key])
    data = dict(vertices=vertices, faces=faces, materials=materials,
                face_materials=indices, heading_deg=float(heading_deg))
    if face_tags is not None:
        if len(face_tags) != len(faces):
            raise ValueError('face_tags must have one tag per generator triangle')
        data['face_tags'] = list(face_tags)
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, separators=(',', ':'), allow_nan=False)+'\n')


def validate(data):
    vertices, faces = data['vertices'], data['faces']
    if not vertices or not faces:
        raise ValueError('geometry must contain vertices and faces')
    if any(len(v) != 3 or not all(math.isfinite(c) for c in v) for v in vertices):
        raise ValueError('vertices must be finite metre triples')
    if any(len(f) < 3 or any(type(i) is not int or not 0 <= i < len(vertices)
                             for i in f) for f in faces):
        raise ValueError('face indices are out of range')
    materials = data.get('materials', [dict(name='wall', engine_id=87,
                                           color_rgb=[157, 163, 164])])
    indices = data.get('face_materials', [0]*len(faces))
    tags = data.get('face_tags', ['Crown geometry']*len(faces))
    if len(indices) != len(faces) or len(tags) != len(faces):
        raise ValueError('face_materials and face_tags must match faces')
    if any(type(i) is not int or not 0 <= i < len(materials) for i in indices):
        raise ValueError('material index out of range')
    for material in materials:
        rgb = material.get('color_rgb', [157, 163, 164])
        if len(rgb) != 3 or any(not math.isfinite(c) or not 0 <= c <= 255 for c in rgb):
            raise ValueError('material color_rgb must contain three values in 0..255')
    if not math.isfinite(data.get('heading_deg', 0)):
        raise ValueError('heading_deg must be finite')
    return vertices, faces, materials, indices, tags


def main():
    import bpy
    from mathutils import Vector

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path,
                        default=Path('output/skyscrapers/crown-accurate'))
    parser.add_argument('--name', default='crown-sydney')
    parser.add_argument('--resolution', type=int, default=1400,
                        help='Image height; width is 3/4 of height')
    parser.add_argument('--samples', type=int, default=48)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.resolution < 256 or args.samples < 1:
        parser.error('resolution must be >=256 and samples >=1')
    args.output.mkdir(parents=True, exist_ok=True)
    raw = args.input.read_bytes()
    data = json.loads(raw)
    vertices, faces, materials, indices, tags = validate(data)
    a = math.radians(data.get('heading_deg', 0))
    c, s = math.cos(a), math.sin(a)
    points = [Vector((x*c+z*s, x*s-z*c, y)) for x, y, z in vertices]
    low = Vector(tuple(min(p[i] for p in points) for i in range(3)))
    high = Vector(tuple(max(p[i] for p in points) for i in range(3)))
    span = max(high-low)
    if span <= 0:
        raise ValueError('geometry must have nonzero dimensions')
    center = (low+high)/2

    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    scene = bpy.context.scene
    scene.unit_settings.system = 'METRIC'
    scene.unit_settings.scale_length = 1
    scene.unit_settings.length_unit = 'METERS'
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = args.samples
    scene.cycles.use_denoising = True
    scene.render.resolution_x = round(args.resolution*.75)
    scene.render.resolution_y = args.resolution
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'
    scene.view_settings.view_transform = 'AgX'
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes['Background']
    background.inputs['Color'].default_value = (.42, .42, .42, 1)
    background.inputs['Strength'].default_value = .8

    def material(name, rgb, roughness=.6, metallic=0):
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        bsdf = mat.node_tree.nodes.get('Principled BSDF')
        linear = lambda v: v/12.92 if v <= .04045 else ((v+.055)/1.055)**2.4
        color = tuple(linear(v/255) for v in rgb)+(1,)
        bsdf.inputs['Base Color'].default_value = color
        bsdf.inputs['Roughness'].default_value = roughness
        bsdf.inputs['Metallic'].default_value = metallic
        mat.diffuse_color = color
        return mat

    slots = []
    for spec in materials:
        identity = spec.get('engine_id', 87)
        glass = identity == 15 or 103 <= identity <= 118
        metal = identity in (16, 88, 95)
        mat = material(spec.get('name', f'Engine {identity}'),
                       spec.get('color_rgb', [157, 163, 164]),
                       spec.get('roughness', .24 if glass else .32 if metal else .65),
                       spec.get('metallic', .28 if glass else .72 if metal else 0))
        mat['engine_material_id'] = identity
        mat['inspection_proxy'] = True
        slots.append(mat)
    collection = bpy.data.collections.new('Generator geometry — metres')
    scene.collection.children.link(collection)
    groups = {}
    for i, tag in enumerate(tags):
        groups.setdefault(str(tag), []).append(i)
    for tag, group in groups.items():
        used = sorted({i for f in group for i in faces[f]})
        remap = {v: i for i, v in enumerate(used)}
        mesh = bpy.data.meshes.new(tag)
        mesh.from_pydata([points[i] for i in used], [],
                         [[remap[i] for i in faces[f]] for f in group])
        mesh.update()
        obj = bpy.data.objects.new(tag, mesh)
        collection.objects.link(obj)
        for mat in slots:
            mesh.materials.append(mat)
        for polygon, f in zip(mesh.polygons, group):
            polygon.material_index = indices[f]
        obj['source_geometry_sha256'] = hashlib.sha256(raw).hexdigest()

    bpy.ops.mesh.primitive_plane_add(size=span*20,
                                    location=(center.x, center.y, low.z-.03))
    floor = bpy.context.object
    floor.name = 'Inspection ground — not exported architecture'
    floor.data.materials.append(material('Neutral ground', [157, 157, 157], .85))
    bpy.ops.object.light_add(type='SUN')
    sun = bpy.context.object
    sun.name = 'Soft inspection sun'
    sun.rotation_euler = (math.radians(28), math.radians(-25), math.radians(-30))
    sun.data.energy = 2
    sun.data.angle = math.radians(15)
    guide_material = material('Dimension ink', [45, 45, 45], 1)
    annotations = []
    cameras = []
    aspect = scene.render.resolution_x/scene.render.resolution_y

    def guide_line(name, coords):
        curve = bpy.data.curves.new(name, 'CURVE')
        curve.dimensions = '3D'
        curve.bevel_depth = span*.00032
        spline = curve.splines.new('POLY')
        spline.points.add(len(coords)-1)
        for point, co in zip(spline.points, coords):
            point.co = (*co, 1)
        obj = bpy.data.objects.new(name, curve)
        scene.collection.objects.link(obj)
        curve.materials.append(guide_material)
        return obj

    for label, azimuth, elevation in [('north', 0, 0), ('east', 90, 0),
                                      ('south', 180, 0), ('west', 270, 0),
                                      ('perspective', 135, 20)]:
        az, el = math.radians(azimuth), math.radians(elevation)
        direction = Vector((math.sin(az)*math.cos(el),
                            -math.cos(az)*math.cos(el), math.sin(el)))
        bpy.ops.object.camera_add(location=center+direction*span*3)
        camera = bpy.context.object
        camera.name = f'Inspection {label}'
        camera.rotation_euler = (center-camera.location).to_track_quat('-Z', 'Y').to_euler()
        rotation = camera.rotation_euler.to_quaternion()
        right, up = rotation @ Vector((1, 0, 0)), rotation @ Vector((0, 1, 0))
        horizontal = [(p-center).dot(right) for p in points]
        vertical = [(p-center).dot(up) for p in points]
        width, height = max(horizontal)-min(horizontal), max(vertical)-min(vertical)
        fit = max(height, width/aspect)*1.24
        camera.data.clip_start = .01
        camera.data.clip_end = span*30
        if label == 'perspective':
            camera.data.type = 'PERSP'
            camera.data.sensor_fit = 'VERTICAL'
            camera.data.sensor_height = 36
            camera.data.lens = 55
            # Account for the near side's larger perspective projection.
            depth = max((p-center).dot(direction) for p in points)
            camera.location = center+direction*(depth+fit/(2*math.tan(camera.data.angle_y/2)))
        else:
            camera.data.type = 'ORTHO'
            camera.data.ortho_scale = fit
        view_guides = []
        if label != 'perspective':
            x = min(horizontal)-fit*.045
            front = center+direction*span
            bottom = front+right*x+up*min(vertical)
            top = front+right*x+up*max(vertical)
            tick = right*fit*.008
            view_guides.append(guide_line(f'{label} envelope height', [bottom, top]))
            for point in (bottom, top):
                view_guides.append(guide_line(f'{label} dimension tick', [point-tick, point+tick]))
            text_data = bpy.data.curves.new(f'{label} height text', 'FONT')
            text_data.body = f'{high.z-low.z:.2f} m'
            text_data.size = fit*.018
            text_data.align_x = 'CENTER'
            text_data.materials.append(guide_material)
            text_obj = bpy.data.objects.new(text_data.name, text_data)
            scene.collection.objects.link(text_obj)
            text_obj.location = (bottom+top)/2-right*fit*.018
            text_obj.rotation_euler = camera.rotation_euler.copy()
            text_obj.rotation_euler.rotate_axis('Z', math.pi/2)
            view_guides.append(text_obj)
        annotations.extend(view_guides)
        cameras.append((label, camera, view_guides))

    results = []
    for label, camera, guides in cameras:
        for obj in annotations:
            obj.hide_render = obj not in guides
        scene.camera = camera
        path = args.output/f'{args.name}-{label}.png'
        scene.render.filepath = str(path.resolve())
        bpy.ops.render.render(write_still=True)
        results.append(dict(view=label, image=str(path.resolve()),
                            sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    for obj in annotations:
        obj.hide_render = True
        obj.hide_set(True)
    scene.camera = cameras[-1][1]
    scene['source_geometry_sha256'] = hashlib.sha256(raw).hexdigest()
    scene['source_geometry_path'] = str(args.input.resolve())
    scene['coordinate_convention'] = '+X east; -Y north; +Z up; units metres'
    blend = args.output/f'{args.name}.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(blend.resolve()))
    record = dict(input_sha256=hashlib.sha256(raw).hexdigest(),
                  blend=str(blend.resolve()), blend_sha256=hashlib.sha256(blend.read_bytes()).hexdigest(),
                  vertices=len(vertices), faces=len(faces), objects=list(groups),
                  bounds_blender_m=[list(low), list(high)], units='metres',
                  dimensions_note='Envelope height includes authored foundation below datum.',
                  materials_note='Opaque Blender glass/wall proxies; engine atlas is not reproduced.',
                  renders=results)
    (args.output/f'{args.name}-inspection.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps(record), flush=True)


if __name__ == '__main__':
    main()
