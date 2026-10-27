"""Authored waterfront silhouettes; all light geometry is time invariant."""

import math

try:
    from .common import Mesh
except ImportError:
    from common import Mesh


# Road alignment derived from OpenStreetMap contributors (ODbL 1.0),
# snapshot 2026-09-12. Coordinates are latitude, longitude, DEM metres.
_GWANGAN_OSM = {'north': [[35.148996, 129.1314637, 0.014],
           [35.1541649, 129.1363892, 0.051],
           [35.154712, 129.1366969, -0.005],
           [35.1551932, 129.136886, -0.056],
           [35.1557258, 129.1369836, -0.138],
           [35.1562436, 129.1370081, -0.191],
           [35.1567442, 129.1369539, -0.106],
           [35.1573253, 129.1367548, 0.011],
           [35.1578789, 129.136498, -0.059],
           [35.1580722, 129.1363599, -0.085],
           [35.1585908, 129.1360011, -0.083],
           [35.1588261, 129.1358012, -0.168]],
 'south': [[35.1425428, 129.12527, 0.008],
           [35.1382762, 129.1211753, 0.0],
           [35.1381655, 129.1210514, 0.004],
           [35.1379936, 129.1208537, 0.009],
           [35.137791, 129.1205839, 0.02],
           [35.1375277, 129.1201831, 0.019],
           [35.1373693, 129.1198756, -0.002],
           [35.1372168, 129.1195487, -0.042]],
 'north_upper': [[35.1588261, 129.1358012, -0.168],
                 [35.159467, 129.1352151, -0.957],
                 [35.1595775, 129.1351104, -1.104],
                 [35.1604611, 129.1343677, -0.795],
                 [35.1610622, 129.1338592, 0.232],
                 [35.1618382, 129.1331997, -0.351],
                 [35.1625359, 129.1325671, -1.701],
                 [35.1632126, 129.1320078, -14.427],
                 [35.1639744, 129.1313781, -7.023],
                 [35.1652784, 129.1303288, 6.374]],
 'north_lower': [[35.1586007, 129.1360183, -0.082],
                 [35.1591194, 129.1356015, -0.426],
                 [35.1597606, 129.1350659, -1.194],
                 [35.160949, 129.1340472, 0.05],
                 [35.1624177, 129.1328856, -2.952],
                 [35.1640012, 129.1315393, -8.416],
                 [35.1646374, 129.131025, 3.323],
                 [35.1653516, 129.1304388, 6.789]],
 'south_upper': [[35.1372168, 129.1195487, -0.042],
                 [35.1370617, 129.1191988, -0.087],
                 [35.1369811, 129.1189831, -0.103],
                 [35.1369028, 129.1187303, -0.102],
                 [35.1368346, 129.1184677, -0.087],
                 [35.1367695, 129.1181599, -0.053],
                 [35.1367276, 129.1179239, -0.022],
                 [35.1366864, 129.1176587, 0.011],
                 [35.1366109, 129.1170043, 0.046],
                 [35.1364995, 129.1159849, -0.008],
                 [35.1362879, 129.1143292, 0.031],
                 [35.1361007, 129.112963, 1.77],
                 [35.1360465, 129.1125564, 2.806],
                 [35.1359912, 129.1122319, 3.591]],
 'south_lower': [[35.1372098, 129.1197761, -0.041],
                 [35.1370933, 129.1195272, -0.076],
                 [35.1369717, 129.1192373, -0.108],
                 [35.1368797, 129.1189642, -0.12],
                 [35.1368411, 129.1188295, -0.12],
                 [35.1368055, 129.1187009, -0.114],
                 [35.1367525, 129.1184916, -0.095],
                 [35.1367281, 129.1183794, -0.083],
                 [35.1367072, 129.118281, -0.07],
                 [35.1366487, 129.1179602, -0.028],
                 [35.1365884, 129.1175282, 0.02],
                 [35.1363906, 129.1158695, -0.007],
                 [35.1362334, 129.1146796, 0.003],
                 [35.1360387, 129.1131594, 1.292],
                 [35.1359122, 129.1122491, 3.371]],
 'south_upper_ramp': [[35.1372168, 129.1195487, -0.042],
                      [35.1371078, 129.1192028, -0.074],
                      [35.1370469, 129.118984, -0.087],
                      [35.1369846, 129.1187535, -0.09],
                      [35.1369094, 129.1184044, -0.072],
                      [35.136836, 129.1179646, -0.023],
                      [35.136679, 129.1167493, 0.047],
                      [35.1366231, 129.1161475, 0.005],
                      [35.1365462, 129.1155183, -0.052],
                      [35.1363726, 129.1142322, 0.031]],
 'south_lower_ramp': [[35.1369717, 129.1192373, -0.108],
                      [35.136889, 129.1190546, -0.122],
                      [35.1368459, 129.1189609, -0.125],
                      [35.1368115, 129.1188705, -0.126],
                      [35.1367762, 129.118771, -0.122],
                      [35.1367427, 129.1186644, -0.114],
                      [35.1367071, 129.1185429, -0.102],
                      [35.1366714, 129.118414, -0.088],
                      [35.1366393, 129.1182898, -0.072],
                      [35.1365929, 129.1180858, -0.044],
                      [35.1365506, 129.1178884, -0.021],
                      [35.1365017, 129.1176237, 0.007],
                      [35.1364529, 129.1172689, 0.024],
                      [35.1363519, 129.1165305, 0.013],
                      [35.1362558, 129.1157482, -0.006],
                      [35.1361563, 129.115047, -0.01],
                      [35.1360643, 129.1143516, 0.054]]}


