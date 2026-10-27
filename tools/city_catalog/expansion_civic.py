"""Original static civic landmarks, authored in metres from architectural references.

OSM contributes placement and footprint measurements, not imported geometry.
Terrain datums are the medians of sixteen surveyed perimeter samples; fixed
values keep each authored foundation consistent across receiving surface cells.
"""
import math

import numpy as np

from common import Mesh

WHITE = (229, 230, 220)
STONE = (202, 192, 165)
DARK = (45, 55, 58)
GLASS = (86, 112, 126)
GOLD = (204, 158, 57)
WARM = (242, 182, 94)
TEAL = (70, 131, 137)


def _patch(half, base, feather=8.0):
    return dict(center_xz=[0.0, 0.0], half_extent=list(map(float, half)),
                feather_m=float(feather), sample_clearance_m=15.0,
                fixed_base_geo=float(base))


def _santiago_terrace_patches():
    # Inscribed strips keep the complete correction, including its feather,
    # inside the podium. The nearest surveyed pond boundary is 24.134m away.
    feather = .1
    radius = 23.0 - math.sqrt(2.0)*feather
    strips = [(float(z), .5) for z in range(-22, 23)]
    strips.extend([(-22.65, .15), (22.65, .15)])
    patches = []
    for z, half_z in strips:
        half_x = math.sqrt(radius*radius-(abs(z)+half_z)**2)
        patch = _patch((half_x, half_z), 983.1224060058594, feather)
        patch['center_xz'] = [0.0, z]
        patches.append(patch)
    return patches


ENTRIES = [
    dict(name="brasilia-cathedral", city="Brasilia", lat=-15.7983445, lon=-47.8755465,
         heading=0.0, base=1079.3665771484375, suppress_buildings=True,
         suppress_roads=True, suppress_trees=True, suppress_bbox=[[-44., -44.], [44., 44.]],
         terrain_flatten=[_patch((44, 44), 1079.3665771484375)],
         references=["https://catedral.org.br/institucional/historia/",
                     "https://catedral.org.br/comunicacao-da-catedral/a-genialidade-de-joaquim-cardozo-na-estrutura-da-catedral/",
                     "https://www.openstreetmap.org/way/66379902"],
         description="Sixteen white curved concrete blades pinch into a crown above blue glass, with a circular foundation, still reflecting pool, and steady warm rib lighting.",
         source_audit=dict(center_ground_m=1078.5120849609375,
                           perimeter_median_m=1079.3665771484375,
                           pool_source_peak_above_base_m=.445133,
                           pool_fit="Production water-datum audit: mapped reflecting-pool boundary peaks 0.445m above base; authored surface 0.65m and rim 0.80m cover it without moving the cathedral.",
                           dimensions="Official structural account: 16 columns, 30m high, 70m foundation diameter; the underground nave is not a raised drum."),
         capture=dict(distance=128., azimuth=215., elevation=19., target_height=12., time_utc="2026-03-20T20:45:00Z")),
    dict(name="santiago-bahai-temple", city="Santiago", lat=-33.47637685, lon=-70.51172385,
         heading=0.0, base=983.1224060058594, suppress_buildings=True,
         suppress_roads=True, suppress_trees=True, suppress_bbox=[[-24., -24.], [24., 24.]],
         terrain_flatten=_santiago_terrace_patches(),
         references=["https://hp-arch.com/projects/bahai-temple-of-south-america/",
                     "https://www.hariripontarini.com/news/hariri-pontarini-architects-conceives-bahai-temple-of-south-america",
                     "https://www.openstreetmap.org/way/268450155"],
         description="Nine torqued translucent-looking shell wings gather around an oculus, with bronze entry recesses, visible panel seams, and a small hillside terrace.",
         source_audit=dict(center_ground_m=980.9346923828125,
                           perimeter_median_m=983.1224060058594,
                           podium_perimeter_min_m=973.9169921875,
                           pond_boundary_min_radius_m=24.134,
                           terrace_fit="Inscribed 1m strips and 0.1m feather remain within radius 23m, clear of the mapped pond. The radius 21.5m terrace is flat; the outer structural shoulder meets the natural uphill slope. Existing 10m foundation reaches the surveyed downhill minimum. Source pond datum outside the podium remains unchanged.",
                           dimensions="30m high nine-wing structure; 33m OSM roof footprint. The small terrace cuts the natural east-uphill grade locally."),
         capture=dict(distance=96., azimuth=260., elevation=19., target_height=12., time_utc="2026-03-20T22:30:00Z")),
    dict(name="oriente-station", city="Lisbon", lat=38.76782055, lon=-9.09907265,
         heading=4.1, base=16.291507720947266, suppress_buildings=True,
         suppress_roads=False, suppress_trees=True, suppress_bbox=[[-39., -131.], [39., 131.]],
         references=["https://prod.calatrava.com/projects/oriente-station-lisboa.html",
                     "https://www.ippatrimonio.pt/pt-pt/node/184",
                     "https://www.openstreetmap.org/way/759763392"],
         description="Four rows of branching white steel trees carry a scalloped glass canopy above eight retained railway tracks, with paired concrete undercroft arches and lit platform edges.",
         source_audit=dict(center_ground_m=16.291507720947266,
                           south_rail_ground_m=18.452516555786133,
                           north_rail_ground_m=21.386268615722656,
                           dimensions="Architect: 260m bridge structure, 78m wide, 17.07m roof grid; OSM canopy 258m by 70m, bearing 4.1deg. Platform fit assumes existing bridge clearance of 5m; source railway approaches remain in place."),
         capture=dict(distance=335., azimuth=112., elevation=24., target_height=16., time_utc="2026-03-20T17:45:00Z")),
    dict(name="monas", city="Jakarta", lat=-6.1754073, lon=106.82716715,
         heading=1.1, base=4.835268259048462, suppress_buildings=True,
         suppress_roads=True, suppress_trees=True, suppress_bbox=[[-47., -47.], [47., 47.]],
         terrain_flatten=[_patch((47, 47), 4.835268259048462)],
         references=["https://dpp.jakarta.go.id/berita/monumen-nasional-monas-simbol-perjuangan-bangsa-yang-terus-menginspirasi-generasi",
                     "https://www.jakarta.go.id/index.php/wisata-bersejarah",
                     "https://www.openstreetmap.org/way/1159084257"],
         description="A tapered marble obelisk rises from a broad stepped museum and upturned square cup to a recessed observation deck and a sculpted, steadily lit gold flame.",
         source_audit=dict(center_ground_m=2.0943586826324463,
                           perimeter_median_m=4.835268259048462,
                           dimensions="Jakarta government:132m total,45x45m cup at17m and observation level115m; footprint measured93m."),
         capture=dict(distance=280., azimuth=210., elevation=17., target_height=55., time_utc="2026-03-20T10:45:00Z")),
    dict(name="azadi-tower", city="Tehran", lat=35.6997383, lon=51.3380605,
         heading=91.7, base=1184.437255859375, suppress_buildings=True,
         suppress_roads=False, suppress_trees=True, suppress_bbox=[[-36., -24.], [36., 24.]],
         terrain_flatten=[_patch((36, 24), 1184.437255859375)],
         references=["https://amanatarchitect.com/project/shahyad-azadi-monument/",
                     "https://www.skyscrapercenter.com/tehran/azadi-tower/23652",
                     "https://www.openstreetmap.org/relation/7814369"],
         description="Four sweeping marble feet merge into a beveled tower above intersecting open arches, with turquoise inlays, woven vault ribs, and steady entry lighting.",
         source_audit=dict(center_ground_m=1184.6292724609375,
                           perimeter_median_m=1184.437255859375,
                           dimensions="CTBUH architectural height45m overrides erroneous OSM63m dome parts. OSM overall footprint70m north-south by42m east-west."),
         capture=dict(distance=115., azimuth=115., elevation=14., target_height=18., time_utc="2026-03-20T14:15:00Z")),
    dict(name="abuja-national-mosque", city="Abuja", lat=9.06019645, lon=7.48979445,
         heading=0.7, base=483.0863342285156, suppress_buildings=True,
         suppress_roads=True, suppress_trees=True, suppress_bbox=[[-39., -39.], [39., 39.]],
         terrain_flatten=[_patch((39, 39), 483.0863342285156)],
         references=["https://aim-consultants.com/projects",
                     "https://www.archnet.org/sites/703",
                     "https://www.openstreetmap.org/way/188837229",
                     "https://www.openstreetmap.org/way/564467266"],
         description="A ribbed gold main dome and four smaller domes crown a battered stone prayer hall between four fluted minarets, with open galleries and steady gold-white illumination.",
         source_audit=dict(center_ground_m=498.3421630859375,
                           perimeter_median_m=483.0863342285156,
                           dimensions="73m mapped main footprint. Authored dome40m/minarets~63m follow mapped55m shafts plus finials and the architect's photograph; unsourced120m claims contradict these proportions."),
         capture=dict(distance=205., azimuth=220., elevation=18., target_height=23., time_utc="2026-03-20T17:00:00Z")),
]


