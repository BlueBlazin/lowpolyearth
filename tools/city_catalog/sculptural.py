"""Six original, static architectural studies in local metric coordinates.

The source references describe physical forms and dimensions. No downloaded
meshes, textures, logos, or photographic pixels participate in these assets.
"""
import math

import numpy as np

from common import Mesh


TAU = math.tau
STONE = (211, 205, 190)
WHITE = (237, 237, 225)
SILVER = (166, 177, 184)
GLASS = (35, 54, 62)
WARM = (245, 190, 115)


ENTRIES = [
    dict(name="dongdaemun-design-plaza", city="Seoul", lat=37.56710068, lon=127.00984227,
         heading=12., base="terrain", suppress_buildings=True,
         suppress_roads=False, suppress_trees=True,
         references=["https://www.zha.com/projects/architecture/dongdaemun-design-plaza",
                     "https://kiaf.org/insights/2905",
                     "https://www.openstreetmap.org/relation/10647068"],
         description="Asymmetric aluminum-clad volumes with recessed garden roofs, a cantilevered northern nose, and steady warm entrance lighting.",
         capture=dict(distance=400., azimuth=235., elevation=24.)),
    dict(name="rotterdam-cube-houses", city="Rotterdam", lat=51.92028399, lon=4.49051507,
         heading=24., base="terrain", suppress_buildings=True,
         suppress_roads=False, suppress_trees=True,
         suppress_bbox=([-59., -46.], [66., 46.]),
         references=["https://kubuswoning.nl/en/",
                     "https://rotterdam.info/app/uploads/2018/09/rotterdamICONS.pdf",
                     "https://www.openstreetmap.org/node/34051054"],
         description="A compact forest of yellow corner-up cube dwellings on hexagonal masonry stems, with slate roofs and white-framed oblique windows.",
         capture=dict(distance=155., azimuth=225., elevation=19.)),
    dict(name="seville-setas", city="Seville", lat=37.39313734, lon=-5.99182458,
         heading=95., base="terrain", suppress_buildings=True,
         suppress_roads=False, suppress_trees=True,
         references=["https://setasdesevilla.com/en/events/-/asset_publisher/rLfde59VaxQq/",
                     "https://www.spain.info/en/top/andalusia-contemporary-architecture/"],
         description="Six connected mushroom canopies formed from open orthogonal timber fins, with a winding rooftop promenade and shaded market podium.",
         capture=dict(distance=240., azimuth=210., elevation=32.)),
    dict(name="lotus-temple", city="New Delhi", lat=28.55336, lon=77.25861,
         heading=12., base="terrain", suppress_buildings=True,
         suppress_roads=False, suppress_trees=True, activation_pad=8.,
         references=["https://bahaihouseofworship.in/architecture/",
                     "https://bahaihouseofworship.in/wp-content/uploads/2024/04/architectural_blossoming_of_the_lotus.pdf",
                     "https://www.openstreetmap.org/way/44827278"],
         description="Twenty-seven marble shells in three ninefold tiers above a sandstone podium, nine blue pools, and radial entrance bridges.",
         capture=dict(distance=170., azimuth=192., elevation=19.)),
    dict(name="louvre-abu-dhabi", city="Abu Dhabi", lat=24.53377587, lon=54.39844611,
         heading=35., base=4., suppress_buildings=True,
         suppress_roads=False, suppress_trees=True,
         suppress_bbox=([-125., -139.], [167., 146.]),
         references=["https://www.louvreabudhabi.ae/en/about-us/architecture",
                     "https://dct.gov.ae/en/media.centre/news/louvre.abu.dhabi.closer.to.completion.with.two.major.milestones.aspx",
                     "https://www.openstreetmap.org/way/498046180"],
         description="A 180-metre shallow silver dome with an open star lattice above a waterfront cluster of low white gallery volumes.",
         capture=dict(distance=295., azimuth=300., elevation=24.)),
    dict(name="miyajima-torii", city="Miyajima", lat=34.29730, lon=132.31814,
         heading=133., base=-1., suppress_buildings=True,
         suppress_roads=False, suppress_trees=False,
         references=["https://www.miyajima.or.jp/sightseeing/ss_ootorii.html",
                     "https://visit-miyajima-japan.com/en/culture-and-heritage/spiritual-heritage-temples-shrines/le-torii-flottant.html",
                     "https://www.openstreetmap.org/way/555763409"],
         description="The vermilion Ryobu gate: two great camphor posts, four sleeve pillars, penetrating ties, and the upswept dark cypress cap.",
         capture=dict(distance=65., azimuth=305., elevation=10.)),
]


def _quad(mesh, points, color, material, normal):
    p = np.asarray(points)
    if np.dot(np.cross(p[1] - p[0], p[2] - p[0]), normal) < 0:
        p = p[::-1]
    mesh.quad(*p, color, material)