_GWANGAN_LOWER_REFERENCE = {'north': [(35.1489181, 129.1314892),
           (35.1499542, 129.132468),
           (35.1513648, 129.1337866),
           (35.1528505, 129.1352309),
           (35.1536716, 129.1359997),
           (35.1541523, 129.1364154),
           (35.1543464, 129.1365148),
           (35.1547123, 129.1367053),
           (35.1549828, 129.136824),
           (35.1551961, 129.1369065),
           (35.1556446, 129.1370009),
           (35.1562312, 129.1370257),
           (35.1567544, 129.1369703),
           (35.1573281, 129.1367771),
           (35.1578876, 129.1365137),
           (35.1586007, 129.1360183),
           (35.1591194, 129.1356015),
           (35.1597606, 129.1350659)],
 'south': [(35.1425188, 129.1253014),
           (35.1411538, 129.1239816),
           (35.1405189, 129.1233783),
           (35.140185, 129.1230592),
           (35.1400236, 129.1229042),
           (35.1397026, 129.1225965),
           (35.1392415, 129.1221535),
           (35.1384986, 129.1214544),
           (35.1382169, 129.1211618),
           (35.1380282, 129.1209588),
           (35.1378655, 129.1207733),
           (35.1377067, 129.1205755),
           (35.1375736, 129.1203934),
           (35.1374497, 129.1202094),
           (35.1373122, 129.1199756),
           (35.1372098, 129.1197761),
           (35.1370933, 129.1195272),
           (35.1369717, 129.1192373)]}

def _smooth(value):
    t = min(1.0, max(0.0, value))
    return t*t*(3-2*t)


def _gwangan_local(lat, lon):
    origin = math.radians(35.145662)
    q = 1-6.6943799901413165e-3*math.sin(origin)**2
    meridian = 6378137*(1-6.6943799901413165e-3)/q**1.5
    prime = 6378137/math.sqrt(q)
    north = math.radians(lat-35.145662)*meridian
    east = math.radians(lon-129.128345)*prime*math.cos(origin)
    heading = math.radians(38.30)
    return (east*math.cos(heading)-north*math.sin(heading),
            east*math.sin(heading)+north*math.cos(heading))