def _quad(m, points, color, material=17, normal=None):
    p = np.array(points, dtype=float)
    if normal is not None and np.dot(np.cross(p[1]-p[0], p[2]-p[0]), normal) < 0:
        p = p[::-1]
    m.quad(*p, color, material)


def _lathe(m, profile, color, material=17, n=48, center=(0, 0, 0)):
    """Closed outward-wound surface of revolution; profile is (height,radius)."""
    c = np.asarray(center)
    rings = [[c + [r*math.cos(j*math.tau/n), y, r*math.sin(j*math.tau/n)]
              for j in range(n)] for y, r in profile]
    for a, b in zip(rings, rings[1:]):
        for j in range(n):
            k = (j+1) % n
            m.quad(a[j], b[j], b[k], a[k], color, material)
    for j in range(n):
        k = (j+1) % n
        m.triangle(c+[0, profile[0][0], 0], rings[0][j], rings[0][k], color, material)
        m.triangle(c+[0, profile[-1][0], 0], rings[-1][k], rings[-1][j], color, material)


def _annulus(m, inner, outer, y, color, material=8, n=64):
    for j in range(n):
        a, b = j*math.tau/n, (j+1)*math.tau/n
        p = lambda r, t: (r*math.cos(t), y, r*math.sin(t))
        _quad(m, [p(inner, a), p(inner, b), p(outer, b), p(outer, a)], color, material, (0, 1, 0))


def _rect_loft(m, profile, color, material=17, center=(0, 0, 0)):
    c = np.asarray(center)
    rings = [[c+[sx*x, y, sz*z] for sx, sz in [(-1, -1), (-1, 1), (1, 1), (1, -1)]]
             for y, x, z in profile]
    for a, b in zip(rings, rings[1:]):
        for j in range(4):
            k = (j+1) % 4
            m.quad(a[j], a[k], b[k], b[j], color, material)
    m.quad(*rings[0][::-1], color, material)
    m.quad(*rings[-1], color, material)