def _polygon(mesh, points, color, material, normal):
    p = np.asarray(points)
    if len(p) < 3:
        return
    if np.dot(np.cross(p[1] - p[0], p[2] - p[0]), normal) < 0:
        p = p[::-1]
    for i in range(1, len(p) - 1):
        mesh.triangle(p[0], p[i], p[i + 1], color, material)


def _clip_polygon(points, normal, distance):
    """Convex half-plane clipping in a face's own two-dimensional frame."""
    output = []
    for a, b in zip(points, points[1:] + points[:1]):
        da, db = np.dot(a, normal) - distance, np.dot(b, normal) - distance
        if da <= 0:
            output.append(a)
        if (da < 0 < db) or (db < 0 < da):
            output.append(a + (b - a) * da / (da - db))
    return output


def _ring(radius, height, count=64, center=(0., 0.)):
    return [np.array((center[0] + radius * math.cos(i * TAU / count), height,
                      center[1] + radius * math.sin(i * TAU / count))) for i in range(count)]


def _disc(mesh, radius, bottom, top, color, material=8, count=64):
    low, high = _ring(radius, bottom, count), _ring(radius, top, count)
    for i in range(count):
        j = (i + 1) % count
        _quad(mesh, [low[i], low[j], high[j], high[i]], color, material, high[i] * (1, 0, 1))
        mesh.triangle((0, top, 0), high[j], high[i], color, material)
        mesh.triangle((0, bottom, 0), low[i], low[j], color, material)


def _ribbon(mesh, points, width, thickness, color, material=11):
    """Horizontal-width swept solid; the supplied path must not close on itself."""
    points = np.asarray(points, dtype=float)
    frames = []
    for i, p in enumerate(points):
        tangent = points[min(i + 1, len(points) - 1)] - points[max(i - 1, 0)]
        tangent[1] = 0
        tangent /= np.linalg.norm(tangent)
        side = np.array((-tangent[2], 0., tangent[0])) * width * .5
        up = np.array((0., thickness * .5, 0.))
        frames.append([p - side - up, p + side - up, p + side + up, p - side + up])
    for a, b in zip(frames, frames[1:]):
        for k in range(4):
            q = [a[k], a[(k + 1) % 4], b[(k + 1) % 4], b[k]]
            center = np.mean(a + b, axis=0)
            _quad(mesh, q, color, material, np.mean(q, axis=0) - center)
    _quad(mesh, frames[0], color, material, points[0] - points[1])
    _quad(mesh, frames[-1], color, material, points[-1] - points[-2])


def _catmull(points, steps, closed=True):
    p = [np.asarray(q, dtype=float) for q in points]
    result = []
    for i in range(len(p) if closed else len(p) - 1):
        a, b, c, d = [p[j % len(p)] if closed else p[min(max(j, 0), len(p) - 1)]
                      for j in (i - 1, i, i + 1, i + 2)]
        for k in range(steps):
            t = k / steps
            result.append(.5 * ((2 * b) + (-a + c) * t + (2 * a - 5 * b + 4 * c - d) * t * t
                                  + (-a + 3 * b - 3 * c + d) * t ** 3))
    if not closed:
        result.append(p[-1])
    return result