def _approach_paths():
    """Frozen OSM alignment and DEM samples; no online inputs during baking."""
    paths = {}
    for name, source in _GWANGAN_OSM.items():
        raw = [[*_gwangan_local(lat, lon), ground] for lat, lon, ground in source]
        if name in ('north', 'south'):
            raw[0] = [0.0, 450.0 if name == 'north' else -450.0, 0.0]
        # A fixed subdivision is shared by all LODs and suppression polygons.
        points = [raw[0]]
        for a, b in zip(raw, raw[1:]):
            count = max(1, math.ceil(math.hypot(b[0]-a[0], b[1]-a[1])/35))
            points.extend([[a[k]+(b[k]-a[k])*i/count for k in range(3)]
                           for i in range(1, count+1)])
        distances = [0.0]
        for a, b in zip(points, points[1:]):
            distances.append(distances[-1]+math.hypot(b[0]-a[0], b[1]-a[1]))
        common = name in ('north', 'south')
        lower_reference = [_gwangan_local(*p) for p in _GWANGAN_LOWER_REFERENCE.get(name, [])]
        result = []
        for point, distance in zip(points, distances):
            x, z, ground = point
            if common:
                top = 47.3+(5.0-47.3)*_smooth(distance/distances[-1])
                # Once duplex decks converge, their two mapped road strips form
                # one wider corridor. Fit both source centerlines, so a low
                # duplicate strip cannot survive next to the authored deck.
                best = (float('inf'), x, z)
                for start, end in zip(lower_reference, lower_reference[1:]):
                    dx,dz=end[0]-start[0],end[1]-start[1]
                    t=min(1.0,max(0.0,((x-start[0])*dx+(z-start[1])*dz)/(dx*dx+dz*dz)))
                    qx,qz=start[0]+t*dx,start[1]+t*dz
                    candidate=(math.hypot(qx-x,qz-z),qx,qz)
                    if candidate[0]<best[0]:best=candidate
                blend=_smooth(distance/120)
                half_width=12.0+best[0]*.5*blend
                x+=(best[1]-x)*.5*blend
                z+=(best[2]-z)*.5*blend
            else:
                # The current source fallback has 5 m clearance. The actual
                # northern bridge/ground transitions ramp this to zero in 48 m.
                clearance = 5.0
                if name in ('north_upper', 'north_lower'):
                    clearance *= min(1.0, (distances[-1]-distance)/48)
                top = max(0.0, ground)+clearance
                half_width = 9.5
            result.append((x, top, z, half_width, distance, ground))
        paths[name] = result
    return paths


def _path_sides(path, pad=0.0):
    sides = []
    for i, point in enumerate(path):
        a, b = path[max(0, i-1)], path[min(len(path)-1, i+1)]
        dx, dz = b[0]-a[0], b[2]-a[2]
        length = math.hypot(dx, dz)
        nx, nz = dz/length, -dx/length
        radius = point[3]+pad
        sides.append(((point[0]-nx*radius, point[2]-nz*radius),
                      (point[0]+nx*radius, point[2]+nz*radius)))
    return sides


def _road_polygons():
    polygons = [[[-19.0, -450.0], [19.0, -450.0],
                 [19.0, 450.0], [-19.0, 450.0]]]
    for path in _approach_paths().values():
        # Mitered shared cross-sections keep the convex clips gap-free.
        sides = _path_sides(path, 1.6)
        for a, b in zip(sides, sides[1:]):
            polygons.append([[float(x), float(z)] for x, z in [a[0], a[1], b[1], b[0]]])
    return polygons


def _deck_segment(mesh, left_a, right_a, left_b, right_b, top_a, top_b, thickness):
    a = (left_a[0], top_a, left_a[1])
    b = (right_a[0], top_a, right_a[1])
    c = (right_b[0], top_b, right_b[1])
    d = (left_b[0], top_b, left_b[1])
    low = [(p[0], p[1]-thickness, p[2]) for p in (a, b, c, d)]
    mesh.quad(a, d, c, b, (0.15, 0.17, 0.18), 7)
    mesh.quad(low[0], low[1], low[2], low[3], CONCRETE, 16)
    for j in range(4):
        k = (j+1) % 4
        mesh.quad(low[j], (a,b,c,d)[j], (a,b,c,d)[k], low[k], CONCRETE, 16)


