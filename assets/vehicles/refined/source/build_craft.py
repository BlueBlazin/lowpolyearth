"""Build the two textured craft from preserved CC0 meshes with Blender.

Run: blender --background --factory-startup --python assets/vehicles/refined/source/build_craft.py
The original OBJ UVs and albedo images remain authoritative. All runtime geometry
is evaluated here, including small edge bevels; Blender is not a runtime dependency.
"""

import json
import math
import os
from pathlib import Path
import shutil
import sys

sys.dont_write_bytecode = True
import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from export_vehicle import export_scene

SOURCES = ROOT / "source" / "quaternius"
URLS = [
    "https://quaternius.com/packs/ultimatespaceships.html",
    "https://opengameart.org/content/lowpoly-spaceships-pack",
]
CRAFT = [
    dict(name="kestrel", model="Bob", texture="Bob_Blue.png", span=3.6,
         layer=1, bevel=.006, canopy_vertices=69, engine_vertices=(72,),
         paint=(255, 250, 238), glass=(219, 231, 246),
         ports=[(1.075, -.050, -1.759, .47), (-1.075, -.050, -1.759, .47)]),
    dict(name="phase_manta", model="Executioner", texture="Executioner_Blue.png", span=8.0,
         layer=2, bevel=.012, canopy_vertices=102, engine_vertices=(224, 288),
         paint=(243, 249, 255), glass=(213, 234, 255),
         ports=[(4.3927, .1545, -3.070, .35), (-4.3927, .1545, -3.070, .35),
                (.9450, -.1562, -2.899, .35), (-.9450, -.1562, -2.899, .35)]),
]


def engine_vector(vector):
    return Vector((vector.x, vector.z, -vector.y))


def components(mesh):
    parents = list(range(len(mesh.vertices)))
    def find(index):
        while parents[index] != index:
            parents[index] = parents[parents[index]]
            index = parents[index]
        return index
    for polygon in mesh.polygons:
        for index in polygon.vertices[1:]:
            parents[find(index)] = find(polygon.vertices[0])
    result = {}
    for vertex in mesh.vertices:
        result.setdefault(find(vertex.index), []).append(vertex.index)
    return result


def material(name, identifier, texture, config, tint, roughness, metallic):
    value = bpy.data.materials.new(name)
    value.use_nodes = True
    value["vearth_material_id"] = identifier
    value["vearth_color_srgb"] = tint
    value["vearth_texture_layer"] = config["layer"]
    value["vearth_texture_path"] = str(texture)
    node = value.node_tree.nodes.get("Principled BSDF")
    node.inputs["Roughness"].default_value = roughness
    node.inputs["Metallic"].default_value = metallic
    image_node = value.node_tree.nodes.new("ShaderNodeTexImage")
    image_node.image = bpy.data.images.load(str(texture), check_existing=True)
    value.node_tree.links.new(image_node.outputs["Color"], node.inputs["Base Color"])
    return value


def make_links_relative(blend_path):
    for image in bpy.data.images:
        if image.source == "FILE" and image.filepath:
            image.filepath = "//" + os.path.relpath(bpy.path.abspath(image.filepath), blend_path.parent)
    for value in bpy.data.materials:
        if "vearth_texture_path" in value:
            value["vearth_texture_path"] = "//" + os.path.relpath(
                bpy.path.abspath(value["vearth_texture_path"]), blend_path.parent)