def _lotus(lod):
    m = Mesh()
    segments = (144, 90, 54)[lod]
    _disc(m, 53., -1.5, .7, (172, 134, 121), 8, segments)
    _disc(m, 31., .7, 3., (174, 130, 114), 8, segments)
    _disc(m, 18., 3., 3.4, (173, 153, 135), 8, segments)
    # Pools remain nine independent basins; raised stone rims separate the
    # water from the radial bridges, even at the far silhouette LOD.
    for k in range(9):
        theta = TAU * k / 9
        steps = (24, 16, 10)[lod]
        inner, outer = [], []
        for j in range(steps + 1):
            s = -1 + 2 * j / steps
            a = theta + .302 * s
            r0 = 30.8 + 3.2 * (1 - s * s)
            r1 = 48.5 + 3.1 * (1 - s * s)
            inner.append(np.array((r0 * math.cos(a), 1.04, r0 * math.sin(a))))
            outer.append(np.array((r1 * math.cos(a), 1.04, r1 * math.sin(a))))
        for j in range(steps):
            _quad(m, [inner[j], outer[j], outer[j + 1], inner[j + 1]], (28, 137, 156), 1, (0, 1, 0))
        boundary = inner + outer[::-1] + [inner[0]]
        _ribbon(m, [p + (0., .2, 0.) for p in boundary], .75, .5, WHITE, 17)
        a = theta + math.pi / 9
        bridge = [(r * math.cos(a), y, r * math.sin(a)) for r, y in [(28., 3.05), (34., 3.05), (50., 1.12), (55., .75)]]
        _ribbon(m, bridge, 3.1, .4, (185, 144, 128), 8)

    def shell(theta, tier):
        nu, nv = (24, 14) if lod == 0 else (16, 10) if lod == 1 else (10, 6)
        radial = np.array((math.cos(theta), 0., math.sin(theta)))
        tangent = np.array((-math.sin(theta), 0., math.cos(theta)))

        def point(t, s, inside=False):
            if tier == 0:
                radius = 16.4 - 15.6 * t + 5.9 * math.sin(math.pi * t)
                y = 3.2 + 31.1 * t
                width = (1 - t) ** .63 * (5.9 + 4.5 * math.sin(math.pi * t))
                crown = 1.0 * (1 - s * s) * math.sin(math.pi * t)
            elif tier == 1:
                radius = 26.7 - 10.7 * t + 3.5 * math.sin(math.pi * t)
                y = 3.15 + 21.4 * t
                width = (1 - t) ** .66 * (9.0 + 4.0 * math.sin(math.pi * t))
                crown = 1.8 * (1 - s * s) * math.sin(math.pi * t)
            else:
                radius = 25.7 + 11.6 * t
                y = 3.1 + 11.0 * t + 1.5 * math.sin(math.pi * t)
                width = 8.7 * (1 - t) ** .72
                crown = .8 * (1 - s * s) * math.sin(math.pi * t)
                y += 4.0 * (1 - s * s) * (1 - t)
            p = radial * (radius + crown) + tangent * (width * s) + (0., y, 0.)
            if inside:
                p -= radial * .32
                p[1] -= .14
            return p

        for i in range(nu):
            for j in range(nv):
                t0, t1 = i / nu, (i + 1) / nu
                s0, s1 = -1 + 2 * j / nv, -1 + 2 * (j + 1) / nv
                uv = ((t0, s0), (t0, s1), (t1, s1), (t1, s0))
                # Two real shell faces preserve the entrances and petal gaps
                # without depending on a two-sided renderer material.
                q = [point(t, s) for t, s in uv]
                n = radial + (0., .5 if tier == 2 else .16, 0.)
                color = (235 + ((i * 3 + j + tier) % 4), 235 + ((i * 3 + j + tier) % 4), 224 + ((i + j) % 4))
                _quad(m, q, color, 17, n)
                _quad(m, [point(t, s, True) for t, s in uv], (211, 210, 198), 17, -n)
        perimeter = ([(i / nu, -1.) for i in range(nu + 1)] +
                     [(i / nu, 1.) for i in range(nu, -1, -1)] + [(0., -1.)])
        for uv0, uv1 in zip(perimeter, perimeter[1:]):
            a, b = point(*uv0), point(*uv1)
            c, d = point(*uv1, True), point(*uv0, True)
            center = radial * 17 + (0., 15., 0.)
            _quad(m, [a, b, c, d], WHITE, 17, (a + b) * .5 - center)

    for k in range(9):
        theta = TAU * k / 9
        for tier in range(3):
            shell(theta + (math.pi / 9 if tier == 1 else 0.), tier)
        # The dark entrance is recessed behind the upward-folded outer leaf.
        a = theta
        r = np.array((math.cos(a), 0., math.sin(a)))
        t = np.array((-math.sin(a), 0., math.cos(a)))
        _quad(m, [r * 24 - t * 4 + (0, 3.2, 0), r * 24 + t * 4 + (0, 3.2, 0),
                  r * 24 + t * 4 + (0, 7.3, 0), r * 24 - t * 4 + (0, 7.3, 0)], GLASS, 15, r)
        for d in (-2.6, -1.3, 0., 1.3, 2.6):
            p = r * 24.05 + t * d
            m.beam(p + (0, 3.2, 0), p + (0, 7.3, 0), .055, (161, 157, 143), 16, 4)
    _disc(m, 3.6, 30.8, 31., (107, 130, 133), 15, 18)
    return m