def _gwangan_approaches(mesh, lod):
    paths = _approach_paths()
    segments = [(a, b) for path in paths.values() for a,b in zip(path,path[1:])]
    def covered(x, z, own_a, own_b):
        for a,b in segments:
            if a is own_a and b is own_b:
                continue
            dx,dz=b[0]-a[0],b[2]-a[2]
            t=((x-a[0])*dx+(z-a[2])*dz)/(dx*dx+dz*dz)
            if 0.02 < t < 0.98 and math.hypot(x-a[0]-t*dx,z-a[2]-t*dz)<min(a[3],b[3])-0.35:
                return True
        return False
    for name, path in paths.items():
        common = name in ('north', 'south')
        sides = _path_sides(path)
        last_pier = -100.0
        for i,(a,b) in enumerate(zip(path,path[1:])):
            left_a,right_a=sides[i];left_b,right_b=sides[i+1]
            thickness = 2.7 if common else 1.8
            _deck_segment(mesh,left_a,right_a,left_b,right_b,a[1],b[1],thickness)
            if common:
                # The distant source has one road elevation. The lower deck
                # rises into the upper approach before that source transition.
                gap_a=8*(1-_smooth(a[4]/550));gap_b=8*(1-_smooth(b[4]/550))
                if gap_a>0.05:
                    _deck_segment(mesh,left_a,right_a,left_b,right_b,
                                  a[1]-gap_a,b[1]-gap_b,min(2.7,max(0.1,gap_a*.4)))
            for side, (p,q) in enumerate(((left_a,left_b),(right_a,right_b))):
                if covered((p[0]+q[0])/2,(p[1]+q[1])/2,a,b):
                    continue
                rail_a=(p[0],a[1]+.60,p[1]);rail_b=(q[0],b[1]+.60,q[1])
                mesh.beam(rail_a,rail_b,.24,CONCRETE,16,4)
                if common and a[4] < 600:
                    mesh.beam((p[0],a[1]-.1,p[1]),(q[0],b[1]-.1,q[1]),.13,ICE,94,4)
            if a[4]-last_pier>80 and a[1]-max(0,a[5])>4:
                last_pier=a[4]
                bottom=max(-2,a[5]-1)
                height=a[1]-thickness-bottom
                mesh.box((a[0],bottom+height/2,a[2]),(3.2,height,4.3),CONCRETE,16)
                mesh.beam((left_a[0],a[1]-thickness-.9,left_a[1]),
                          (right_a[0],a[1]-thickness-.9,right_a[1]),.9,CONCRETE,16,4)
            if lod<2 and i%2==0:
                mx,mz=(a[0]+b[0])/2,(a[2]+b[2])/2
                dx,dz=b[0]-a[0],b[2]-a[2];length=math.hypot(dx,dz)
                for offset in (-3.1,3.1):
                    cx,cz=mx+dz/length*offset,mz-dx/length*offset
                    mesh.beam((cx-dx/length*2,(a[1]+b[1])/2+.025,cz-dz/length*2),
                              (cx+dx/length*2,(a[1]+b[1])/2+.025,cz+dz/length*2),.065,(.78,.78,.69),8,4)


ENTRIES = [
    {
        "name": "hongyadong", "city": "Chongqing",
        # The anchor is the uphill terrace, not the river road 48.5 m below it.
        "lat": 29.564827, "lon": 106.575395, "heading": -20.0,
        "base": 231.29296875, "suppress_buildings": True,
        "activation_pad": 3.0,
        "activation_bounds_xz": [[-103.0, -14.0], [103.0, 90.0]],
        "suppress_roads": False, "suppress_trees": True,
        "suppress_bbox": [[-102.0, -13.0], [102.0, 56.0]],
        "references": [
            "https://www.architecture.com/-/media/Files/International/RIBA_China_Architects_Building_Contemporary_China_2024.pdf",
            "https://www.travelchina.org.cn/en/article/vF1cJZaYCtX8",
            "https://www.openstreetmap.org/way/939578294",
        ],
        "description": "A 200 m riverfront village descending the 48.5 m cliff, with timber galleries, sweeping tiled roofs and steady amber eaves.",
        "capture": {"distance": 265, "azimuth": 340, "elevation": 9},
    },
    {
        "name": "gwangandaegyo-bridge", "city": "Busan",
        "lat": 35.145662, "lon": 129.128345, "heading": 38.30,
        "base": 0.0, "suppress_buildings": True,
        "suppress_roads": True, "suppress_trees": False,
        "suppress_bbox": [[-25.0, -470.0], [25.0, 470.0]],
        "activation_pad": 3.0,
        "road_exclusion_polygons": _road_polygons(),
        "alignment_attribution": "Road geography derived from OpenStreetMap contributors, ODbL 1.0; snapshot 2026-09-12. Bridge mesh and height adaptation are authored. https://www.openstreetmap.org/copyright",
        "references": [
            "https://bisco.or.kr/gwanganbridge/image/brochure/diamondbridge_brochure.pdf",
            "https://www.openstreetmap.org/way/382760140",
            "https://www.openstreetmap.org/way/1238722288",
            "https://www.openstreetmap.org/way/1238722289",
            "https://www.openstreetmap.org/way/382760156",
            "https://www.openstreetmap.org/way/580959067",
            "https://www.openstreetmap.org/way/382760546",
            "https://www.openstreetmap.org/way/382760547",
            "https://www.openstreetmap.org/way/382760780",
            "https://www.openstreetmap.org/way/382760781",
            "https://www.suyeong.go.kr/eng/index.suyeong",
        ],
        "description": "Gwangan's 500 m main span, paired 105 m portal towers, double deck, 200 m side spans and adjoining viaducts, traced by steady cool lights.",
        "capture": {"distance": 1250, "azimuth": 130, "elevation": 12},
    },
]