def _path(m, points, radius, color, material=16, sides=6):
    # One connected tube avoids hidden caps at every sampled curve segment.
    p = np.asarray(points, dtype=float)
    p = p[np.r_[True, np.linalg.norm(np.diff(p, axis=0), axis=1) > 1e-7]]
    if len(p) < 2:
        return
    rings = []
    previous_side = None
    for i, center in enumerate(p):
        axis = p[min(i+1,len(p)-1)]-p[max(i-1,0)]
        axis /= np.linalg.norm(axis)
        side = (previous_side-axis*np.dot(previous_side,axis)
                if previous_side is not None else np.zeros(3))
        if np.linalg.norm(side) < 1e-6:
            side = np.cross(axis, [0,1,0] if abs(axis[1]) < .9 else [1,0,0])
        side /= np.linalg.norm(side)
        previous_side = side
        up = np.cross(axis, side)
        rings.append([center+radius*(math.cos(j*math.tau/sides)*side+
                                    math.sin(j*math.tau/sides)*up) for j in range(sides)])
    for a,b in zip(rings,rings[1:]):
        for j in range(sides):
            k=(j+1)%sides
            m.quad(a[j],a[k],b[k],b[j],color,material)
    for j in range(sides):
        k=(j+1)%sides
        m.triangle(p[0],rings[0][k],rings[0][j],color,material)
        m.triangle(p[-1],rings[-1][j],rings[-1][k],color,material)


def _ring(m, center, radius, thickness, color, material, n):
    x, y, z = center
    pts = [(x+radius*math.cos(t), y, z+radius*math.sin(t)) for t in np.linspace(0, math.tau, n+1)]
    _path(m, pts, thickness, color, material, 5)