def _torii(lod):
    m = Mesh()
    vermilion, dark = (194, 48, 28), (45, 39, 32)
    steps = (20, 14, 10)[lod]

    def post(x, z, height, radius, lean=0.):
        sections = [(0., 1.08), (.5, 1.03), (height * .45, 1.), (height, .81)]
        rings = []
        for y, scale in sections:
            rings.append([np.array((x + lean * y / height + radius * scale * math.cos(i * TAU / steps), y,
                                    z + radius * scale * math.sin(i * TAU / steps))) for i in range(steps)])
        for j in range(len(rings) - 1):
            for i in range(steps):
                k = (i + 1) % steps
                q = [rings[j][i], rings[j][k], rings[j + 1][k], rings[j + 1][i]]
                _quad(m, q, vermilion, 11, np.mean(q, axis=0) - (x, np.mean(q, axis=0)[1], z))
        for i in range(steps):
            m.triangle((x + lean, height, z), rings[-1][(i + 1) % steps], rings[-1][i], vermilion, 11)
            m.triangle((x, 0, z), rings[0][i], rings[0][(i + 1) % steps], dark, 11)
        m.beam((x, .05, z), (x + lean * .45 / height, .45, z), radius * 1.09, (55, 45, 35), 11, steps)

    for sign in (-1, 1):
        x = sign * 7.65
        post(x, 0., 13.4, 1.55, -sign * .58)
        for z in (-5.45, 5.45):
            post(x, z, 6.15, .65)
        for y in (3.9, 5.45):
            m.box((x, y, 0.), (1.0, .74, 13.2), vermilion)
        # Small sleeve roofs and penetrating ties are part of the Ryobu
        # structure; they matter to the silhouette from oblique approaches.
        for z in (-5.45, 5.45):
            m.box((x, 6.35, z), (2.1, .32, 2.15), dark, 12)
    m.box((0., 10.5, 0.), (21.4, 1.45, 1.85), vermilion)
    m.box((0., 12.5, 0.), (1.25, 3.3, 1.4), vermilion)
    # The central plaque has no letters or borrowed artwork.
    for z in (-1.09, 1.09):
        m.box((0., 12.54, z), (1.82, 2.40, .15), (145, 104, 43), 16)
        m.box((0., 12.54, z * 1.085), (1.49, 2.04, .075), (47, 57, 53), 11)
    xs = np.linspace(-12.1, 12.1, (40, 26, 18)[lod])
    cap = [(float(x), 14.15 + 1.14 * (abs(x) / 12.1) ** 2.65, 0.) for x in xs]
    _ribbon(m, cap, 2.8, 1.25, vermilion)
    roof = [(x, y + .95, z) for x, y, z in cap]
    _ribbon(m, roof, 3.75, .42, dark, 12)
    _ribbon(m, [(x, y + .28, z) for x, y, z in roof], 1.15, .25, (60, 52, 39), 12)
    return m


def _cubes(lod):
    m = Mesh()
    yellow, roof, trim = (224, 181, 44), (112, 124, 135), (228, 227, 215)
    # A body diagonal is vertical: three yellow faces look down and three
    # slate faces look up. A 45-degree turn about a single axis is not enough.
    matrix = np.array([[1 / math.sqrt(2), -1 / math.sqrt(2), 0.],
                       [1 / math.sqrt(3), 1 / math.sqrt(3), 1 / math.sqrt(3)],
                       [-1 / math.sqrt(6), -1 / math.sqrt(6), 2 / math.sqrt(6)]])
    assert np.linalg.det(matrix) > .999

    def house(x, z, size=7.7, high=7.15):
        center = np.array((x, high + size * math.sqrt(3) * .5, z))
        bottom = high
        m.beam((x, .0, z), (x, bottom + 1.0, z), 2.0, (156, 147, 128), 11, 6)
        for dx, dz in [(-.85, -1.76), (.85, -1.76)]:
            m.box((x + dx, 2.1, z + dz), (.78, 3.3, .10), (55, 60, 55), 15)
        for axis in range(3):
            u, v = (axis + 1) % 3, (axis + 2) % 3
            for sign in (-1, 1):
                normal = matrix[:, axis] * sign

                def p(a, b, offset=.0):
                    q = np.zeros(3)
                    q[axis] = sign * (size * .5 + offset)
                    q[u], q[v] = a * size, b * size
                    return matrix @ q + center

                _quad(m, [p(-.5, -.5), p(.5, -.5), p(.5, .5), p(-.5, .5)],
                      yellow if sign < 0 else roof, 11 if sign < 0 else 12, normal)
                # a+b is horizontal in world space after the diagonal-up
                # rotation. These glazing bands wrap the corners at a common
                # floor height, as the actual cube houses' windows do.
                square = [np.array(q) for q in [(-.49, -.49), (.49, -.49), (.49, .49), (-.49, .49)]]
                bands = [(-.10, .26)] if sign < 0 else [(-.66, -.33), (.50, .68)]
                for low, high in bands:
                    panel = _clip_polygon(_clip_polygon(square, (1., 1.), high), (-1., -1.), -low)
                    _polygon(m, [p(a, b, .038) for a, b in panel], trim, 11, normal)
                    glass = _clip_polygon(_clip_polygon(square, (1., 1.), high - .036), (-1., -1.), -low - .036)
                    # Separating glass panes retains the wide structural
                    # cross, instead of an uninterrupted dark wrap.
                    for pa, pb in [(-.92, -.31), (-.26, .26), (.31, .92)]:
                        pane = _clip_polygon(_clip_polygon(glass, (1., -1.), pb), (-1., 1.), -pa)
                        _polygon(m, [p(a, b, .046) for a, b in pane], GLASS, 15, normal)
                if sign < 0:
                    _quad(m, [p(-.045, -.5, .055), p(.045, -.5, .055),
                              p(.045, .5, .055), p(-.045, .5, .055)], trim, 11, normal)
                if lod < 2:
                    for a, b in [((-.5, -.5), (.5, -.5)), ((.5, -.5), (.5, .5)), ((.5, .5), (-.5, .5)), ((-.5, .5), (-.5, -.5))]:
                        m.beam(p(*a, .035), p(*b, .035), .085, trim, 11, 4)
                    if lod == 0 and sign > 0:
                        for value in np.linspace(-.36, .36, 6):
                            m.beam(p(value, -.47, .012), p(value, .47, .012), .024, (88, 100, 112), 12, 4)

    # Hand-selected connected courtyard rows model the dense Blaak Forest,
    # leaving the central promenade open instead of filling a rectangular grid.
    rows = [(-22., [-33., -22., -11., 0., 11., 22., 33.]),
            (-11., [-38.5, -27.5, -16.5, 16.5, 27.5, 38.5]),
            (0., [-33., -22., -11., 11., 22., 33.]),
            (11., [-38.5, -27.5, -16.5, 16.5, 27.5, 38.5]),
            (22., [-33., -22., -11., 0., 11., 22., 33.]),
            (33., [-27.5, -16.5, -5.5, 5.5, 16.5, 27.5])]
    m.box((0., .35, 5.5), (89., .7, 69.), (164, 158, 141), 8)
    for z, xs in rows:
        for x in xs:
            house(x, z)
        m.box(((min(xs) + max(xs)) * .5, 5.5, z), (max(xs) - min(xs) + 5., 1.4, 4.0), (154, 146, 129), 11)
    for x in (-35.5, 35.5):
        m.box((x, 5.5, 5.5), (4., 1.4, 60.), (154, 146, 129), 11)
    # Broad steps connect the raised passages to the square.
    for side in (-1, 1):
        for i in range(12):
            m.box((side * 8., .23 * (i + 1), -30. - i * .65), (4., .46 * (i + 1), .68), STONE, 8)
    return m