WOOD = (0.24, 0.115, 0.06)
WALL = (0.48, 0.32, 0.19)
TILE = (0.20, 0.255, 0.24)
AMBER = (1.0, 0.49, 0.095)
WARM = (1.0, 0.67, 0.25)
STEEL = (0.68, 0.74, 0.77)
CONCRETE = (0.51, 0.55, 0.57)
ICE = (0.30, 0.69, 1.0)
VIOLET = (0.60, 0.39, 1.0)


def _roof(mesh, x, y, z, width, depth, rise, lod):
    """Three hip rings preserve the low concave slope and lifted corners."""
    def ring(w, d, height, lift):
        return [(x-w/2, y+height+lift, z-d/2),
                (x, y+height, z-d/2), (x+w/2, y+height+lift, z-d/2),
                (x+w/2, y+height, z), (x+w/2, y+height+lift, z+d/2),
                (x, y+height, z+d/2), (x-w/2, y+height+lift, z+d/2),
                (x-w/2, y+height, z)]
    rings = [ring(width, depth, 0, 0.6),
             ring(width*0.69, depth*0.60, rise*0.34, 0.0),
             ring(width*0.38, 0.12, rise, 0.0)]
    for lower, upper in zip(rings, rings[1:]):
        for j in range(8):
            k = (j+1) % 8
            mesh.quad(lower[j], upper[j], upper[k], lower[k], TILE, 12)
    mesh.beam((x-width*0.20, y+rise+0.05, z),
              (x+width*0.20, y+rise+0.05, z), 0.19, WOOD, 12, 6)
    outer = rings[0]
    for j in range(8):
        mesh.beam(outer[j], outer[(j+1) % 8], 0.13, AMBER, 94, 4)
    # Raised hip ribs explain the roof shape from above without tile textures.
    if lod < 2:
        for j in (0, 2, 4, 6):
            for lower, upper in zip(rings, rings[1:]):
                a, b = lower[j], upper[j]
                mesh.beam((a[0], a[1]+0.08, a[2]),
                          (b[0], b[1]+0.08, b[2]), 0.09, (0.30, 0.34, 0.29), 12, 4)


def _pavilion(mesh, x, bottom, z, width, depth, floors, lod, roof=True):
    floor_height = 4.45
    height = floors*floor_height
    front = z+depth/2
    mesh.box((x, bottom+height/2, z), (width, height, depth), WALL)
    if bottom > -40:
        # The site's cleaned cliff descends beneath each gallery. Masonry
        # retaining walls close the resulting wedge, not the open lowest stilts.
        foundation = []
        for px, pz in [(x-width/2, z-depth/2), (x+width/2, z-depth/2),
                       (x+width/2, z+depth/2), (x-width/2, z+depth/2)]:
            grade = -min(50.8, max(0.0, (pz+10.0)*1.25))
            foundation.append((px, min(bottom-0.35, grade-0.8), pz))
        top = [(p[0], bottom+0.02, p[2]) for p in foundation]
        stone = (0.28, 0.265, 0.225)
        for j in range(4):
            k = (j+1) % 4
            mesh.quad(foundation[j], top[j], top[k], foundation[k], stone, 11)
        mesh.quad(top[0], top[3], top[2], top[1], stone, 11)
        # Timber diagonals carry the projecting balcony above the stone wall.
        for bx in range(max(2, round(width/6))):
            px = x-width/2+(bx+0.5)*width/max(2, round(width/6))
            mesh.beam((px, bottom-2.3, front-0.12),
                      (px, bottom-0.08, front+1.95), 0.17, WOOD, 11, 4)
    bays = max(3, round(width/4.3))
    bay_width = width/bays
    # Deep balconies, timber posts and recessed bays separate the illuminated
    # eaves from the unlit wall; they must retain their depth at every LOD.
    for floor in range(floors):
        fy = bottom+floor*floor_height
        mesh.box((x, fy+0.18, front+0.9), (width+1.1, 0.36, 2.5), WOOD)
        mesh.box((x, fy+3.78, front+1.45), (width+1.8, 0.38, 0.65), WOOD)
        mesh.box((x, fy+3.94, front+1.8), (width+1.9, 0.14, 0.16), AMBER, 94)
        mesh.box((x, fy+1.20, front+1.98), (width+0.7, 0.14, 0.16), WOOD)
        for bay in range(bays):
            bx = x-width/2+(bay+0.5)*bay_width
            mesh.box((bx, fy+2.14, front+0.045), (bay_width-0.52, 2.45, 0.11), (0.16, 0.10, 0.055), 15)
            if (bay+floor) % 4 != 1:
                mesh.box((bx, fy+2.25, front+0.12), (bay_width-0.82, 1.95, 0.09), (0.82, 0.37, 0.08), 94)
            mesh.box((bx-bay_width/2, fy+2.1, front+1.4), (0.28, 4.0, 0.32), WOOD)
            if lod == 0:
                for dx in (-0.7, 0.0, 0.7):
                    mesh.box((bx+dx, fy+2.22, front+0.2), (0.10, 2.35, 0.12), WOOD)
                mesh.box((bx, fy+2.4, front+0.22), (bay_width-0.5, 0.12, 0.13), WOOD)
                for dx in (-0.65, 0.65):
                    mesh.box((bx+dx, fy+0.82, front+1.98), (0.13, 0.70, 0.14), WOOD)
        # Side balconies remain legible from the bridge-side view.
        for side in (-1, 1):
            sx = x+side*(width/2+0.55)
            mesh.box((sx, fy+0.18, z), (1.45, 0.36, depth), WOOD)
            mesh.box((sx+side*0.74, fy+3.9, z), (0.12, 0.15, depth+1.5), AMBER, 94)
    if roof:
        _roof(mesh, x, bottom+height+0.18, z+0.5,
              width+5.8, depth+6.3, 3.1 if width < 28 else 3.8, lod)