def _brasilia(lod):
    m = Mesh()
    n = (96, 64, 48)[lod]
    _lathe(m, [(-6, 42), (-.22, 42), (-.08, 41.5)], (170, 166, 153), 17, n)
    _annulus(m, 34.8, 41, .65, (44, 77, 84), 1, n)
    _annulus(m, 41, 42, .80, WHITE, 17, n)
    for radius, normal in [(41, -1), (42, 1)]:
        for j in range(n):
            a, b = math.tau*j/n, math.tau*(j+1)/n
            p = lambda t, y: (radius*math.cos(t), y, radius*math.sin(t))
            _quad(m, [p(a, -.08), p(a, .80), p(b, .80), p(b, -.08)],
                  WHITE, 17, (normal*math.cos(a), 0, normal*math.sin(a)))
    _ring(m, (0, .71, 0), 34.8, .10, WHITE, 17, n)
    _lathe(m, [(-.15, 34.9), (.27, 34.9), (.35, 34.5)], WHITE, 17, n)
    # Each column is a solid sickle, wider in its radial section than tangentially.
    ny = (28, 18, 12)[lod]
    def radius(t):
        return 7.15+27.1*(1-t)**3+3.0*t**7
    for rib in range(16):
        theta = math.tau*rib/16
        radial = np.array([math.cos(theta), 0, math.sin(theta)])
        tangent = np.array([-math.sin(theta), 0, math.cos(theta)])
        rings = []
        for i in range(ny+1):
            t = i/ny
            c = radial*radius(t)+[0, .28+30.0*t, 0]
            half_w = .11+.69*math.sin(math.pi*t)**.65+.23*(1-t)
            deep = .15+2.0*math.sin(math.pi*t)**1.2
            rings.append([c+radial*deep, c+tangent*half_w,
                          c-radial*.42*deep, c-tangent*half_w])
        for a, b in zip(rings, rings[1:]):
            for j in range(4):
                k = (j+1) % 4
                _quad(m, [a[j], b[j], b[k], a[k]], WHITE, 17,
                      (a[j]+a[k])/2-(a[0]+a[2])/2)
        m.quad(*rings[0][::-1], WHITE, 17)
        m.quad(*rings[-1], WHITE, 17)
        pts = [radial*(radius(t)+.12)+tangent*.18+[0, .3+30*t, 0]
               for t in np.linspace(.02, .96, ny+1)]
        _path(m, pts, .095, WARM, 94, (6, 5, 4)[lod])
        # Blue glazing fills the lower hyperboloid; the crown tips remain open.
        panes = (7, 4, 3)[lod]
        levels = (18, 12, 8)[lod]
        def pane(u, v):
            t = .70*v
            a = theta+.031+(math.tau/16-.062)*u
            r = radius(t)-.42
            return np.array([r*math.cos(a), .22+30*t, r*math.sin(a)])
        for i in range(levels):
            for j in range(panes):
                u, v = j/panes, i/levels
                p = [pane(u, v), pane(u, v+1/levels),
                     pane(u+1/panes, v+1/levels), pane(u+1/panes, v)]
                tint = [(58, 93, 126), (65, 100, 119), (46, 74, 108)][(rib+j+i//3)%3]
                _quad(m, p, tint, 15, np.mean(p, axis=0)*[1, 0, 1])
        for j in range(panes+1):
            _path(m, [pane(j/panes, v)+radial*.04 for v in np.linspace(0, 1, levels+1)], .035, DARK, 16, 4)
        if lod < 2:
            for i in range(1, levels):
                _path(m, [pane(u, i/levels)+radial*.05 for u in np.linspace(0, 1, panes+1)], .025, DARK, 16, 4)
    _ring(m, (0, 21.3, 0), radius(.7), .22, WHITE, 17, n)
    m.beam((0, 22, 0), (0, 32.7, 0), .14, (72, 67, 58), 16, 6)
    m.beam((-1.7, 31.3, 0), (1.7, 31.3, 0), .12, (72, 67, 58), 16, 6)
    return m


def _santiago(lod):
    m = Mesh()
    n = (90, 63, 45)[lod]
    _lathe(m, [(-10, 23), (-.3, 23), (.0, 22.6)], (166, 159, 142), 17, n)
    _lathe(m, [(0, 21.5), (.28, 21.5), (.28, 20.5), (.57, 20.5), (.57, 19.5), (.85, 19.5)], STONE, 8, n)
    _lathe(m, [(.8, 14.4), (4.0, 14.4)], (102, 86, 65), 11, n)
    nu, nv = (24, 18) if lod == 0 else (16, 12) if lod == 1 else (10, 8)
    # Leaf-like wings rotate as they ascend. The fold has independent curvature
    # across its width; a surface of revolution would lose the nine-wing form.
    def point(k, u, v, inner=False):
        a = math.tau*k/9+.36*v+.20*math.sin(math.tau*v) + .318*u
        y = 1.0+29.0*v
        r = 2.0+12.0*max(0, math.cos(math.pi*v/2))**.72+2.0*math.sin(math.pi*v)
        r += 2.8*math.sin(math.pi*v)*(1-u*u)
        y += 2.1*(1-u*u)*math.sin(math.pi*v)
        if inner:
            r -= .28
        return np.array([r*math.cos(a), y, r*math.sin(a)])
    for k in range(9):
        for i in range(nv):
            for j in range(nu):
                u, v = -1+2*j/nu, i/nv
                p = [point(k,u,v),point(k,u,v+1/nv),point(k,u+2/nu,v+1/nv),point(k,u+2/nu,v)]
                shade = (j*3+i*7+k)%6
                tint = (226-shade*2, 221-shade*2, 203-shade*2)
                _quad(m, p, tint, 17, np.mean(p,axis=0)*[1,0,1])
                q = [point(k,u,v,True),point(k,u,v+1/nv,True),point(k,u+2/nu,v+1/nv,True),point(k,u+2/nu,v,True)]
                _quad(m, q, (220,194,140),17,-np.mean(q,axis=0)*[1,0,1])
        for side in [-1, 1]:
            for i in range(nv):
                v=i/nv
                _quad(m,[point(k,side,v),point(k,side,v+1/nv),point(k,side,v+1/nv,True),point(k,side,v,True)],STONE,17)
            _path(m,[point(k,side,v) for v in np.linspace(0,1,nv+1)],.07,(215,208,181),94,(6,5,4)[lod])
        for i in range(nv):
            v=i/nv
            p=[point(k,1,v)*[.997,1,.997],point(k,1,v+1/nv)*[.997,1,.997],
               point(k+1,-1,v+1/nv)*[.997,1,.997],point(k+1,-1,v)*[.997,1,.997]]
            _quad(m,p,(73,92,99),15,np.mean(p,axis=0)*[1,0,1])
        for v in [0, 1]:
            for j in range(nu):
                u=-1+2*j/nu
                _quad(m,[point(k,u,v),point(k,u+2/nu,v),point(k,u+2/nu,v,True),point(k,u,v,True)],STONE,17)
        # Fine bronze joints follow the same curved skin, never float as a net.
        seam_step = (3, 4, 5)[lod]
        for i in range(seam_step,nv,seam_step):
            _path(m,[point(k,u,i/nv)*[1.001,1,1.001] for u in np.linspace(-1,1,nu+1)],.022,(157,148,124),16,4)
        for u in [-.65, 0, .65] if lod == 0 else [0]:
            _path(m,[point(k,u,v)*[1.001,1,1.001] for v in np.linspace(0,1,nv+1)],.025,(157,148,124),16,4)
        a = math.tau*(k+.5)/9
        x,z=14.4*math.cos(a),14.4*math.sin(a)
        door = Mesh();door.box((0,2.1,0),(2.8,3.5,.22),DARK,15)
        door.box((0,3.94,.1),(3.1,.12,.35),WARM,94)
        ca,sa=math.cos(a-math.pi/2),math.sin(a-math.pi/2)
        door.transform([[ca,0,-sa,x],[0,1,0,.85],[sa,0,ca,z],[0,0,0,1]])
        m.merge(door)
    _ring(m,(0,30.0,0),2.03,.16,(210,179,119),94,(36,27,18)[lod])
    _lathe(m,[(29.82,2.0),(29.92,2.0)],(77,91,94),15,(36,27,18)[lod])
    return m


_ORIENTE_Z = [-145., -130., -110., -85., -60., -40., -20., 0., 20., 40., 60., 85., 110., 130., 145.]
_ORIENTE_GROUND = [
    [17.145,18.142,19.209,20.375], [16.994,17.846,18.827,19.899],
    [16.553,17.261,18.237,19.202], [15.893,16.632,17.668,18.584],
    [15.199,16.111,17.365,18.402], [14.913,15.867,17.238,18.350],
    [14.830,15.725,17.112,18.278], [14.725,15.600,17.069,18.354],
    [14.542,15.498,17.283,18.904], [14.542,15.759,17.979,20.002],
    [15.221,16.704,19.243,21.196], [16.856,18.621,20.777,21.746],
    [18.748,20.348,21.343,21.381], [19.997,21.034,21.457,21.162],
    [20.675,21.267,21.440,21.089],
]


def _oriente_ground(x, z):
    row = [np.interp(z, _ORIENTE_Z, np.array(_ORIENTE_GROUND)[:, j]) for j in range(4)]
    return float(np.interp(x, [-26, -9, 9, 26], row))-16.291507720947266


def _oriente(lod):
    m = Mesh()
    pitch = 17.07
    sides = (6, 5, 3)[lod]
    branch_steps = (7, 4, 2)[lod]
    # Platforms occupy the islands between source railway centerlines. They
    # never cover the source's eight tracks or manufacture new approach rails.
    zs = np.linspace(-128, 128, (33, 25, 17)[lod])
    for x in [-25.605, -8.535, 8.535, 25.605]:
        for z0, z1 in zip(zs, zs[1:]):
            h0, h1 = _oriente_ground(x,z0)+5.15, _oriente_ground(x,z1)+5.15
            a=(x-4.5,h0,z0);b=(x+4.5,h0,z0);c=(x+4.5,h1,z1);d=(x-4.5,h1,z1)
            _quad(m,[a,b,c,d],(163,170,164),8,(0,1,0))
            for s in [-1,1]:
                _quad(m,[(x+s*4.5,h0-.8,z0),(x+s*4.5,h0,z0),
                         (x+s*4.5,h1,z1),(x+s*4.5,h1-.8,z1)],WHITE,17,(s,0,0))
                _quad(m,[(x+s*4.28-.065,h0+.035,z0),(x+s*4.28+.065,h0+.035,z0),
                         (x+s*4.28+.065,h1+.035,z1),(x+s*4.28-.065,h1+.035,z1)],
                      (237,218,152),94,(0,1,0))
        for j in range(15):
            z = (j-7)*pitch
            floor = _oriente_ground(x,z)+5.15
            m.beam((x,floor-.9,z),(x,16.7,z),.24,WHITE,16,sides)
            if lod<2:
                _lathe(m,[(floor-.2,.44),(floor+.1,.44),(floor+.7,.28)],WHITE,17,12,(x,0,z))
            else:
                m.box((x,floor+.2,z),(.65,.7,.65),WHITE,17)
            def roof(u,v):
                y=23.1+1.6*(abs(u)**1.55+abs(v)**1.55)/2+.65*abs(u*v)
                return np.array([x+u*pitch/2,y,z+v*pitch/2])
            for dx,dz in [(-1,-1),(-1,1),(1,1),(1,-1)]:
                end=roof(dx,dz)
                def branch(t):
                    return np.array([x+dx*pitch*.5*t*t,
                                     16.4+(end[1]-16.4)*(2*t-t*t),
                                     z+dz*pitch*.5*t*t])
                _path(m,[branch(t) for t in np.linspace(0,1,branch_steps+1)],.14,WHITE,16,sides)
                if lod==0:
                    _path(m,[branch(t)+[0,-.16,0] for t in np.linspace(.48,.94,4)],.038,(214,235,227),94,4)
            # Square glass modules have scalloped boundaries, four branch
            # fans, and subdivision mullions that retain the cathedral rhythm.
            grid = (6,3,2)[lod]
            for u0 in range(grid):
                for v0 in range(grid):
                    u,v=-1+2*u0/grid,-1+2*v0/grid
                    p=[roof(u,v),roof(u+2/grid,v),roof(u+2/grid,v+2/grid),roof(u,v+2/grid)]
                    tint=(160,185,191) if (u0+v0+j)%3 else (174,195,198)
                    _quad(m,p,tint,15,(0,1,0))
                    _quad(m,[p[3]-[0,.045,0],p[2]-[0,.045,0],p[1]-[0,.045,0],p[0]-[0,.045,0]],tint,15,(0,-1,0))
            for index in range(grid+1) if lod<2 else []:
                t=-1+2*index/grid
                for swapped in [False,True]:
                    pts=[roof(s,t) if swapped else roof(t,s) for s in np.linspace(-1,1,grid+1)]
                    _path(m,[p+[0,.07,0] for p in pts],.06 if index not in[0,grid] else .105,WHITE,16,4)
            if lod==0:
                # Fine diagonal ribs split each tree's four principal fans.
                for dx,dz in [(-1,0),(0,1),(1,0),(0,-1)]:
                    _path(m,[(x,18.2,z),roof(dx*.45,dz*.45)-[0,1.1,0],roof(dx,dz)],.07,WHITE,16,4)
    # Paired undercroft arches fit beneath the retained elevated railway.
    for x in [-34.14,-17.07,0,17.07,34.14]:
        for j in range(14):
            z=(j-6.5)*pitch
            spring=min(_oriente_ground(x,z-pitch/2),_oriente_ground(x,z+pitch/2))-.3
            crown=_oriente_ground(x,z)+4.3
            for offset in [-.42,.42] if lod==0 else [0.]:
                pts=[(x+offset,spring+(crown-spring)*math.sin(math.pi*t)**.7,z+(t-.5)*pitch)
                     for t in np.linspace(0,1,(12,6,4)[lod]+1)]
                _path(m,pts,.29,WHITE,17,sides)
    if lod==2:
        for x in [-34.14,34.14]:
            _path(m,[(x,23.9+(1.45 if i%2==0 else 0),z)
                     for i,z in enumerate(np.linspace(-128.025,128.025,31))],.12,WHITE,16,3)
        for z in [-128.025,128.025]:
            _path(m,[(x,23.9+(1.45 if i%2==0 else 0),z)
                     for i,x in enumerate(np.linspace(-34.14,34.14,9))],.12,WHITE,16,3)
    # Side balustrades make the long lifted platform legible in silhouette.
    for x in [-36.5,36.5]:
        for z0,z1 in zip(zs,zs[1:]):
            y0,y1=_oriente_ground(x,z0)+6.2,_oriente_ground(x,z1)+6.2
            m.beam((x,y0,z0),(x,y1,z1),.11,WHITE,16,sides)
        for z in np.linspace(-128,128,(65,33,17)[lod]):
            y=_oriente_ground(x,z)+5.0
            m.beam((x,y,z),(x,y+1.2,z),.055,WHITE,16,4)
    return m


def _monas(lod):
    m=Mesh()
    n=(64,40,24)[lod]
    # Low museum terraces precede the unmistakable 45m upturned square cup.
    _rect_loft(m,[(-5,46,46),(.0,46,46),(.45,45.5,45.5)],(178,174,159))
    for i in range(8):
        width=44-i*.8
        m.box((0,.45+i*.24+.12,0),(2*width,.24,2*width),WHITE,17)
    _rect_loft(m,[(2.37,36,36),(7.3,36,36),(8.0,34.8,34.8)],STONE)
    # Recessed relief bands have original geometric facets, no inscriptions.
    for side in range(4):
        a=side*math.pi/2
        out=np.array([math.sin(a),0,math.cos(a)])
        tangent=np.array([math.cos(a),0,-math.sin(a)])
        for i in range((18,12,8)[lod]):
            t=(i/((18,12,8)[lod]-1)-.5)*61
            c=out*36.08+tangent*t+[0,4.7,0]
            panel=Mesh();panel.box((0,0,0),(2.5,2.65,.1),(104,96,74),16)
            panel.transform([[math.cos(a),0,math.sin(a),c[0]],[0,1,0,c[1]],[-math.sin(a),0,math.cos(a),c[2]],[0,0,0,1]])
            m.merge(panel)
    _rect_loft(m,[(8.,10.0,10.0),(9.7,11.1,11.1),(11.5,13.7,13.7),
                  (13.5,17.2,17.2),(15.6,22.5,22.5),(17.0,22.5,22.5)],WHITE)
    _rect_loft(m,[(16.7,22.63,22.63),(17.08,22.63,22.63)],WHITE)
    for y,w in [(16.75,22.68),(114.5,5.8),(117.0,5.2)]:
        for s in [-1,1]:
            m.beam((-w,y,s*w),(w,y,s*w),.08,WARM,94,4)
            m.beam((s*w,y,-w),(s*w,y,w),.08,WARM,94,4)
    # The long square shaft stays slender, with softly beveled corners.
    _rect_loft(m,[(17.08,4.6,4.6),(108.4,2.6,2.6),(110.2,2.6,2.6)],WHITE)
    for side in range(4):
        a=side*math.pi/2
        p=lambda x,y,z:(x*math.cos(a)+z*math.sin(a),y,-x*math.sin(a)+z*math.cos(a))
        _quad(m,[p(-.75,18,4.62),p(-.4,108,2.63),p(.4,108,2.63),p(.75,18,4.62)],(218,218,205),17)
    _rect_loft(m,[(110.2,2.7,2.7),(112.7,5.8,5.8),(113.25,5.8,5.8)],WHITE)
    m.box((0,114.55,0),(10.3,2.6,10.3),DARK,15)
    for side in range(4):
        a=side*math.pi/2
        for x in np.linspace(-4.7,4.7,(9,7,5)[lod]):
            p=(x*math.cos(a)+5.22*math.sin(a),114.55,-x*math.sin(a)+5.22*math.cos(a))
            m.box(p,(.17,2.65,.17),WHITE,17)
    _rect_loft(m,[(115.85,5.8,5.8),(116.4,5.8,5.8),(117.0,4.0,4.0)],WHITE)
    _lathe(m,[(117,2.7),(117.7,2.7),(118.0,2.1)],GOLD,16,n)
    # A fixed asymmetric flame is a closed sculpted volume, never particles.
    levels=(30,20,12)[lod]
    rings=[]
    for i in range(levels+1):
        t=i/levels
        r=2.45*(math.sin(math.pi*(.10+.9*t)))**.7+.08
        cx=1.5*math.sin(math.pi*t)*t-.8*t*t
        rings.append([np.array([cx+r*(1+.15*math.sin(5*a+5*t))*math.cos(a),
                                 118+14*t,r*(1+.15*math.sin(5*a+5*t))*math.sin(a)])
                      for a in np.linspace(0,math.tau,n,endpoint=False)])
    for i,(a,b) in enumerate(zip(rings,rings[1:])):
        for j in range(n):
            k=(j+1)%n
            color=(225,169,54) if j%5 else (242,196,83)
            m.quad(a[j],b[j],b[k],a[k],color,94 if j%8==0 else 16)
    for j in range(n):
        k=(j+1)%n
        m.triangle((0,118,0),rings[0][j],rings[0][k],GOLD,16)
        m.triangle((-.8,132,0),rings[-1][k],rings[-1][j],GOLD,16)
    for i in range(5):
        a=i*math.tau/5
        pts=[]
        for t in np.linspace(0,1,(16,12,8)[lod]):
            r=2.2*(1-t)+.2
            pts.append((r*math.cos(a+1.2*t)+.3*t,118+10.4*t,r*math.sin(a+1.2*t)))
        _path(m,pts,.14,(245,196,79),94,(6,5,4)[lod])
    return m


def _azadi(lod):
    m=Mesh()
    _rect_loft(m,[(-5,35.7,23.4),(-.15,35.7,23.4),(.18,35.2,22.9)],(189,185,167))
    levels=(36,24,16)[lod]
    ys=sorted(set(np.linspace(.18,44.3,levels+1).tolist()+[18.,21.5,40.,43.]))
    def shape(y):
        w=9.2+25.8*math.exp(-y/10.8)
        d=6.3+14.7*math.exp(-y/10.8)
        gx=11.5*max(0,1-y/21.5)**.57
        gz=7.8*max(0,1-y/18.)**.65
        return w,d,gx,gz
    for sx in [-1,1]:
        for sz in [-1,1]:
            rings=[]
            for y in ys:
                w,d,gx,gz=shape(y)
                rings.append([np.array([sx*x,y,sz*z]) for x,z in [(gx,gz),(gx,d),(w,d),(w,gz)]])
            for a,b in zip(rings,rings[1:]):
                c=(sum(a)+sum(b))/8
                for j in range(4):
                    k=(j+1)%4
                    p=[a[j],a[k],b[k],b[j]]
                    normal=(sum(p)/4-c)*[1,0,1]
                    _quad(m,p,WHITE,17,normal)
            _quad(m,rings[0],WHITE,17,(0,-1,0))
            _quad(m,rings[-1],WHITE,17,(0,1,0))
    # The woven blue spandrel is distinct from the open passage beneath it.
    for front in [-1,1]:
        ny=(16,12,8)[lod]
        def spandrel(u,v):
            y=21.7+10.2*v
            width=6.7*(1-v)**.8
            return np.array([u*width,y,front*(shape(y)[1]+.055)])
        for i in range(ny):
            v=i/ny
            for j in range((10,8,6)[lod]):
                nx=(10,8,6)[lod]
                u=-1+2*j/nx
                p=[spandrel(u,v),spandrel(u+2/nx,v),spandrel(u+2/nx,v+1/ny),spandrel(u,v+1/ny)]
                _quad(m,p,(58,114,120),17,(0,0,front))
        # Woven diagonal marble ribs terminate inside the pointed field.
        nr=(11,9,7)[lod]
        for slope in [-1,1]:
            for intercept in np.linspace(-1.7,1.7,nr):
                pts=[]
                for v in np.linspace(0,.97,ny+1):
                    u=intercept+slope*1.3*v
                    if abs(u)<=.97:
                        pts.append(spandrel(u,v)+[0,0,front*.09])
                if len(pts)>1:_path(m,pts,.08,WHITE,17,(5,4,4)[lod])
        # Turquoise stone inlays follow the flaring concrete, not straight
        # floating strips; the lines are non-emissive except at entry lamps.
        for sx in [-1,1]:
            for f in np.linspace(.12,.91,(7,5,4)[lod]):
                pts=[]
                for y in np.linspace(.35,40.1,(28,20,14)[lod]):
                    w,d,gx,gz=shape(y)
                    x=sx*(gx+(w-gx)*f)
                    pts.append((x,y,front*(d+.025)))
                _path(m,pts,.045,TEAL,17,4)
        for x in np.linspace(-7.5,7.5,(11,9,7)[lod]):
            z=front*(shape(41.5)[1]+.045)
            m.box((x,41.4,z),(.48,2.25,.10),DARK,15)
        # Light washes are represented only by small fixed fixtures.
        for sx in [-1,1]:
            for x in [15,23,31]:
                m.box((sx*x,.26,front*20.2),(1.35,.18,.45),WARM,94)
    for side in [-1,1]:
        for f in np.linspace(-.85,.85,(9,7,5)[lod]):
            pts=[(side*(shape(y)[0]+.03),y,f*shape(y)[1]) for y in np.linspace(22,40.1,(16,12,8)[lod])]
            _path(m,pts,.035,TEAL,17,4)
        for z in np.linspace(-5.1,5.1,(7,5,3)[lod]):
            m.box((side*(shape(41.5)[0]+.04),41.4,z),(.1,2.25,.42),DARK,15)
    # A recessed saddle cap retains the four raised corner tips.
    w,d,_,_=shape(44.3)
    for sx in [-1,1]:
        for sz in [-1,1]:
            m.triangle((0,43.6,0),(sx*w,45,sz*d),(sx*w,44.3,0),WHITE,17)
            m.triangle((0,43.6,0),(0,44.3,sz*d),(sx*w,45,sz*d),WHITE,17)
    # The arch underside remains open; shallow crossing ribs add depth above
    # the visitor passage without filling its two principal sight lines.
    for k in range((9,7,5)[lod]):
        z=-5.9+k*11.8/((9,7,5)[lod]-1)
        pts=[(x,21.25-.038*x*x,z) for x in np.linspace(-6,6,(12,8,6)[lod])]
        _path(m,pts,.07,(202,204,192),17,4)
    return m


def _dome(m, center, radius, rise, lod, color=GOLD, ribbed=True):
    n=(64,40,24)[lod]
    rings=(18,12,8)[lod]
    x,y,z=center
    profile=[(rise*math.sin(t),radius*math.cos(t)) for t in np.linspace(0,math.pi/2,rings+1)]
    _lathe(m,profile,color,16,n,center)
    _ring(m,center,radius,.15,(232,188,84),94,(64,40,24)[lod])
    if ribbed:
        for a in np.linspace(0,math.tau,(32,20,12)[lod],endpoint=False):
            pts=[(x+(radius+.035)*math.cos(t)*math.cos(a),y+rise*math.sin(t)+.035,
                  z+(radius+.035)*math.cos(t)*math.sin(a)) for t in np.linspace(0,math.pi*.48,rings+1)]
            _path(m,pts,.045,(172,130,40),16,4)
    _lathe(m,[(0,.29),(.7,.29),(1.15,.13),(2.7,.08),(3.0,.015)],(221,179,70),16,12,(x,y+rise,z))
    m.ellipsoid((x,y+rise+.6,z),(.35,.4,.35),GOLD,16,(16,12,8)[lod],8)


def _abuja(lod):
    m=Mesh()
    n=(48,32,20)[lod]
    _rect_loft(m,[(-7,37.8,37.8),(.2,37.8,37.8),(.7,37.3,37.3)],(171,158,124))
    _rect_loft(m,[(.7,33.5,33.5),(2.0,33.5,33.5),(2.4,32.8,32.8)],(210,199,168))
    _rect_loft(m,[(2.4,31.8,31.8),(23.0,26.0,26.0),(24.,26.,26.)],(183,168,121))
    _rect_loft(m,[(23.6,26.6,26.6),(24.1,26.6,26.6)],WHITE)
    # Battered bays, deep vertical joints and strong pale buttresses make the
    # square prayer hall legible beneath the five gold domes.
    for side in range(4):
        a=side*math.pi/2
        def p(x,y,z):
            return (x*math.cos(a)+z*math.sin(a),y,-x*math.sin(a)+z*math.cos(a))
        bays=(17,13,9)[lod]
        for i in range(bays):
            x=(i/(bays-1)-.5)*50.5
            _quad(m,[p(x-.11,3,31.7),p(x+.11,3,31.7),p(x+.11,23,26.15),p(x-.11,23,26.15)],(113,104,80),16)
        for x in [-25,-12.5,0,12.5,25]:
            _path(m,[p(x,2.5,32.0),p(x,22.8,26.5)],.22,(222,210,170),17,(6,5,4)[lod])
        for i in range((29,21,15)[lod]):
            x=(i/((29,21,15)[lod]-1)-.5)*51.4
            # Low pointed crenellations are original solids, not texture art.
            points=[p(x-.47,24.1,26.35),p(x+.47,24.1,26.35),p(x,25.03,26.35)]
            m.triangle(*points,WHITE,17)
            m.triangle(*[p(x0,y,26.1) for x0,y in [(x+.47,24.1),(x-.47,24.1),(x,25.03)]],WHITE,17)
    _lathe(m,[(23.9,17.2),(26.3,17.2)],(115,110,90),11,n)
    for a in np.linspace(0,math.tau,(40,28,20)[lod],endpoint=False):
        x,z=17.32*math.cos(a),17.32*math.sin(a)
        m.beam((x,24.0,z),(x,26.5,z),.22,WHITE,17,(6,5,4)[lod])
    _lathe(m,[(26.3,17.65),(26.6,17.65)],WHITE,17,n)
    _dome(m,(0,26.6,0),17.7,14.0,lod)
    for x in [-22.0,22.0]:
        for z in [-22.0,22.0]:
            _lathe(m,[(23.9,3.65),(25.0,3.65)],WHITE,17,24,(x,0,z))
            for a in np.linspace(0,math.tau,(12,8,6)[lod],endpoint=False):
                xx,zz=x+3.68*math.cos(a),z+3.68*math.sin(a)
                m.beam((xx,24.15,zz),(xx,25,zz),.15,STONE,17,4)
            _dome(m,(x,25,z),3.8,3.1,lod,(214,172,65),False)
    for x,z in [(-33.1,-33.4),(-33.1,34.5),(35.1,-33.4),(35.1,34.5)]:
        _rect_loft(m,[(.7,3.0,3.0),(4.7,3.0,3.0),(6.3,2.1,2.1)],STONE,17,(x,0,z))
        _lathe(m,[(6.3,1.62),(46.6,1.38),(47.6,2.4),(48.35,2.4),(48.65,1.4),
                  (54.4,1.35),(54.8,1.95),(55.25,1.95)],(202,181,113),16,n,(x,0,z))
        for a in np.linspace(0,math.tau,(12,10,8)[lod],endpoint=False):
            xx,zz=math.cos(a),math.sin(a)
            m.beam((x+1.63*xx,6.4,z+1.63*zz),(x+1.39*xx,46.7,z+1.39*zz),.07,(171,150,83),16,4)
        # Balcony corbels and the open upper lantern define the minaret top.
        for a in np.linspace(0,math.tau,(16,12,8)[lod],endpoint=False):
            ux,uz=math.cos(a),math.sin(a)
            m.beam((x+1.39*ux,45.9,z+1.39*uz),(x+2.25*ux,47.75,z+2.25*uz),.12,(174,151,89),17,4)
        for y,r in [(48.25,2.42),(54.87,1.95)]:
            _ring(m,(x,y,z),r,.085,WARM,94,(32,24,16)[lod])
        _lathe(m,[(55.25,1.92),(55.8,1.7),(56.5,1.2),(57.4,.72),
                  (58.5,.34),(59.7,.12),(60.7,.08)],WHITE,17,n,(x,0,z))
        m.beam((x,60.4,z),(x,63.0,z),.075,GOLD,16,6)
        for y,r in [(59.8,.2),(60.5,.15)]:
            m.ellipsoid((x,y,z),(r,r,r),GOLD,16,12,8)
    # A small muqarnas-like entrance recess terminates inside the surveyed
    # forecourt, preserving annex buildings immediately south of the mosque.
    _rect_loft(m,[(.7,9,3.5),(7.8,9,3.5),(10.1,7.4,2.7)],STONE,17,(0,0,-34.5))
    m.box((0,4.0,-38.04),(10.9,6.3,.10),DARK,15)
    for j in range((5,4,3)[lod]):
        y=6.0+j*.65
        half=5.8-j*.65
        for s in [-1,1]:
            m.beam((s*half,y,-38.18),(s*(half-.68),y+.72,-38.27),.22,WHITE,17,4)
        m.beam((-half,y,-38.19),(half,y,-38.19),.08,WARM,94,4)
    return m


def build(name, lod):
    if lod not in (0, 1, 2):
        raise ValueError("city catalog LOD must be 0, 1 or 2")
    builders={"brasilia-cathedral":_brasilia, "santiago-bahai-temple":_santiago,
              "oriente-station":_oriente, "monas":_monas,
              "azadi-tower":_azadi, "abuja-national-mosque":_abuja}
    return builders[name](lod)
