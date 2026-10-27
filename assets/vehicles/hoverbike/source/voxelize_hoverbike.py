"""Voxelize the editable Sparrow Blender model for Low Poly Earth's staging workspace.

Run with:
  blender --background assets/vehicles/hoverbike/sparrow_hoverbike.blend \
    --python assets/vehicles/hoverbike/source/voxelize_hoverbike.py

The compact SVX2 output is stage-only. It stores one material palette, sparse
signed voxel coordinates, and body/left-rotor/right-rotor group ids in
Low Poly Earth's X-forward, Y-up, Z-side frame.
"""

import bpy
import math
import os
import struct
from mathutils import Vector
from mathutils.bvhtree import BVHTree


ROOT = os.getcwd()
OUTPUT = os.path.join(
    ROOT, "crates", "vearth-app", "assets", "stage", "sparrow_hoverbike.svx"
)
PITCH_M = 0.04
COLLECTION = "SPARROW_HOVERBIKE"

# Runtime material id followed by renderer-calibrated RGB tint.
PALETTE = [
    (1, (202, 43, 48)),    # scarlet ceramic
    (1, (151, 26, 34)),    # crimson panels and fan ducts
    (1, (151, 161, 165)),  # titanium structure
    (1, (52, 57, 62)),     # technical-weave saddle/grips
    (1, (43, 49, 54)),     # fan interiors
    (1, (42, 214, 242)),   # cyan field/navigation
    (1, (246, 91, 24)),    # amber navigation
]
MATERIAL_INDEX = {
    "Body · scarlet ceramic": 0,
    "Panel · crimson": 1,
    "Structure · titanium": 2,
    "Rotor · titanium": 2,
    "Seat · technical weave": 3,
    "Thruster interior": 4,
    "Vector field · cyan": 5,
    "Navigation · amber": 6,
}


def evaluated_surfaces():
    depsgraph = bpy.context.evaluated_depsgraph_get()
    collection = bpy.data.collections.get(COLLECTION)
    if collection is None:
        raise RuntimeError(f"missing Blender collection {COLLECTION!r}")

    surfaces = []
    all_vertices = []
    for source in collection.objects:
        if source.type not in {"MESH", "CURVE"}:
            continue
        evaluated = source.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh(preserve_all_data_layers=False, depsgraph=depsgraph)
        try:
            world_vertices = [
                evaluated.matrix_world @ vertex.co for vertex in mesh.vertices
            ]
            polygons = [tuple(polygon.vertices) for polygon in mesh.polygons]
            if not world_vertices or not polygons:
                continue
            tree = BVHTree.FromPolygons(world_vertices, polygons, all_triangles=False)
            material_name = source.active_material.name if source.active_material else ""
            palette_index = MATERIAL_INDEX.get(material_name, 1)
            group = (
                1
                if source.name == "HBK_Downward_Rotor_L"
                else 2
                if source.name == "HBK_Downward_Rotor_R"
                else 0
            )
            surfaces.append((tree, palette_index, group, source.name))
            all_vertices.extend(world_vertices)
        finally:
            evaluated.to_mesh_clear()
    if not surfaces:
        raise RuntimeError("hoverbike collection produced no evaluable surfaces")
    return surfaces, all_vertices


def voxelize():
    surfaces, vertices = evaluated_surfaces()
    low = Vector(
        (
            min(vertex.x for vertex in vertices),
            min(vertex.y for vertex in vertices),
            min(vertex.z for vertex in vertices),
        )
    )
    high = Vector(
        (
            max(vertex.x for vertex in vertices),
            max(vertex.y for vertex in vertices),
            max(vertex.z for vertex in vertices),
        )
    )
    minimum = [math.floor(low[axis] / PITCH_M) - 1 for axis in range(3)]
    maximum = [math.ceil(high[axis] / PITCH_M) + 1 for axis in range(3)]
    capture_distance = PITCH_M * 0.86
    occupied = {}

    for ix in range(minimum[0], maximum[0] + 1):
        for iy in range(minimum[1], maximum[1] + 1):
            for iz in range(minimum[2], maximum[2] + 1):
                point = Vector((ix * PITCH_M, iy * PITCH_M, iz * PITCH_M))
                nearest_distance = math.inf
                nearest_palette = None
                nearest_group = 0
                for tree, palette_index, group, name in surfaces:
                    surface_distance = (
                        PITCH_M * 0.62 if "Downward_" in name else capture_distance
                    )
                    nearest = tree.find_nearest(point, surface_distance)
                    if nearest is None or nearest[0] is None or nearest[3] is None:
                        continue
                    distance = nearest[3]
                    if distance < nearest_distance:
                        nearest_distance = distance
                        nearest_palette = palette_index
                        nearest_group = group
                if nearest_palette is None:
                    continue
                # Blender: X forward, Y lateral, Z up.
                # Low Poly Earth stage: X forward, Y up, Z lateral.
                occupied[(ix, iz, -iy)] = (nearest_palette, nearest_group)

    if not occupied:
        raise RuntimeError("voxelizer produced no occupied cells")
    return occupied


def write_asset(occupied):
    os.makedirs(os.path.dirname(OUTPUT), exist_ok=True)
    with open(OUTPUT, "wb") as output:
        output.write(b"SVX2")
        output.write(struct.pack("<HBBfI", 2, len(PALETTE), 0, PITCH_M, len(occupied)))
        for material, tint in PALETTE:
            output.write(struct.pack("<BBBB", material, *tint))
        for (x, y, z), (palette_index, group) in sorted(occupied.items()):
            if not all(-32768 <= value <= 32767 for value in (x, y, z)):
                raise ValueError("voxel coordinate exceeds SVX2 i16 range")
            output.write(struct.pack("<hhhBB", x, y, z, palette_index, group))


occupied = voxelize()
write_asset(occupied)
print(
    f"SPARROW_VOXEL_OK pitch={PITCH_M:.3f} occupied={len(occupied)} "
    f"bytes={os.path.getsize(OUTPUT)} output={OUTPUT}"
)