def _hongyadong(lod):
    mesh = Mesh()
    # The long shallow masses descend toward +z (the river). Separate roof
    # terraces, unequal heights, and an open cleft retain the cliff-village form.
    mesh.box((0, -49.4, 30), (197, 2.8, 41), (0.30, 0.29, 0.25), 8)
    for x, bottom, z, width, depth, floors in [
        (-79, -47, 34, 28, 17, 4), (-47, -47, 37, 28, 15, 3),
        (-16, -47, 39, 27, 13, 3), (15, -47, 37, 27, 15, 3),
        (48, -47, 33, 29, 17, 4), (80, -47, 29, 24, 18, 5),
        (-77, -29.2, 19, 30, 16, 4), (-42, -33.65, 23, 28, 16, 5),
        (-8, -29.2, 20, 30, 17, 4), (29, -29.2, 17, 33, 17, 5),
        (68, -24.75, 13, 28, 16, 5),
        (-76, -11.4, 1, 26, 15, 3), (-42, -11.4, 2, 28, 15, 2),
        (-8, -11.4, 1, 29, 14, 2), (28, -6.95, -1, 29, 14, 2),
        (67, -2.5, -2, 25, 15, 2),
    ]:
        _pavilion(mesh, x, bottom, z, width, depth, floors, lod)
    # The lower gallery is carried on dark exposed stilt bracing.
    for x in range(-88, 93, 9):
        mesh.box((x, -47.3, 47), (0.55, 4.8, 0.65), WOOD)
        if lod < 2:
            mesh.beam((x, -49.5, 47), (x+5, -45.2, 47), 0.17, WOOD, 11, 4)
    # A pair of side stair flights connects the terraces rather than filling
    # every opening with a tower or a uniformly emissive slab.
    for side, z0, y0 in [(-1, 39, -45), (1, 25, -27)]:
        x = side*96
        for step in range(18 if lod < 2 else 9):
            inc = 1 if lod < 2 else 2
            s = step*inc
            mesh.box((x, y0+s*0.30, z0-s*0.40),
                     (3.2, 0.32*inc, 0.46*inc), (0.43, 0.36, 0.27), 8)
        mesh.beam((x+side*1.55, y0+1, z0+0.2),
                  (x+side*1.55, y0+6.2, z0-7), 0.10, WARM, 94, 4)
    return mesh


