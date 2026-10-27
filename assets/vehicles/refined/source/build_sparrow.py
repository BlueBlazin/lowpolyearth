"""Author the Sparrow sport hoverbike and export its evaluated game mesh.

Run with Blender --background --python this_file. Add -- --preview to render
an internal modelling check. Continuous superellipse lofts derive from the
project's original hoverbike/source/build_hoverbike.py; coordinates here are
metres, +X forward and +Z up, rotated into the shared exporter at the end.
"""

import math
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector


ROOT = Path(__file__).resolve().parents[4]
OUT = ROOT / "assets/vehicles/refined"
sys.path.insert(0, str(OUT / "tools"))
sys.dont_write_bytecode = True
from export_vehicle import export_scene

bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
for collection in list(bpy.data.collections):
    bpy.data.collections.remove(collection)
scene = bpy.context.scene
bpy.context.preferences.filepaths.save_version = 0
scene.unit_settings.system = "METRIC"
scene.unit_settings.scale_length = 1.0
collection = bpy.data.collections.new("SPARROW")
scene.collection.children.link(collection)


def linear(value):
    value /= 255.0
    return value / 12.92 if value <= .04045 else ((value + .055) / 1.055) ** 2.4


def material(name, rgb, ident, roughness=.35, metallic=0.0):
    mat = bpy.data.materials.new(name)
    color = tuple(linear(channel) for channel in rgb)
    mat.diffuse_color = (*color, 1)
    mat.use_nodes = True
    node = mat.node_tree.nodes.get("Principled BSDF")
    node.inputs["Base Color"].default_value = (*color, 1)
    node.inputs["Roughness"].default_value = roughness
    node.inputs["Metallic"].default_value = metallic
    if ident in (100, 101):
        node.inputs["Emission Color"].default_value = (*color, 1)
        node.inputs["Emission Strength"].default_value = 2.0
    mat["vearth_material_id"] = ident
    mat["vearth_color_srgb"] = list(rgb)
    return mat


IVORY = material("Sparrow | warm porcelain", (227, 224, 211), 96, .30)
CRIMSON = material("Sparrow | oxblood lacquer", (135, 31, 39), 96, .32)
RED_EDGE = material("Sparrow | scarlet seam", (172, 46, 49), 96, .34)
DARK = material("Sparrow | graphite machine casing", (42, 49, 55), 97, .4, .85)
STEEL = material("Sparrow | satin titanium", (139, 151, 157), 97, .3, .9)
RUBBER = material("Sparrow | technical saddle", (32, 35, 39), 98, .72)
SEAM = material("Sparrow | recessed panel joint", (22, 27, 30), 98, .7)
GLASS = material("Sparrow | smoked instrument glass", (33, 54, 64), 99, .16)
COOL = material("Sparrow | vector drive ceramic", (143, 214, 230), 100, .3)
HEADLIGHT = material("Sparrow | headlamp", (250, 234, 186), 101, .3)
TAILLIGHT = material("Sparrow | rear signal lens", (240, 70, 45), 101, .3)
STITCH = material("Sparrow | saddle thread", (82, 85, 89), 98, .7)