def build(config):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    directory = SOURCES / config["model"]
    obj_path = directory / (config["model"] + ".obj")
    mtl_path = directory / (config["model"] + ".mtl")
    source_texture = directory / config["texture"]
    texture_path = ROOT / "textures" / (config["name"] + ".png")
    texture_path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source_texture, texture_path)
    bpy.ops.wm.obj_import(filepath=str(obj_path), forward_axis="NEGATIVE_Z", up_axis="Y")
    objects = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
    if len(objects) != 1:
        raise ValueError("The preserved craft must import as one mesh")
    obj = objects[0]
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    mesh = obj.data
    points = [obj.matrix_world @ vertex.co for vertex in mesh.vertices]
    low = Vector(tuple(min(point[axis] for point in points) for axis in range(3)))
    high = Vector(tuple(max(point[axis] for point in points) for axis in range(3)))
    center = (low + high) * .5
    scale = config["span"] / max(high - low)
    for vertex in mesh.vertices:
        vertex.co = (obj.matrix_world @ vertex.co - center) * scale
    obj.matrix_world.identity()
    obj.name = config["name"]
    obj["vearth_part"] = "body"
    mesh.materials.clear()
    for value in [
        material("Painted airframe", 96, texture_path, config, config["paint"], .38, .05),
        material("Titanium nacelles", 97, texture_path, config, (255, 255, 255), .28, .72),
        material("Cockpit glass", 99, texture_path, config, config["glass"], .12, .10),
    ]:
        mesh.materials.append(value)
    islands = components(mesh)
    island_for_vertex = {index: indices for indices in islands.values() for index in indices}
    canopy = [indices for indices in islands.values() if len(indices) == config["canopy_vertices"]]
    if len(canopy) != 1:
        raise ValueError("The authored canopy topology has changed")
    canopy = set(canopy[0])
    image = bpy.data.images.load(str(source_texture), check_existing=True)
    pixels = list(image.pixels)
    width, height = image.size
    uv = mesh.uv_layers.active.data
    def sample(loop_indices):
        point = sum((uv[index].uv for index in loop_indices), Vector((0, 0))) / len(loop_indices)
        x = min(width - 1, max(0, int(point.x * width)))
        y = min(height - 1, max(0, int(point.y * height)))
        start = (y * width + x) * 4
        return pixels[start:start + 3]
    counts = [0, 0, 0]
    for polygon in mesh.polygons:
        count = len(island_for_vertex[polygon.vertices[0]])
        slot = 1 if count in config["engine_vertices"] else 0
        if polygon.vertices[0] in canopy:
            color = sample(polygon.loop_indices)
            if max(color) < .30:
                slot = 2
        polygon.material_index = slot
        polygon.use_smooth = True
        counts[slot] += 1
    if counts[2] < 4:
        raise ValueError("The authored dark canopy was not classified")
    mesh.normals_split_custom_set([(0.0, 0.0, 0.0)] * len(mesh.loops))
    mesh.update()
    mesh.set_sharp_from_angle(angle=math.radians(38))
    bevel = obj.modifiers.new("Machined edge radius", "BEVEL")
    bevel.width = config["bevel"]
    bevel.segments = 2
    bevel.limit_method = "ANGLE"
    bevel.angle_limit = math.radians(38)
    bevel.harden_normals = True
    bevel.use_clamp_overlap = True
    normals = obj.modifiers.new("Weighted panel normals", "WEIGHTED_NORMAL")
    normals.keep_sharp = True
    normals.weight = 50
    # Small bevel junctions can produce a smooth normal outside a corner's
    # geometric hemisphere. Bake evaluated triangles and flatten only those
    # corners; the compressed runtime normal keeps a safe hemisphere margin.
    dependencies = bpy.context.evaluated_depsgraph_get()
    evaluated = obj.evaluated_get(dependencies)
    cooked = evaluated.to_mesh(preserve_all_data_layers=True, depsgraph=dependencies)
    cooked.calc_loop_triangles()
    points, triangles, corner_uvs, corner_normals, slots = [], [], [], [], []
    for triangle in cooked.loop_triangles:
        positions = [cooked.vertices[index].co.copy() for index in triangle.vertices]
        cross = (positions[1] - positions[0]).cross(positions[2] - positions[0])
        if cross.length_squared < 1.0e-16:
            continue
        geometric = cross.normalized()
        start = len(points)
        points.extend(positions)
        triangles.append((start, start + 1, start + 2))
        slots.append(cooked.polygons[triangle.polygon_index].material_index)
        for loop in triangle.loops:
            normal = cooked.corner_normals[loop].vector.copy().normalized()
            corner_normals.append(normal if normal.dot(geometric) >= .03 else geometric)
            corner_uvs.append(cooked.uv_layers.active.data[loop].uv.copy())
    prepared = bpy.data.meshes.new(config["name"] + " evaluated")
    prepared.from_pydata(points, [], triangles)
    layer = prepared.uv_layers.new(name="UVMap")
    for index, point in enumerate(corner_uvs):
        layer.data[index].uv = point
    for value in mesh.materials:
        prepared.materials.append(value)
    for polygon, slot in zip(prepared.polygons, slots):
        polygon.material_index = slot
        polygon.use_smooth = True
    prepared.normals_split_custom_set(corner_normals)
    evaluated.to_mesh_clear()
    obj.modifiers.clear()
    obj.data = prepared
    ports = []
    core = bpy.data.materials.new("Recessed exhaust core")
    core.use_nodes = True
    core["vearth_material_id"] = 100
    core["vearth_color_srgb"] = (105, 201, 238)
    core.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value = (.09, .50, .76, 1)
    core.node_tree.nodes.get("Principled BSDF").inputs["Emission Color"].default_value = (.12, .62, 1, 1)
    core.node_tree.nodes.get("Principled BSDF").inputs["Emission Strength"].default_value = 2
    for number, (x, y, z, radius) in enumerate(config["ports"]):
        position = (Vector((x, -z, y)) - center) * scale
        bpy.ops.mesh.primitive_circle_add(vertices=24, radius=radius * scale,
                                         fill_type="NGON", location=position,
                                         rotation=(-math.pi / 2, 0, 0))
        nozzle = bpy.context.object
        nozzle.name = f"exhaust_core_{number}"
        nozzle["vearth_part"] = "body"
        nozzle.data.materials.append(core)
        ports.append({"position": list(engine_vector(position)),
                      "direction": [0, 0, -1], "radius": radius * scale})
    bpy.context.scene.unit_settings.system = "METRIC"
    bpy.context.scene.unit_settings.scale_length = 1.0
    blend_path = ROOT / "source" / (config["name"] + ".blend")
    bpy.context.preferences.filepaths.save_version = 0
    make_links_relative(blend_path)
    bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
    export_scene(
        ROOT / (config["name"] + ".mesh"),
        source_paths=[Path(__file__), obj_path, mtl_path, source_texture,
                      SOURCES / "License.txt", SOURCES / "provenance.json", blend_path],
        author="Quaternius; Vearth material and mesh preparation",
        license_name="CC0-1.0", source_urls=URLS,
    )
    result = {"name": config["name"], "source_model": config["model"],
              "source_center_blender": list(center), "uniform_scale": scale,
              "material_faces": dict(zip(("paint", "metal", "glass"), counts)), "nozzles": ports}
    print("CRAFT_METADATA " + json.dumps(result))


if __name__ == "__main__":
    for configuration in CRAFT:
        build(configuration)