def _gwangan(lod):
    mesh = Mesh()
    deck_top, lower_top = 47.2, 39.2
    extent = 450
    # Both carriageways share the same longitudinal centerline in this duplex
    # bridge. The lower deck's soffit preserves the documented 35 m clearance.
    for top in (lower_top, deck_top):
        mesh.box((0, top-1.35, 0), (24, 2.7, extent*2), STEEL, 16)
        mesh.box((0, top+0.05, 0), (21.8, 0.10, extent*2), (0.15, 0.17, 0.18), 7)
        for x in (-11.35, 11.35):
            mesh.box((x, top+0.62, 0), (0.46, 1.25, extent*2), CONCRETE, 16)
        if lod < 2:
            for x in (-5.5, 0, 5.5):
                for z in range(-extent+8, extent, 19):
                    mesh.box((x, top+0.115, z), (0.16, 0.03, 5), (0.78, 0.78, 0.69), 8)
    for side in (-1, 1):
        x = side*12.32
        mesh.beam((x, deck_top-0.10, -extent), (x, deck_top-0.10, extent), 0.17, ICE, 94, 4)
        mesh.beam((x, lower_top-0.10, -extent), (x, lower_top-0.10, extent), 0.11, VIOLET, 94, 4)
    # Real tower centers, 500 m apart, were measured from OSM support footprints.
    for z in (-250, 250):
        for x in (-15.2, 15.2):
            mesh.box((x, 2, z), (11.5, 8, 15), CONCRETE)
            mesh.box((x, 53.5, z), (4.7, 103, 7.0), STEEL, 16)
            for side in (-1, 1):
                mesh.box((x+side*2.39, 76, z), (0.12, 55, 3.7), ICE, 94)
            mesh.box((x, 105.8, z), (5.1, 1.6, 7.4), STEEL, 16)
        for y, h in [(36.0, 4.2), (68.5, 3.2), (100.8, 3.8)]:
            mesh.box((0, y, z), (30.5, h, 5.5), STEEL, 16)
            for sz in (-1, 1):
                mesh.box((0, y-0.4, z+sz*2.8), (27, 0.22, 0.16), ICE, 94)
    for z in (-450, 450):
        mesh.box((0, 12, z), (36, 28, 30), CONCRETE)
        mesh.box((0, 31, z), (31, 11, 24), STEEL, 16)
    def cable_y(z):
        if abs(z) <= 250:
            return 55 + 49*(z/250)**2
        t = (abs(z)-250)/200
        return 104-66*t+3.3*t*(1-t)
    step = (10, 15, 25)[lod]
    for x in (-15.2, 15.2):
        points = [(x, cable_y(z), z) for z in range(-450, 451, step)]
        if points[-1][2] != 450:
            points.append((x, cable_y(450), 450))
        for a, b in zip(points, points[1:]):
            mesh.beam(a, b, 0.50, STEEL, 16, 6)
            mesh.beam((a[0], a[1]+0.5, a[2]), (b[0], b[1]+0.5, b[2]),
                      0.18, ICE, 94, 4)
        for z in range(-425, 426, (12, 18, 25)[lod]):
            y = cable_y(z)
            if y <= deck_top:
                continue
            mesh.beam((x, deck_top, z), (x, y, z), 0.085 if lod == 0 else 0.10, STEEL, 16, 4)
            mesh.beam((math.copysign(11.8, x), deck_top-0.7, z),
                      (x, deck_top, z), 0.20, STEEL, 16, 4)
            # Lighting is deliberately steady, with violet support rhythms.
            if z % 2 == 0:
                mesh.beam((x, deck_top+0.1, z), (x, y, z), 0.055, VIOLET, 94, 4)
    # Repeated diagonal trusses explain the duplex deck at an oblique approach.
    for side in (-1, 1):
        x = side*11.9
        for z in range(-extent, extent-20, (12, 20, 30)[lod]):
            length = min((12, 20, 30)[lod], extent-z)
            mesh.beam((x, lower_top+0.8, z), (x, deck_top-1.6, z+length),
                      0.16, STEEL, 16, 4)
    _gwangan_approaches(mesh, lod)
    return mesh


def build(name, lod):
    if lod not in (0, 1, 2):
        raise ValueError(f"invalid LOD: {lod}")
    if name == "hongyadong":
        mesh = _hongyadong(lod)
    elif name == "gwangandaegyo-bridge":
        mesh = _gwangan(lod)
    else:
        raise ValueError(f"unknown waterfront catalog entry: {name}")
    mesh.triangles = [(p, tuple(round(c*255) for c in color), material)
                      for p, color, material in mesh.triangles]
    return mesh