def _ddp(lod):
    m = Mesh()
    steps = (12, 8, 5)[lod]

    def lobe(outline, center, height, gradient, roofcolor, garden=.68, triangular=False):
        contour = _catmull(outline, steps)
        center = np.asarray(center)
        garden_boundary = _catmull([(-42., 94.), (13., 80.), (-22., 26.)], 30) if triangular else []
        # Radial sections create a rolled aluminum edge and an undercut
        # cantilever, rather than a stack of intersecting solid ellipsoids.
        profile = [(0.62, 0.), (.65, .08), (.73, .32), (.90, .51),
                   (1.00, .63), (1.015, .75), (1., .85), (.96, .93),
                   (.87, 1.), (garden, .95)]
        rings = []
        for index, (scale, y) in enumerate(profile):
            ring = []
            for p in contour:
                xz = center + (p - center) * scale
                if triangular and index == len(profile) - 1:
                    direction = p - center
                    for a, b in zip(garden_boundary, garden_boundary[1:] + garden_boundary[:1]):
                        matrix = np.column_stack((direction, a - b))
                        if abs(np.linalg.det(matrix)) < 1e-8:
                            continue
                        t, s = np.linalg.solve(matrix, a - center)
                        if t > 0 and 0 <= s <= 1:
                            xz = center + direction * t
                            break
                h = height + gradient * (xz[1] - center[1])
                ring.append(np.array((xz[0], 1.2 + h * y, xz[1])))
            rings.append(ring)
        for k in range(len(rings) - 1):
            for i in range(len(contour)):
                j = (i + 1) % len(contour)
                q = [rings[k][i], rings[k][j], rings[k + 1][j], rings[k + 1][i]]
                n = np.mean(q, axis=0) - (center[0], height * .45, center[1])
                variation = ((i * 7 + k * 11) % 5) - 2
                color = tuple(c + variation for c in SILVER)
                _quad(m, q, color, 16, n)
        # Recessed, planted roof inserts are opaque surfaces, not open holes.
        # Their asymmetry follows the boundary instead of generic circles.
        top = rings[-1]
        middle = np.array((center[0], height * .85 + 1.2, center[1]))
        for i in range(len(contour)):
            j = (i + 1) % len(contour)
            q = [middle, top[i], top[j]]
            if np.cross(q[1] - q[0], q[2] - q[0])[1] < 0:
                q.reverse()
            m.triangle(*q, roofcolor, 12)
        for i in range(len(contour)):
            j = (i + 1) % len(contour)
            p, q = rings[1][i], rings[1][j]
            _ribbon(m, [p + (0, .15, 0), q + (0, .15, 0)], .22, .32, WARM, 94)
            # Panel seams follow the rolled exterior; a low-contrast seam
            # every second sample reads as cladding without drawing a cage.
            if lod == 0 and i % 2 == 0:
                for k in range(2, len(rings) - 1):
                    a, b = rings[k][i], rings[k + 1][i]
                    n = np.array((a[0] - center[0], 2., a[2] - center[1]))
                    n /= np.linalg.norm(n)
                    m.beam(a + n * .03, b + n * .03, .035, (136, 147, 155), 16, 4)
        for k in (2, 5, 7):
            if lod < 2:
                loop = rings[k] + [rings[k][0]]
                for a, b in zip(loop, loop[1:]):
                    m.beam(a, b, .035, (145, 157, 164), 16, 4)

    # Local +z follows the principal site axis. The tapered southern hall,
    # broad northern hall, and slender east wing retain separate roof gardens.
    lobe([(-57., -136.), (-76., -112.), (-71., -53.), (-49., -9.),
          (-13., 0.), (9., -29.), (-10., -74.), (-36., -128.)],
         (-37., -66.), 19., .065, (59, 67, 56), .77)
    lobe([(-44., 2.), (-63., 29.), (-64., 80.), (-33., 116.),
          (8., 122.), (36., 103.), (38., 59.), (18., 21.), (-5., 3.)],
         (-13., 64.), 31., .025, (59, 66, 54), .59, True)
    lobe([(18., 36.), (50., 38.), (65., 69.), (62., 114.),
          (36., 130.), (6., 127.), (16., 114.), (31., 99.), (34., 67.)],
         (35., 89.), 18., -.02, (69, 75, 59), .66)
    # Smooth raised saddle joins the halls at the narrow central waist.
    path = [(-39., 16., -23.), (-22., 21., -5.), (-17., 24., 15.), (-26., 25., 30.)]
    _ribbon(m, _catmull(path, steps, False), 29., 2.8, SILVER, 16)
    _ribbon(m, [(-54., 15.2, -113.), (-39., 17.8, -68.), (-25., 21.2, -20.)], .65, .28, SILVER, 16)
    # A low, glazed northern support and exposed V struts preserve the
    # characteristic bright void underneath the large northern cantilever.
    m.box((-12., 4.4, 85.), (41., 7., 34.), GLASS, 15)
    for x in (-37., -12., 13.):
        for z in (88., 109.):
            m.beam((x - 7., 11.8, z), (x, 1.4, z), .9, STONE, 11, 6)
            m.beam((x, 1.4, z), (x + 7., 11.8, z), .9, STONE, 11, 6)
    promenade = _catmull([(-70., .3, -133.), (-91., .3, -71.), (-81., .3, -3.),
                          (-83., .3, 73.), (-52., .3, 131.), (10., .3, 142.), (58., .3, 126.)], steps, False)
    _ribbon(m, promenade, 6., .45, (181, 177, 165), 8)
    _ribbon(m, [(x, y + .4, z) for x, y, z in promenade], .2, .18, WARM, 94)
    return m


