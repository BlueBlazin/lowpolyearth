"""Build the Blender source for the Low Poly Earth Sparrow hoverbike.

Run with:
  blender --background --python assets/vehicles/hoverbike/source/build_hoverbike.py

The asset is authored in metres, points along +X, and uses Z-up.
"""

import bpy
import math
import os
from mathutils import Vector


ROOT = os.path.abspath(
    os.path.join(os.path.dirname(bpy.data.filepath or __file__), "..", "..", "..", "..")
)
if not os.path.isdir(os.path.join(ROOT, "assets")):
    ROOT = os.getcwd()
ASSET_DIR = os.path.join(ROOT, "assets", "vehicles", "hoverbike")
MODEL_SCALE = 0.80


def clear_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)


clear_scene()
scene = bpy.context.scene
model_collection = bpy.data.collections.new("SPARROW_HOVERBIKE")
scene.collection.children.link(model_collection)


def material(name, base, metallic=0.0, roughness=0.4, emission=None, strength=0.0):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*base, 1.0)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*base, 1.0)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    if emission is not None:
        bsdf.inputs["Emission Color"].default_value = (*emission, 1.0)
        bsdf.inputs["Emission Strength"].default_value = strength
    return mat


MAT_BODY = material("Body · scarlet ceramic", (0.42, 0.012, 0.015), 0.62, 0.24)
MAT_PANEL = material("Panel · crimson", (0.24, 0.008, 0.012), 0.52, 0.29)
MAT_EDGE = material("Structure · titanium", (0.28, 0.31, 0.32), 0.9, 0.2)
MAT_ROTOR = material("Rotor · titanium", (0.46, 0.49, 0.50), 0.82, 0.24)
MAT_SEAT = material("Seat · technical weave", (0.045, 0.050, 0.055), 0.0, 0.70)
MAT_INNER = material("Thruster interior", (0.035, 0.040, 0.045), 0.35, 0.44)
MAT_CYAN = material(
    "Vector field · cyan", (0.01, 0.14, 0.17), 0.2, 0.16, (0.02, 0.78, 1.0), 7.5
)
MAT_AMBER = material(
    "Navigation · amber", (0.12, 0.038, 0.004), 0.1, 0.2, (1.0, 0.20, 0.025), 5.0
)


