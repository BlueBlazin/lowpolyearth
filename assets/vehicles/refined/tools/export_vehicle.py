"""Export evaluated Blender geometry to the VEARTHV1 runtime asset format.

Run through Blender's background Python, or import ``export_scene`` in an asset
builder. Objects must name their runtime part with ``vearth_part``. Materials
must set ``vearth_material_id`` (96..102); optional ``vearth_color_srgb`` is an
RGB byte triplet. Textured materials set ``vearth_texture_layer`` (1..3) and
``vearth_texture_path`` to the source base-color image. Layer zero is untextured.

The little-endian header is <8sHHI (magic, version, reserved, part count).
Each part starts with <HHII (name bytes, reserved, vertices, indices), its ASCII
name, 40-byte <8f3BBB3x corner records, then uint32 triangle indices. A corner is
position, unit normal, UV, sRGB tint, material and texture layer. Alpha is 255.
UVs retain Blender's bottom-left origin; the renderer owns image-row conversion.
Runtime positions use metres, +Y up, +Z forward. No object transforms remain.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import struct
import sys

import bpy
from mathutils import Matrix, Vector


MAGIC = b"VEARTHV1"
VERSION = 1
VERTEX = struct.Struct("<8f3BBB3x")
MAX_PARTS = 32
MAX_VERTICES = 1_000_000
MAX_INDICES = 3_000_000
BLENDER_TO_ENGINE = Matrix(((1, 0, 0), (0, 0, 1), (0, -1, 0)))


def _sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _path_record(path, output_dir):
    path = Path(path).resolve(strict=True)
    return {
        "path": os.path.relpath(path, output_dir),
        "sha256": _sha256(path),
        "bytes": path.stat().st_size,
    }


def _srgb_byte(linear):
    linear = max(0.0, min(1.0, float(linear)))
    srgb = linear * 12.92 if linear <= 0.0031308 else 1.055 * linear ** (1.0 / 2.4) - 0.055
    return round(srgb * 255.0)


def _material(material, output_dir, textures):
    if material is None or "vearth_material_id" not in material:
        raise ValueError("Every exported polygon needs an explicitly mapped vehicle material")
    material_id = int(material["vearth_material_id"])
    layer = int(material.get("vearth_texture_layer", 0))
    if material_id not in range(96, 103) or layer not in range(4):
        raise ValueError(f"Unsupported material mapping on {material.name!r}")
    if layer:
        if "vearth_texture_path" not in material:
            raise ValueError(f"Textured material {material.name!r} has no source image path")
        texture = Path(bpy.path.abspath(material["vearth_texture_path"])).resolve(strict=True)
        record = _path_record(texture, output_dir)
        record["color_space"] = "sRGB"
        if layer in textures and textures[layer] != record:
            raise ValueError(f"Texture layer {layer} is assigned to different source images")
        textures[layer] = record
    explicit = material.get("vearth_color_srgb")
    if explicit is not None:
        color = tuple(int(value) for value in explicit)
        if len(color) != 3 or any(value < 0 or value > 255 for value in color):
            raise ValueError(f"Invalid sRGB color on {material.name!r}")
    elif layer:
        color = (255, 255, 255)
    else:
        color = tuple(_srgb_byte(value) for value in material.diffuse_color[:3])
    return color, material_id, layer


def _part_name(obj):
    value = obj.get("vearth_part")
    if not isinstance(value, str) or not re.fullmatch(r"[a-z0-9_]{1,63}", value):
        raise ValueError(f"Mesh {obj.name!r} needs a valid vearth_part name")
    return value


def export_scene(
    output_path,
    *,
    source_paths,
    author,
    license_name,
    source_urls=(),
    coordinates="blender",
):
    """Export the current isolated scene, including a verifiable JSON manifest.

    ``source_paths`` must include the authoritative model/generator and license
    files. Provenance text is supplied by the caller after checking the original
    source; this function verifies file contents, not legal ownership.
    """
    if not author.strip() or not license_name.strip() or not source_paths:
        raise ValueError("Author, verified license and source files are required")
    if coordinates not in ("blender", "engine"):
        raise ValueError("Coordinates must be 'blender' or 'engine'")
    output_path = Path(output_path).resolve()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_dir = output_path.parent
    source_records = [_path_record(path, output_dir) for path in source_paths]
    source_records.sort(key=lambda record: record["path"])
    axis = BLENDER_TO_ENGINE if coordinates == "blender" else Matrix.Identity(3)
    scale = bpy.context.scene.unit_settings.scale_length
    if not math.isfinite(scale) or scale <= 0.0:
        raise ValueError("Scene must have a positive finite metre scale")
    axis *= scale
    dependencies = bpy.context.evaluated_depsgraph_get()
    parts = {}
    textures = {}
    material_records = {}
    material_values = {}
    for obj in sorted(bpy.context.scene.objects, key=lambda item: item.name):
        if obj.type != "MESH" or obj.hide_render:
            continue
        name = _part_name(obj)
        part = parts.setdefault(name, {"vertices": [], "lookup": {}, "indices": []})
        evaluated = obj.evaluated_get(dependencies)
        mesh = evaluated.to_mesh(preserve_all_data_layers=True, depsgraph=dependencies)
        try:
            mesh.calc_loop_triangles()
            if hasattr(mesh, "calc_normals_split"):
                mesh.calc_normals_split()
            transform = axis.to_4x4() @ evaluated.matrix_world
            linear = transform.to_3x3()
            if abs(linear.determinant()) < 1.0e-12:
                raise ValueError(f"Mesh {obj.name!r} has a singular transform")
            normal_transform = linear.inverted().transposed()
            reversed_winding = linear.determinant() < 0.0
            uv_data = mesh.uv_layers.active.data if mesh.uv_layers.active else None
            for triangle in mesh.loop_triangles:
                polygon = mesh.polygons[triangle.polygon_index]
                material = mesh.materials[polygon.material_index] if mesh.materials else None
                if material is None:
                    raise ValueError(f"Mesh {obj.name!r} has an unmapped material slot")
                if material.name not in material_values:
                    material_values[material.name] = _material(material, output_dir, textures)
                    color, material_id, layer = material_values[material.name]
                    material_records[material.name] = {
                        "material_id": material_id,
                        "color_srgb": list(color),
                        "texture_layer": layer,
                    }
                color, material_id, layer = material_values[material.name]
                if layer and uv_data is None:
                    raise ValueError(f"Textured mesh {obj.name!r} has no UV map")
                corners = list(triangle.loops)
                if reversed_winding:
                    corners[1], corners[2] = corners[2], corners[1]
                triangle_records = []
                points = []
                normals = []
                for loop_index in corners:
                    loop = mesh.loops[loop_index]
                    position = transform @ mesh.vertices[loop.vertex_index].co
                    if hasattr(mesh, "corner_normals"):
                        source_normal = mesh.corner_normals[loop_index].vector
                    else:
                        source_normal = loop.normal
                    normal = (normal_transform @ source_normal).normalized()
                    uv = tuple(uv_data[loop_index].uv) if uv_data is not None else (0.0, 0.0)
                    if any(not math.isfinite(value) or abs(value) > 256.0 for value in position):
                        raise ValueError(f"Mesh {obj.name!r} is outside finite metre bounds")
                    if any(not math.isfinite(value) or abs(value) > 10_000.0 for value in uv):
                        raise ValueError(f"Mesh {obj.name!r} has invalid UVs")
                    if not all(math.isfinite(value) for value in normal) or abs(normal.length_squared - 1.0) > 0.002:
                        raise ValueError(f"Mesh {obj.name!r} has invalid corner normals")
                    packed = VERTEX.pack(*position, *normal, *uv, *color, material_id, layer)
                    values = VERTEX.unpack(packed)
                    points.append(Vector(values[:3]))
                    normals.append(Vector(values[3:6]))
                    triangle_records.append(packed)
                cross = (points[1] - points[0]).cross(points[2] - points[0])
                if cross.length_squared <= 1.0e-16:
                    raise ValueError(f"Mesh {obj.name!r} contains a degenerate triangle")
                geometric = cross.normalized()
                if any(normal.dot(geometric) <= 0.001 for normal in normals):
                    raise ValueError(f"Mesh {obj.name!r} has normals opposing triangle winding")
                for packed in triangle_records:
                    index = part["lookup"].get(packed)
                    if index is None:
                        index = len(part["vertices"])
                        part["lookup"][packed] = index
                        part["vertices"].append(packed)
                    part["indices"].append(index)
        finally:
            evaluated.to_mesh_clear()
    if not 1 <= len(parts) <= MAX_PARTS:
        raise ValueError("Scene has no exported parts or exceeds the part limit")
    total_vertices = sum(len(part["vertices"]) for part in parts.values())
    total_indices = sum(len(part["indices"]) for part in parts.values())
    if total_vertices > MAX_VERTICES or total_indices > MAX_INDICES:
        raise ValueError("Scene exceeds the vehicle asset geometry limits")
    payload = bytearray(struct.pack("<8sHHI", MAGIC, VERSION, 0, len(parts)))
    part_records = []
    for name, part in sorted(parts.items()):
        vertices = part["vertices"]
        indices = part["indices"]
        if len(vertices) < 3 or len(indices) < 3:
            raise ValueError(f"Part {name!r} has no triangles")
        name_bytes = name.encode("ascii")
        payload.extend(struct.pack("<HHII", len(name_bytes), 0, len(vertices), len(indices)))
        payload.extend(name_bytes)
        payload.extend(b"".join(vertices))
        payload.extend(struct.pack(f"<{len(indices)}I", *indices))
        positions = [VERTEX.unpack(vertex)[:3] for vertex in vertices]
        part_records.append({
            "name": name,
            "vertices": len(vertices),
            "triangles": len(indices) // 3,
            "bounds_m": {
                "min": [min(point[axis] for point in positions) for axis in range(3)],
                "max": [max(point[axis] for point in positions) for axis in range(3)],
            },
        })
    output_path.write_bytes(payload)
    manifest = {
        "format": "VEARTHV1",
        "version": VERSION,
        "coordinates": {"units": "metres", "up": "+Y", "forward": "+Z"},
        "uv_origin": "bottom-left",
        "author": author,
        "license": license_name,
        "source_urls": list(source_urls),
        "sources": source_records,
        "exporter": _path_record(__file__, output_dir),
        "blender_version": bpy.app.version_string,
        "mesh": _path_record(output_path, output_dir),
        "parts": part_records,
        "materials": dict(sorted(material_records.items())),
        "textures": {str(layer): record for layer, record in sorted(textures.items())},
    }
    manifest_path = output_path.with_suffix(".json")
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"Exported {output_path}: {total_vertices} corners, {total_indices // 3} triangles")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--source", action="append", required=True)
    parser.add_argument("--author", required=True)
    parser.add_argument("--license", required=True)
    parser.add_argument("--source-url", action="append", default=[])
    parser.add_argument("--coordinates", choices=("blender", "engine"), default="blender")
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    export_scene(
        args.output,
        source_paths=args.source,
        author=args.author,
        license_name=args.license,
        source_urls=args.source_url,
        coordinates=args.coordinates,
    )


if __name__ == "__main__":
    main()