def _setas(lod):
    m = Mesh()
    wood, edge = (219, 197, 153), (234, 216, 181)
    # Six smooth lobes define the connected canopy. The grid is made of
    # actual fins, so sky and plaza are visible through its cells.
    centers = [(-57., 13., 18., 18.), (-26., 11., 26., 22.),
               (14., 11., 21., 20.), (55., 9., 18., 20.),
               (-43., -25., 22., 19.), (-7., -21., 27., 21.)]

    def field(x, z):
        lobes = min(((x - cx) / rx) ** 2 + ((z - cz) / rz) ** 2 for cx, cz, rx, rz in centers)
        eastern_neck = ((x - 35.) / 25.) ** 2 + ((z - 11.) / 7.8) ** 2
        return min(lobes, eastern_neck)

    def bounds(x, z):
        top = 21.2 + 2.5 * math.sin((x + 26.) / 30.) + 1.9 * math.cos(z / 16.)
        # Broad flared stems flow into the canopy rather than meeting flat
        # discs. Their lower fin edges follow six individual bowl profiles.
        d = min(math.sqrt(((x - cx) / 14.) ** 2 + ((z - cz) / 12.) ** 2)
                for cx, cz, _, _ in centers)
        bottom = max(5.1, top - 2.1 - 17.2 * math.exp(-d * d * 2.2))
        return bottom, top

    spacing = (1.65, 2.4, 3.5)[lod]
    step = (1.2, 1.8, 2.5)[lod]
    thickness = (.17, .24, .35)[lod]
    for axis in (0, 1):
        extent = 76. if axis == 0 else 48.
        other = 48. if axis == 0 else 76.
        for fixed in np.arange(-extent, extent + .01, spacing):
            values = np.arange(-other, other + step, step)
            inside = [field(fixed, u) <= 1. if axis == 0 else field(u, fixed) <= 1. for u in values]
            valid = [a and b for a, b in zip(inside, inside[1:])]
            for index, (u0, u1) in enumerate(zip(values, values[1:])):
                coords = [(fixed, float(u)) if axis == 0 else (float(u), fixed) for u in (u0, u1)]
                if not valid[index]:
                    continue
                (x0, z0), (x1, z1) = coords
                b0, t0 = bounds(x0, z0)
                b1, t1 = bounds(x1, z1)
                lateral = np.array((thickness * .5, 0., 0.)) if axis == 0 else np.array((0., 0., thickness * .5))
                a, b = np.array((x0, b0, z0)), np.array((x1, b1, z1))
                c, d = np.array((x1, t1, z1)), np.array((x0, t0, z0))
                _quad(m, [a + lateral, b + lateral, c + lateral, d + lateral], wood, 11, lateral)
                _quad(m, [a - lateral, b - lateral, c - lateral, d - lateral], wood, 11, -lateral)
                _quad(m, [d - lateral, c - lateral, c + lateral, d + lateral], edge, 11, (0, 1, 0))
                _quad(m, [a - lateral, b - lateral, b + lateral, a + lateral], (195, 169, 123), 11, (0, -1, 0))
                if index == 0 or not valid[index - 1]:
                    _quad(m, [a - lateral, a + lateral, d + lateral, d - lateral], wood, 11, a - b)
                if index + 1 == len(valid) or not valid[index + 1]:
                    _quad(m, [b - lateral, b + lateral, c + lateral, c - lateral], wood, 11, b - a)
    # The market plaza is elevated five metres. Its central opening and
    # broad stairs retain the shaded public-space silhouette.
    m.box((-20., 2.35, -7.), (86., 4.7, 54.), (174, 157, 130), 11)
    m.box((-20., 4.9, -7.), (90., .4, 58.), (215, 208, 188), 8)
    for x in np.arange(-59., 23., 8.):
        m.box((x, 2.4, -34.08), (6.8, 3.5, .18), GLASS, 15)
    for i in range(14):
        h = 4.9 * (i + 1) / 14
        m.box((-20., h * .5, -43. + i * .60), (27., h, .62), STONE, 8)
    for cx, cz, _, _ in centers:
        # White cores are largely hidden by the flared lattice at close range.
        m.beam((cx, 4.8 if cx < 25 else 0., cz), (cx, 17., cz), 2.9, (214, 198, 167), 11, (12, 10, 8)[lod])
    walkway = [(-55., 12.), (-61., 20.), (-48., 23.), (-29., 13.),
               (-14., -4.), (-6., -19.), (-22., -24.), (-40., -17.),
               (-50., -24.), (-47., -35.), (-35., -31.), (-25., -15.),
               (-10., -4.), (7., 2.), (19., 18.), (29., 20.), (32., 10.),
               (19., 1.), (4., -5.), (-9., -10.), (-19., 3.), (-24., 16.),
               (-26., 25.), (-35., 23.), (-42., 14.), (-55., 12.)]
    spline = _catmull(walkway, (6, 4, 3)[lod], False)
    points = [(x, bounds(x, z)[1] + .40, z) for x, z in spline]
    _ribbon(m, points, 2.8, .45, edge, 11)
    for sign in (-1, 1):
        rail = []
        for i, p in enumerate(points):
            p = np.asarray(p)
            t = np.asarray(points[min(i + 1, len(points) - 1)]) - np.asarray(points[max(i - 1, 0)])
            s = np.array((-t[2], 0., t[0])); s /= np.linalg.norm(s)
            rail.append(p + s * 1.25 * sign + (0, .9, 0))
        for i, (a, b) in enumerate(zip(rail, rail[1:])):
            m.beam(a, b, .065, (203, 187, 156), 16, 4)
            if i % (2 if lod == 0 else 3) == 0:
                m.beam(a - (0, .88, 0), a, .045, (203, 187, 156), 16, 4)
    return m