def mesh_object(name, vertices, faces, mat, collection=model_collection):
    mesh = bpy.data.meshes.new(name + ".Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    if mat is not None:
        obj.data.materials.append(mat)
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    return obj


def finish_surface(obj, subdivision=2, bevel=0.0):
    if subdivision:
        mod = obj.modifiers.new("Subdivision Surface", "SUBSURF")
        mod.subdivision_type = "CATMULL_CLARK"
        mod.levels = subdivision
        mod.render_levels = subdivision
    if bevel:
        mod = obj.modifiers.new("Edge radii", "BEVEL")
        mod.width = bevel
        mod.segments = 3
        mod.limit_method = "ANGLE"
    return obj


def superellipse_point(angle, y_radius, z_radius, power=2.35):
    c, s = math.cos(angle), math.sin(angle)
    exponent = 2.0 / power
    y = math.copysign(abs(c) ** exponent, c) * y_radius
    z = math.copysign(abs(s) ** exponent, s) * z_radius
    return y, z


def loft(name, sections, mat, sides=32, power=2.35, cap=True, subdivision=2):
    """Create a continuous quad loft.

    Each section is (x, y_center, z_center, y_radius, z_radius, twist_radians).
    """
    vertices = []
    for x, yc, zc, yr, zr, twist in sections:
        for side in range(sides):
            angle = math.tau * side / sides + twist
            y, z = superellipse_point(angle, yr, zr, power)
            vertices.append((x, yc + y, zc + z))
    faces = []
    for ring in range(len(sections) - 1):
        a = ring * sides
        b = (ring + 1) * sides
        for side in range(sides):
            nxt = (side + 1) % sides
            faces.append((a + side, a + nxt, b + nxt, b + side))
    if cap:
        faces.append(tuple(reversed(range(sides))))
        end = (len(sections) - 1) * sides
        faces.append(tuple(end + side for side in range(sides)))
    return finish_surface(mesh_object(name, vertices, faces, mat), subdivision)


def curve_pipe(name, points, radius, mat, cyclic=False, resolution=3, collection=model_collection):
    curve = bpy.data.curves.new(name + ".Curve", "CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = resolution
    curve.bevel_depth = radius
    curve.bevel_resolution = 4
    curve.resolution_u = 10
    curve.use_fill_caps = True
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for bezier, point in zip(spline.bezier_points, points):
        bezier.co = point
        bezier.handle_left_type = "AUTO"
        bezier.handle_right_type = "AUTO"
    spline.use_cyclic_u = cyclic
    obj = bpy.data.objects.new(name, curve)
    collection.objects.link(obj)
    obj.data.materials.append(mat)
    return obj


def prism_from_profiles(name, front, back, mat, bevel=0.02):
    """Join matching perimeter profiles into a closed, bevelled hard-surface part."""
    vertices = list(front) + list(back)
    count = len(front)
    faces = [tuple(reversed(range(count))), tuple(range(count, count * 2))]
    for i in range(count):
        nxt = (i + 1) % count
        faces.append((i, nxt, count + nxt, count + i))
    return finish_surface(mesh_object(name, vertices, faces, mat), 0, bevel)


def fan_transform(xc, yc, zc, tilt, x, y, z):
    """Transform fan-local coordinates, banking its downward axis outboard."""
    ct, st = math.cos(tilt), math.sin(tilt)
    return (xc + x, yc + y * ct - z * st, zc + y * st + z * ct)


def annular_duct(name, xc, yc, zc, tilt):
    """Build a circular, banked airfoil-section duct."""
    sides = 36
    # Local z, outer radius, inner radius.
    levels = [
        (-0.035, 0.370, 0.340),
        (-0.010, 0.406, 0.374),
        (0.035, 0.399, 0.367),
        (0.060, 0.360, 0.332),
    ]
    vertices = []
    for z, outer_radius, inner_radius in levels:
        for i in range(sides):
            angle = math.tau * i / sides
            vertices.append(
                fan_transform(
                    xc,
                    yc,
                    zc,
                    tilt,
                    math.cos(angle) * outer_radius,
                    math.sin(angle) * outer_radius,
                    z,
                )
            )
        for i in range(sides):
            angle = math.tau * i / sides
            vertices.append(
                fan_transform(
                    xc,
                    yc,
                    zc,
                    tilt,
                    math.cos(angle) * inner_radius,
                    math.sin(angle) * inner_radius,
                    z,
                )
            )
    faces = []
    stride = sides * 2
    for level in range(len(levels) - 1):
        a, b = level * stride, (level + 1) * stride
        for i in range(sides):
            nxt = (i + 1) % sides
            faces.append((a + i, a + nxt, b + nxt, b + i))
            faces.append(
                (
                    a + sides + nxt,
                    a + sides + i,
                    b + sides + i,
                    b + sides + nxt,
                )
            )
    for level in (0, len(levels) - 1):
        base = level * stride
        for i in range(sides):
            nxt = (i + 1) % sides
            if level == 0:
                faces.append((base + i, base + sides + i, base + sides + nxt, base + nxt))
            else:
                faces.append((base + nxt, base + sides + nxt, base + sides + i, base + i))
    return finish_surface(mesh_object(name, vertices, faces, MAT_PANEL), subdivision=1)


def animated_rotor(name, xc, yc, zc, tilt, phase, spin_direction):
    """Build one coherent, animation-ready five-blade rotor and hub."""
    vertices = []
    faces = []
    stations = [
        # radius, chord, backward sweep, aerodynamic pitch, thickness
        (0.068, 0.105, math.radians(0.0), 0.020, 0.012),
        (0.145, 0.135, math.radians(4.0), 0.017, 0.010),
        (0.245, 0.112, math.radians(9.0), 0.012, 0.008),
        (0.325, 0.058, math.radians(15.0), 0.006, 0.006),
    ]
    rotor_z = 0.012

    for blade_index in range(5):
        blade_start = len(vertices)
        root_angle = math.tau * blade_index / 5
        for radius, chord, sweep, pitch, thickness in stations:
            angle = root_angle - sweep
            ca, sa = math.cos(angle), math.sin(angle)
            center = Vector((ca * radius, sa * radius))
            tangent = Vector((-sa, ca))
            leading = center - tangent * chord * 0.5
            trailing = center + tangent * chord * 0.5
            vertices.extend(
                [
                    (leading.x, leading.y, rotor_z + pitch * 0.5 + thickness),
                    (trailing.x, trailing.y, rotor_z - pitch * 0.5 + thickness),
                    (leading.x, leading.y, rotor_z + pitch * 0.5 - thickness),
                    (trailing.x, trailing.y, rotor_z - pitch * 0.5 - thickness),
                ]
            )
        for station_index in range(len(stations) - 1):
            a = blade_start + station_index * 4
            b = a + 4
            faces.extend(
                [
                    (a, b, b + 1, a + 1),
                    (a + 3, b + 3, b + 2, a + 2),
                    (a + 2, b + 2, b, a),
                    (a + 1, b + 1, b + 3, a + 3),
                ]
            )
        root = blade_start
        tip = blade_start + (len(stations) - 1) * 4
        faces.extend(
            [
                (root + 2, root, root + 1, root + 3),
                (tip, tip + 2, tip + 3, tip + 1),
            ]
        )

    # A domed hub is part of the same rotating object.
    hub_start = len(vertices)
    hub_sides = 28
    hub_rings = [(-0.018, 0.055), (0.012, 0.083), (0.044, 0.058)]
    for z, radius in hub_rings:
        for i in range(hub_sides):
            angle = math.tau * i / hub_sides
            vertices.append((math.cos(angle) * radius, math.sin(angle) * radius, z))
    for ring in range(len(hub_rings) - 1):
        a, b = hub_start + ring * hub_sides, hub_start + (ring + 1) * hub_sides
        for i in range(hub_sides):
            nxt = (i + 1) % hub_sides
            faces.append((a + i, a + nxt, b + nxt, b + i))
    faces.extend(
        [
            tuple(reversed([hub_start + i for i in range(hub_sides)])),
            tuple(hub_start + 2 * hub_sides + i for i in range(hub_sides)),
        ]
    )

    rotor = finish_surface(mesh_object(name, vertices, faces, MAT_ROTOR), 0, 0.004)
    # Other authored parts have world-space vertices and are uniformly scaled
    # later. This local-origin rotor keeps its center and axis explicit for
    # animation, so its translation is authored at final scale here.
    rotor.location = (
        xc * MODEL_SCALE,
        yc * MODEL_SCALE,
        zc * MODEL_SCALE,
    )
    rotor.rotation_mode = "XYZ"
    rotor["animation_role"] = "rotor"
    rotor["spin_axis_local"] = "+Z"
    rotor["spin_rpm"] = 1_200
    rotor.rotation_euler = (tilt, 0.0, phase)
    rotor.keyframe_insert(data_path="rotation_euler", frame=1, index=2)
    rotor.rotation_euler.z = phase + spin_direction * math.tau
    rotor.keyframe_insert(data_path="rotation_euler", frame=25, index=2)
    return rotor


# Primary monocoque: one continuous shell with purposefully asymmetric section pacing.
body_sections = [
    (-1.22, 0.0, 0.095, 0.012, 0.014, 0.00),
    (-1.16, 0.0, 0.095, 0.075, 0.075, 0.00),
    (-1.04, 0.0, 0.105, 0.225, 0.185, 0.01),
    (-0.72, 0.0, 0.120, 0.335, 0.275, 0.00),
    (-0.30, 0.0, 0.135, 0.380, 0.315, -0.01),
    (0.12, 0.0, 0.145, 0.355, 0.305, 0.00),
    (0.48, 0.0, 0.145, 0.300, 0.270, 0.02),
    (0.82, 0.0, 0.125, 0.235, 0.205, 0.00),
    (1.10, 0.0, 0.105, 0.135, 0.130, 0.00),
    (1.28, 0.0, 0.095, 0.035, 0.045, 0.00),
    (1.32, 0.0, 0.095, 0.006, 0.008, 0.00),
]
loft("HBK_Main_Monocoque", body_sections, MAT_BODY, sides=36, power=2.45, subdivision=2)

# Lower structural keel, blended visually into the monocoque.
keel_sections = [
    (-0.92, 0.0, -0.105, 0.012, 0.010, 0.0),
    (-0.88, 0.0, -0.125, 0.085, 0.050, 0.0),
    (-0.55, 0.0, -0.195, 0.100, 0.120, 0.0),
    (-0.32, 0.0, -0.215, 0.070, 0.135, 0.0),
    (0.05, 0.0, -0.215, 0.065, 0.135, 0.0),
    (0.38, 0.0, -0.190, 0.070, 0.120, 0.0),
    (0.58, 0.0, -0.160, 0.090, 0.100, 0.0),
    (0.88, 0.0, -0.075, 0.035, 0.040, 0.0),
    (0.92, 0.0, -0.060, 0.008, 0.008, 0.0),
]
loft("HBK_Ventral_Keel", keel_sections, MAT_PANEL, sides=28, power=2.2, subdivision=2)

# Sculpted upper cowl directs air around the rider rather than reading as a flat canopy.
cowl_sections = [
    (-0.40, 0.0, 0.335, 0.020, 0.010, 0.0),
    (-0.35, 0.0, 0.345, 0.225, 0.050, 0.0),
    (-0.10, 0.0, 0.405, 0.260, 0.115, 0.0),
    (0.28, 0.0, 0.425, 0.235, 0.165, 0.0),
    (0.58, 0.0, 0.385, 0.170, 0.145, 0.0),
    (0.90, 0.0, 0.280, 0.075, 0.060, 0.0),
    (0.96, 0.0, 0.260, 0.010, 0.010, 0.0),
]
loft("HBK_Forward_Cowl", cowl_sections, MAT_PANEL, sides=30, power=2.25, subdivision=2)

# Saddle uses its own ergonomic crown and pinched waist.
seat_sections = [
    (-0.89, 0.0, 0.355, 0.016, 0.010, 0.0),
    (-0.84, 0.0, 0.365, 0.120, 0.045, 0.0),
    (-0.67, 0.0, 0.430, 0.245, 0.090, 0.0),
    (-0.35, 0.0, 0.450, 0.260, 0.075, 0.0),
    (-0.05, 0.0, 0.435, 0.205, 0.055, 0.0),
    (0.09, 0.0, 0.410, 0.095, 0.025, 0.0),
    (0.13, 0.0, 0.405, 0.012, 0.008, 0.0),
]
loft("HBK_Saddle", seat_sections, MAT_SEAT, sides=28, power=2.8, subdivision=2)

# Two downward-facing fans are recessed into shaped ventral ducts. Nothing
# protrudes from the side silhouette, and every rotor is contained by its duct.
for side, yc, tilt in (
    ("L", 0.415, math.radians(8.0)),
    ("R", -0.415, math.radians(-8.0)),
):
    fan_z = -0.225
    annular_duct(f"HBK_Downward_Duct_{side}", 0.0, yc, fan_z, tilt)
    animated_rotor(
        f"HBK_Downward_Rotor_{side}",
        0.0,
        yc,
        fan_z,
        tilt,
        0.12 if side == "L" else -0.12,
        1.0 if side == "L" else -1.0,
    )
scene.frame_start = 1
scene.frame_end = 24
scene.frame_set(1)

# Rider controls: a single bent titanium bar with integrated grips.
curve_pipe(
    "HBK_Control_Bar",
    [
        (0.36, -0.325, 0.535),
        (0.42, -0.170, 0.560),
        (0.44, 0.0, 0.545),
        (0.42, 0.170, 0.560),
        (0.36, 0.325, 0.535),
    ],
    0.018,
    MAT_EDGE,
)
for side, sign in (("L", 1.0), ("R", -1.0)):
    loft(
        f"HBK_Grip_{side}",
        [
            (0.365, sign * 0.318, 0.538, 0.008, 0.008, 0.0),
            (0.345, sign * 0.326, 0.535, 0.035, 0.035, 0.0),
            (0.275, sign * 0.355, 0.525, 0.041, 0.041, 0.0),
            (0.255, sign * 0.365, 0.522, 0.008, 0.008, 0.0),
        ],
        MAT_SEAT,
        sides=20,
        power=2.0,
        subdivision=1,
    )

# Tail shoulders give the compact craft a directionally stable but agile silhouette.
for side, sign in (("L", 1.0), ("R", -1.0)):
    top = [
        (-0.96, sign * 0.18, 0.25),
        (-1.25, sign * 0.30, 0.20),
        (-1.10, sign * 0.34, 0.10),
        (-0.77, sign * 0.24, 0.13),
    ]
    bottom = [(x + 0.015, y, z - 0.032) for x, y, z in top]
    prism_from_profiles(f"HBK_Tail_Vane_{side}", top, bottom, MAT_BODY, 0.018)

# Emissive energy paths follow the compound surface silhouette.
for side, sign in (("L", 1.0), ("R", -1.0)):
    curve_pipe(
        f"HBK_Field_Rail_{side}",
        [
            (-0.81, sign * 0.327, 0.205),
            (-0.42, sign * 0.385, 0.225),
            (0.04, sign * 0.365, 0.250),
            (0.48, sign * 0.298, 0.245),
            (0.82, sign * 0.213, 0.205),
        ],
        0.009,
        MAT_CYAN,
    )

# Headlight and tail nav lights are modeled lenses.
loft(
    "HBK_Headlight",
    [
        (1.215, 0.0, 0.095, 0.010, 0.008, 0.0),
        (1.225, 0.0, 0.095, 0.065, 0.032, 0.0),
        (1.285, 0.0, 0.095, 0.026, 0.018, 0.0),
        (1.300, 0.0, 0.095, 0.005, 0.005, 0.0),
    ],
    MAT_CYAN,
    sides=24,
    power=2.8,
    subdivision=1,
)
for side, sign in (("L", 1.0), ("R", -1.0)):
    curve_pipe(
        f"HBK_Tail_Light_{side}",
        [
            (-1.06, sign * 0.205, 0.205),
            (-1.17, sign * 0.245, 0.190),
        ],
        0.014,
        MAT_AMBER,
    )

# A small instrument pod with a recessed cyan interface.
loft(
    "HBK_Instrument_Pod",
    [
        (0.17, 0.0, 0.495, 0.010, 0.008, 0.0),
        (0.20, 0.0, 0.505, 0.105, 0.035, 0.0),
        (0.37, 0.0, 0.530, 0.125, 0.060, 0.0),
        (0.47, 0.0, 0.515, 0.080, 0.030, 0.0),
        (0.50, 0.0, 0.505, 0.010, 0.008, 0.0),
    ],
    MAT_BODY,
    sides=24,
    power=2.4,
    subdivision=2,
)
curve_pipe(
    "HBK_Instrument_Display",
    [(0.395, -0.065, 0.555), (0.415, 0.0, 0.565), (0.395, 0.065, 0.555)],
    0.008,
    MAT_CYAN,
)

# Keep the production asset decisively smaller than the Kestrel's 2.2 m
# rotor span while retaining believable single-rider ergonomics.
for model_object in model_collection.objects:
    model_object.scale = (MODEL_SCALE, MODEL_SCALE, MODEL_SCALE)

# Metadata for integration and review.
scene["asset_id"] = "vehicle:sparrow-hoverbike"
scene["forward_axis"] = "+X"
scene["up_axis"] = "+Z"
scene["units"] = "metres"
scene["design_length_m"] = 2.05
scene["design_width_m"] = 1.32
scene["design_height_m"] = 0.73
scene["kestrel_reference_rotor_span_m"] = 2.20
scene["integration_status"] = "model-only"

blend_path = os.path.join(ASSET_DIR, "sparrow_hoverbike.blend")
bpy.ops.wm.save_as_mainfile(filepath=blend_path)

print("SPARROW_BUILD_OK")
print(blend_path)