def mesh_object(name, vertices, faces, mat, smooth=True, part="body"):
    mesh = bpy.data.meshes.new(name + " mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    obj.data.materials.append(mat)
    obj["vearth_part"] = part
    for polygon in mesh.polygons:
        polygon.use_smooth = smooth
    return obj


def bevel(obj, width, segments=2):
    mod = obj.modifiers.new("Manufactured edge radius", "BEVEL")
    mod.width = width
    mod.segments = segments
    mod.limit_method = "ANGLE"
    mod.angle_limit = math.radians(30)
    mod.harden_normals = True
    return obj


def loft(name, sections, mat, sides=24, power=2.4, subdivision=1):
    """Section = longitudinal position, lateral centre, height, halfwidth, halfheight."""
    vertices = []
    for x, yc, zc, yr, zr in sections:
        for i in range(sides):
            a = math.tau * i / sides
            c, s = math.cos(a), math.sin(a)
            y = math.copysign(abs(c) ** (2.0 / power), c) * yr
            z = math.copysign(abs(s) ** (2.0 / power), s) * zr
            vertices.append((x, yc + y, zc + z))
    faces = []
    for ring in range(len(sections) - 1):
        a, b = ring * sides, (ring + 1) * sides
        for i in range(sides):
            j = (i + 1) % sides
            faces.append((a + i, a + j, b + j, b + i))
    faces.append(tuple(reversed(range(sides))))
    end = (len(sections) - 1) * sides
    faces.append(tuple(end + i for i in range(sides)))
    obj = mesh_object(name, vertices, faces, mat)
    if subdivision:
        mod = obj.modifiers.new("Fairing curvature", "SUBSURF")
        mod.levels = subdivision
        mod.render_levels = subdivision
    return obj


def tube(name, points, radius, mat, cyclic=False, part="body"):
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = radius
    curve.bevel_resolution = 2
    curve.resolution_u = 6
    curve.use_fill_caps = True
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for knot, point in zip(spline.bezier_points, points):
        knot.co = point
        knot.handle_left_type = "AUTO"
        knot.handle_right_type = "AUTO"
    spline.use_cyclic_u = cyclic
    obj = bpy.data.objects.new(name, curve)
    collection.objects.link(obj)
    obj.data.materials.append(mat)
    obj["vearth_part"] = part
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj.select_set(False)
    return obj


def box(name, centre, dimensions, mat, radius=.015):
    bpy.ops.mesh.primitive_cube_add(size=1, location=centre)
    obj = bpy.context.object
    obj.name = name
    for c in list(obj.users_collection):
        c.objects.unlink(obj)
    collection.objects.link(obj)
    obj.dimensions = dimensions
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.data.materials.append(mat)
    obj["vearth_part"] = "body"
    bevel(obj, min(radius, min(dimensions) * .35), 3)
    for polygon in obj.data.polygons:
        polygon.use_smooth = False
    obj.select_set(False)
    return obj


def lathe(name, centre, axis, profile, mat, sides=36, part="body"):
    axis = Vector(axis).normalized()
    u = axis.cross(Vector((0, 0, 1)))
    if u.length < .01:
        u = axis.cross(Vector((1, 0, 0)))
    u.normalize()
    v = axis.cross(u)
    centre = Vector(centre)
    points = []
    for axial, radius in profile:
        for i in range(sides):
            a = math.tau * i / sides
            points.append(centre + axis * axial + radius * (u * math.cos(a) + v * math.sin(a)))
    faces = []
    for ring in range(len(profile)):
        nr = (ring + 1) % len(profile)
        for i in range(sides):
            j = (i + 1) % sides
            faces.append((ring * sides + i, ring * sides + j, nr * sides + j, nr * sides + i))
    return mesh_object(name, points, faces, mat, part=part)


# The red load-bearing spine narrows beneath the rider; the front shell rises
# separately around the instruments, keeping the saddle visibly accessible.
loft("01 structural monocoque", [
    (-1.34, 0, .07, .045, .045), (-1.28, 0, .07, .15, .095),
    (-1.10, 0, .09, .24, .17), (-.78, 0, .10, .28, .23),
    (-.42, 0, .06, .26, .22), (-.05, 0, .04, .255, .23),
    (.38, 0, .08, .32, .24), (.76, 0, .10, .31, .22),
    (1.08, 0, .08, .21, .15), (1.34, 0, .04, .06, .055),
    (1.40, 0, .04, .025, .025),
], CRIMSON)
loft("02 ventral drive keel", [
    (-1.17, 0, -.12, .06, .05), (-1.02, 0, -.17, .18, .11),
    (-.57, 0, -.22, .17, .13), (-.12, 0, -.22, .13, .14),
    (.45, 0, -.19, .16, .12), (.88, 0, -.07, .10, .08),
    (1.08, 0, .00, .025, .025),
], DARK, sides=20)
cowl = loft("03 upper nose fairing", [
    (.20, 0, .32, .16, .025), (.26, 0, .34, .235, .06),
    (.44, 0, .36, .275, .12), (.65, 0, .30, .29, .16),
    (.95, 0, .20, .25, .13), (1.23, 0, .095, .15, .065),
    (1.39, 0, .045, .06, .026), (1.43, 0, .04, .025, .013),
], IVORY, sides=28)
loft("04 rear seat shell", [
    (-1.27, 0, .15, .06, .025), (-1.14, 0, .23, .20, .07),
    (-.98, 0, .29, .245, .08), (-.74, 0, .29, .25, .05),
    (-.45, 0, .25, .225, .035), (-.30, 0, .25, .15, .028),
    (-.16, 0, .25, .035, .013), (-.12, 0, .25, .018, .008),
], IVORY, sides=24)
saddle = loft("05 ergonomic saddle", [
    (-1.02, 0, .34, .12, .02), (-.96, 0, .39, .205, .05),
    (-.76, 0, .42, .245, .07), (-.54, 0, .42, .22, .045),
    (-.28, 0, .39, .17, .032), (-.02, 0, .37, .12, .035),
    (.09, 0, .365, .05, .018),
], RUBBER, sides=24, power=3)

for sign, side in ((-1, "port"), (1, "starboard")):
    # Shells are offset curved lofts, not a flat slab glued to the main body.
    loft("06 sculpted lower cheek " + side, [
        (-.85, sign * .21, .02, .04, .02),
        (-.64, sign * .255, -.015, .045, .095),
        (-.27, sign * .265, -.01, .045, .10),
        (.20, sign * .29, .045, .045, .10),
        (.55, sign * .305, .075, .042, .095),
        (.93, sign * .215, .045, .033, .06),
        (1.12, sign * .13, .025, .018, .02),
    ], IVORY, sides=16, power=2.6)
    tube("Panel boundary " + side, [
        (-.83, sign * .257, .08), (-.61, sign * .304, .09),
        (-.25, sign * .315, .10), (.18, sign * .340, .15),
        (.53, sign * .350, .175), (.87, sign * .260, .12),
    ], .0055, SEAM)
    tube("Upper fairing joint " + side, [
        (.27, sign * .232, .35), (.43, sign * .265, .32),
        (.76, sign * .289, .24), (1.10, sign * .195, .13),
    ], .0045, SEAM)
    tube("Saddle welt " + side, [
        (-.95, sign * .187, .405), (-.77, sign * .236, .448),
        (-.54, sign * .211, .44), (-.28, sign * .158, .406),
        (-.03, sign * .115, .39),
    ], .0038, DARK)

    # Downward lift ducts sit forward of the rider's heels and inside the
    # shell width envelope. An inlet lip, throat and inner rotor are distinct.
    centre = (.04, sign * .495, -.17)
    tilt = math.radians(sign * -8)
    axis = Vector((0, math.sin(tilt), math.cos(tilt)))
    lathe("07 lift duct " + side, centre, axis, [
        (-.11, .242), (-.065, .278), (.025, .283), (.10, .266),
        (.125, .242), (.108, .220), (.045, .218), (-.09, .228),
    ], CRIMSON, sides=40)
    lathe("Duct inlet rim " + side, centre, axis, [
        (.099, .265), (.115, .252), (.127, .238),
        (.111, .220), (.099, .230),
    ], STEEL, sides=40)
    lathe("Duct shadow throat " + side, centre, axis, [
        (-.085, .226), (.093, .219), (.093, .208), (-.086, .216),
    ], DARK, sides=32)
    # The rotor sits deep inside its shroud; it remains a static part of the
    # model instead of spending animation geometry on nearly hidden blades.
    for frame in [0]:
        part = "body"
        hub_centre = Vector(centre) + axis * .018
        lathe("Rotor hub " + side + str(frame), hub_centre, axis, [
            (-.022, .028), (.010, .043), (.047, .033), (.055, .009),
        ], STEEL, sides=16, part=part)
        u = Vector((1, 0, 0)); v = axis.cross(u)
        for blade in range(5):
            angle = math.tau * (blade / 5 + frame / 20)
            radial = u * math.cos(angle) + v * math.sin(angle)
            tangent = -u * math.sin(angle) + v * math.cos(angle)
            points = []
            for radius, chord, sweep, z in ((.033, .045, .00, .01), (.12, .067, .015, .002), (.205, .032, .04, -.004)):
                points.extend([hub_centre + radial * radius - tangent * (sweep + chord / 2) + axis * z,
                               hub_centre + radial * radius - tangent * (sweep - chord / 2) + axis * (z - .01)])
            obj = mesh_object("Rotor blade " + side + str(frame) + str(blade), points,
                              [(0, 2, 3, 1), (2, 4, 5, 3)], STEEL, part=part)
            mod = obj.modifiers.new("Blade section", "SOLIDIFY"); mod.thickness = .004
    tube("Duct support " + side, [(-.31, sign * .23, -.15), (-.18, sign * .38, -.18), (.04, sign * .50, -.18)], .024, DARK)

    # A motorcycle seating triangle: saddle, hand grip and foot platform.
    tube("Footrest bracket " + side, [(-.40, sign * .16, -.22), (-.46, sign * .35, -.23), (-.54, sign * .405, -.22)], .028, STEEL)
    box("Foot platform " + side, (-.49, sign * .43, -.19), (.29, .12, .055), DARK, .016)
    for rib in range(5):
        box("Foot platform traction " + side + str(rib), (-.60 + rib * .052, sign * .43, -.156), (.014, .10, .010), RUBBER, .004)
    tube("Handlebar " + side, [(.30, 0, .50), (.33, sign * .15, .61), (.24, sign * .30, .665), (.15, sign * .40, .65)], .017, STEEL)
    tube("Handle grip " + side, [(.23, sign * .31, .661), (.15, sign * .40, .65)], .026, RUBBER)
    tube("Brake lever " + side, [(.28, sign * .305, .637), (.27, sign * .40, .617), (.18, sign * .445, .61)], .008, STEEL)
    tube("Control hose " + side, [(.25, sign * .30, .64), (.35, sign * .18, .48), (.22, sign * .12, .36)], .006, SEAM)

    # Rear propulsion units nest inside the tail shoulders, with a dark
    # exhaust cavity between the nozzle lip and emissive drive surface.
    exhaust_centre = (-1.19, sign * .24, -.085)
    axis = (-1, 0, 0)
    lathe("08 rear drive housing " + side, exhaust_centre, axis, [
        (-.36, .065), (-.25, .110), (-.06, .124), (.06, .105),
        (.105, .092), (.105, .073), (-.04, .073),
    ], DARK, sides=28)
    lathe("Drive nozzle machined rim " + side, exhaust_centre, axis, [
        (.078, .105), (.105, .096), (.119, .086),
        (.115, .072), (.082, .078),
    ], STEEL, sides=28)
    lathe("Drive ceramic ring " + side, exhaust_centre, axis, [
        (.064, .072), (.068, .071), (.068, .050), (.064, .052),
    ], COOL, sides=24)
    for ring in range(4):
        lathe("Drive cooling band " + side + str(ring), exhaust_centre, axis, [
            (-.19 + ring * .038, .118), (-.18 + ring * .038, .122),
            (-.172 + ring * .038, .118),
        ], STEEL, sides=24)
    tube("Rear grab handle " + side, [(-.75, sign * .23, .29), (-.99, sign * .28, .30), (-1.12, sign * .22, .255)], .016, DARK)
    tube("Rear stop lamp " + side, [(-1.24, sign * .105, .205), (-1.22, sign * .190, .202)], .012, TAILLIGHT)

    # Fasteners follow real assembly boundaries; they are not random greebles.
    for number, (x, y, z) in enumerate([(-.65, .293, .08), (-.20, .305, .09), (.50, .338, .145), (.98, .218, .11)]):
        lathe("Panel fastener " + side + str(number), (x, sign * y, z), (0, sign, 0),
              [(-.001, .009), (.004, .009), (.007, .006), (.005, .003)], STEEL, sides=8)

# Twin recessed headlights define the nose without a continuous neon outline.
for sign in (-1, 1):
    tube("Headlamp gasket " + str(sign), [(1.285, sign * .10, .09), (1.15, sign * .177, .145)], .024, SEAM)
    tube("Headlamp lens " + str(sign), [(1.296, sign * .10, .093), (1.162, sign * .177, .149)], .013, HEADLIGHT)

pod = box("Instrument enclosure", (.275, 0, .533), (.19, .22, .075), DARK, .025)
pod.rotation_euler.y = math.radians(-18)
display = box("Recessed instrument glass", (.255, 0, .576), (.145, .165, .009), GLASS, .015)
display.rotation_euler.y = math.radians(-18)
for i in range(3):
    tube("Instrument indicator " + str(i), [(.235 + i * .025, -.045, .582 + i * .008), (.235 + i * .025, .025, .582 + i * .008)], .0025, COOL)


def on_surface(obj, x, y, lift=0.002):
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    hit, position, normal, _ = evaluated.ray_cast(Vector((x, y, 2)), Vector((0, 0, -1)))
    if not hit:
        raise ValueError(f"Surface marking missed {obj.name} at {x}, {y}")
    return position + normal * lift, normal


# The cooling apertures have actual recessed geometry and dark floors. Their
# local frames follow the fairing normal so the slots do not float above it.
for sign in (-1, 1):
    surface, normal = on_surface(cowl, .445, sign * .135, 0)
    along = (Vector((1, 0, 0)) - normal * normal.x).normalized()
    across = normal.cross(along).normalized()
    orientation = Matrix((along, across, normal)).transposed().to_4x4()
    cutter = box("Cooling aperture cutter " + str(sign), (0, 0, 0), (.15, .026, .10), SEAM, .008)
    cutter.matrix_world = Matrix.Translation(surface - normal * .028) @ orientation
    cutter.hide_render = True
    cutter.hide_set(True)
    modifier = cowl.modifiers.new("Recessed cooling aperture " + str(sign), "BOOLEAN")
    modifier.operation = "DIFFERENCE"
    modifier.solver = "EXACT"
    modifier.object = cutter
    floor = box("Cooling aperture floor " + str(sign), (0, 0, 0), (.143, .020, .012), SEAM, .003)
    floor.matrix_world = Matrix.Translation(surface - normal * .045) @ orientation

# Original single-stroke lettering avoids an external font asset. The small
# production code follows the compound nose surface rather than a flat decal.
glyphs = {
    "S": [[(2, 3), (0, 3), (0, 1.5), (2, 1.5), (2, 0), (0, 0)]],
    "P": [[(0, 0), (0, 3), (2, 3), (2, 1.5), (0, 1.5)]],
    "-": [[(.3, 1.5), (1.7, 1.5)]],
    "0": [[(0, 0), (0, 3), (2, 3), (2, 0), (0, 0)]],
    "2": [[(0, 3), (2, 3), (2, 1.7), (0, 0), (2, 0)]],
}
for index, glyph in enumerate("SP-02"):
    for stroke_index, stroke in enumerate(glyphs[glyph]):
        points = [on_surface(cowl, .80 + v * .017, -.104 + (index * 2.5 + u) * .017, .003)[0]
                  for u, v in stroke]
        tube("SP-02 marking " + str(index) + str(stroke_index), points, .0028, CRIMSON)

for x, halfwidth in ((-.82, .195), (-.46, .19)):
    seam_points = [on_surface(saddle, x, halfwidth * step / 4, .002)[0] for step in range(-4, 5)]
    tube("Saddle tailored panel seam " + str(x), seam_points, .002, SEAM)
    for step in range(-5, 6):
        y = step * halfwidth / 6
        points = [on_surface(saddle, x + .005, y, .0025)[0],
                  on_surface(saddle, x + .019, y, .0025)[0]]
        tube("Saddle stitch " + str(x) + str(step), points, .0012, STITCH)

axis_rotation = Matrix.Rotation(-math.pi / 2, 4, "Z")
for obj in collection.objects:
    obj.matrix_world = axis_rotation @ obj.matrix_world
    # Solid-colour parts have no sampled image coordinates. Removing Blender's
    # primitive UV layers also avoids irrelevant bevel-interpolation drift.
    if obj.type == "MESH":
        for layer in list(obj.data.uv_layers):
            obj.data.uv_layers.remove(layer)
scene["asset_id"] = "vehicle:sparrow"
scene["forward_axis"] = "-Y"
scene["up_axis"] = "+Z"
scene["units"] = "metres"
scene["exhaust_ports_engine"] = [-.24, -.085, -1.315, .24, -.085, -1.315]
scene["exhaust_direction_engine"] = [0, 0, -1]
scene["exhaust_radius_m"] = .070
scene["source_design"] = "assets/vehicles/hoverbike/source/build_hoverbike.py"

OUT.mkdir(parents=True, exist_ok=True)
blend_path = OUT / "sparrow.blend"
bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
# Export the serialized scene so Blender's evaluated transform/normal caches
# cannot differ from a later export performed directly from the saved source.
bpy.ops.wm.open_mainfile(filepath=str(blend_path))
scene = bpy.context.scene
collection = bpy.data.collections["SPARROW"]
manifest = export_scene(
    OUT / "sparrow.mesh",
    source_paths=[Path(__file__), blend_path, ROOT / "assets/vehicles/hoverbike/source/build_hoverbike.py", ROOT / "assets/vehicles/hoverbike/sparrow_hoverbike.blend", ROOT / "LICENSE"],
    author="Vearth project contributors",
    license_name="MIT; see repository LICENSE",
)
print("SPARROW_PARTS", manifest["parts"])

if "--preview" in sys.argv:
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 24
    scene.cycles.use_denoising = True
    scene.render.resolution_x = 1200
    scene.render.resolution_y = 900
    scene.render.resolution_percentage = 100
    scene.world.use_nodes = True
    scene.world.node_tree.nodes["Background"].inputs[0].default_value = (.19, .23, .29, 1)
    scene.world.node_tree.nodes["Background"].inputs[1].default_value = .45
    bpy.ops.object.camera_add(location=(3.5, -4.2, 2.8))
    camera = bpy.context.object
    camera.rotation_euler = (Vector((0, 0, .1)) - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera.data.type = "ORTHO"
    camera.data.ortho_scale = 3.65
    scene.camera = camera
    for location, energy, size in [((1, -2, 5), 450, 4), ((-3, -1, 2), 280, 3), ((0, 3, 4), 650, 3)]:
        bpy.ops.object.light_add(type="AREA", location=location)
        light = bpy.context.object
        light.data.energy = energy
        light.data.shape = "DISK"
        light.data.size = size
        light.rotation_euler = (-light.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.film_transparent = False
    output = ROOT / "output/vehicle-refinement"
    output.mkdir(parents=True, exist_ok=True)
    scene.render.filepath = str(output / "sparrow-modelling-check.png")
    bpy.ops.render.render(write_still=True)