def _louvre(lod):
    m = Mesh()
    # The museum is a group of white volumes with open alleys and waterfront
    # courtyards. Their heights remain below the floating canopy's edge.
    volumes = [(-64,-42,23,21,8),(-38,-48,24,26,10),(-10,-47,27,26,8),
               (21,-51,28,22,10),(53,-39,26,23,8),(-69,-14,19,26,8),
               (-43,-16,23,29,11),(-14,-13,26,29,9),(19,-15,27,24,11),
               (51,-10,25,28,8),(-64,18,26,23,8),(-33,21,25,27,10),
               (-3,23,28,27,9),(30,19,27,31,10),(61,23,24,22,9),
               (-45,48,25,22,8),(-16,51,27,22,9),(16,53,26,23,8),
               (46,51,26,22,10),(-22,-79,33,20,10),(16,-81,34,20,8),
               (52,-70,28,26,7),(-60,-68,28,22,8)]
    # The museum datum is +4 m; the plinth must extend below sea level.
    m.box((0., -3., -8.), (159., 6., 164.), (199, 195, 178), 17)
    for k, (x, z, w, d, h) in enumerate(volumes):
        m.box((x, h * .5, z), (w, h, d), (232 + k % 4, 232 + k % 4, 220 + k % 4), 17)
        m.box((x, h + .15, z), (w + .18, .30, d + .18), (211, 211, 200), 12)
        if lod < 2:
            for xx in (-w * .5 + 2.6, w * .5 - 2.6):
                m.box((x + xx, 2.4, z - d * .5 - .04), (1.75, 4.1, .12), GLASS, 15)
    for x, z in [(-54,-54),(-54,54),(54,-54),(54,54)]:
        m.box((x, 11.7, z), (5., 23.4, 5.), WHITE, 17)

    radius = 90.

    def height(x, z):
        # A shallow spherical cap, 36 m above ground at its crown. Its rolled
        # edge is separate, preserving the visual gap above gallery roofs.
        sphere_radius = 360.
        return 36. - sphere_radius + math.sqrt(max(0., sphere_radius ** 2 - x * x - z * z))

    def clip(a, b):
        d = b - a
        aa = d[0] ** 2 + d[2] ** 2
        bb = 2 * (a[0] * d[0] + a[2] * d[2])
        cc = a[0] ** 2 + a[2] ** 2 - (radius - .3) ** 2
        discriminant = bb * bb - 4 * aa * cc
        if discriminant <= 0:
            return None
        root = math.sqrt(discriminant)
        lo, hi = max(0., (-bb - root) / (2 * aa)), min(1., (-bb + root) / (2 * aa))
        if hi - lo < 1e-7:
            return None
        return a + lo * d, a + hi * d

    count = (180, 120, 80)[lod]
    for yoffset, r, thick, color in [(0.,90.,.38,(177,187,192)),(-1.2,89.9,.32,(123,135,143))]:
        ring = [(r * math.cos(i * TAU / count), height(r, 0.) + yoffset, r * math.sin(i * TAU / count)) for i in range(count + 1)]
        for a, b in zip(ring, ring[1:]):
            m.beam(a, b, thick, color, 16, 6)

    # Layered eight-point stars give the roof real openings and patterned
    # edges. LOD coarsens the motif while retaining an open silver canopy.
    spacing = (7.6, 10., 14.)[lod]
    star_radius = spacing * .74
    layers = 2 if lod < 2 else 1
    for layer in range(layers):
        angle = layer * math.pi / 8
        color = (179, 190, 196) if layer == 0 else (139, 151, 159)
        for ix in range(-int(90 / spacing) - 1, int(90 / spacing) + 2):
            for iz in range(-int(90 / spacing) - 1, int(90 / spacing) + 2):
                x, z = ix * spacing + layer * spacing * .5, iz * spacing + layer * spacing * .5
                if x * x + z * z > (radius + star_radius) ** 2:
                    continue
                star = []
                for i in range(16):
                    a = angle + i * TAU / 16
                    r = star_radius if i % 2 == 0 else star_radius * .45
                    xx, zz = x + r * math.cos(a), z + r * math.sin(a)
                    star.append(np.array((xx, height(xx, zz) - layer * .8, zz)))
                for i in range(16):
                    a, b = star[i], star[(i + 1) % 16]
                    segment = clip(a, b)
                    if segment is None:
                        continue
                    a, b = segment
                    tangent = b - a
                    side = np.array((-tangent[2], 0., tangent[0]))
                    side /= np.linalg.norm(side)
                    side *= (.72, .97, 1.50)[lod] * .5
                    panel = [a - side, a + side, b + side, b - side]
                    # Two-sided laminae preserve visible openings while
                    # avoiding internal end caps at every lattice junction.
                    _quad(m, panel, color, 16, (0, 1, 0))
                    _quad(m, panel, color, 16, (0, -1, 0))
                if lod == 0 and (ix + iz) % 2 == 0:
                    for i in range(0, 16, 4):
                        segment = clip(star[i], star[(i + 6) % 16])
                        if segment is not None:
                            m.beam(*segment, .12, (194, 202, 204), 16, 4)
    # Long structural chords bind the repeating panels and the circular rim.
    for a in [i * math.pi / 8 for i in range(8)]:
        points = [(r * math.cos(a), height(r * math.cos(a), r * math.sin(a)) - 1.1, r * math.sin(a))
                  for r in np.linspace(-90., 90., (37, 25, 17)[lod])]
        for p, q in zip(points, points[1:]):
            m.beam(p, q, (.19, .25, .39)[lod], (148, 161, 170), 16, 6)
    return m


BUILDERS = {"dongdaemun-design-plaza": _ddp, "rotterdam-cube-houses": _cubes,
            "seville-setas": _setas, "lotus-temple": _lotus,
            "louvre-abu-dhabi": _louvre, "miyajima-torii": _torii}


def build(name, lod):
    if lod not in (0, 1, 2):
        raise ValueError("the catalog has exactly three LOD levels")
    return BUILDERS[name](lod)
