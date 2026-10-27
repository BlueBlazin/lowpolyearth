#!/usr/bin/env python3
"""Validate source-identified art envelopes and generate the immutable place atlas.

No network is used. GeoNames identities are resolved against the pinned bundled
search database; CC0 Wikidata coordinates are extracted from hashed entity JSON.
"""

import argparse
import hashlib
import json
import math
import mmap
from pathlib import Path
import re
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/vearth-gen/assets/architecture"
CATALOG = ASSETS / "places.json"
OUTPUT = ASSETS / "places-generated.rs"
MANIFEST = ASSETS / "places-manifest.json"
GEONAMES = ROOT / "crates/vearth-data/assets/place-search-v1.bin"
EARTH_RADIUS_M = 6_371_008.8


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_source_snapshot(source, ident, kind="corridor"):
    has_path = "source_snapshot" in source
    has_hash = "source_sha256" in source
    require(has_path == has_hash, f"{ident}: {kind} snapshot and checksum must be paired")
    if not has_path:
        return
    relative = source["source_snapshot"]
    require(isinstance(relative, str) and bool(relative) and "\\" not in relative
            and not Path(relative).is_absolute()
            and all(part not in ("", ".", "..") for part in relative.split("/")),
            f"{ident}: invalid {kind} snapshot path")
    path = (ASSETS / relative).resolve()
    require(path.is_relative_to(ASSETS.resolve()), f"{ident}: {kind} snapshot escapes assets")
    expected = source["source_sha256"]
    require(isinstance(expected, str) and re.fullmatch(r"[0-9a-f]{64}", expected) is not None,
            f"{ident}: invalid {kind} snapshot checksum")
    require(path.is_file(), f"{ident}: {kind} snapshot is missing")
    require(sha256(path) == expected, f"{ident}: {kind} snapshot checksum mismatch")
    return path


def geonames_points(ids, expected_hash):
    require(sha256(GEONAMES) == expected_hash, "GeoNames artifact checksum mismatch")
    points = {}
    with GEONAMES.open("rb") as source:
        with mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as data:
            require(data[:8] == b"VEARTHPS", "GeoNames artifact magic mismatch")
            require(struct.unpack_from("<II", data, 8) == (1, 72), "GeoNames format mismatch")
            count = struct.unpack_from("<I", data, 16)[0]
            records = struct.unpack_from("<I", data, 24)[0]
            strings = struct.unpack_from("<I", data, 32)[0]
            for i in range(count):
                row = struct.unpack_from("<IIIiiIIHBBBBhI", data, records + i * 40)
                no, co, _, lat, lon, _, _, nl, cl, _, _, flags, _, ident = row
                if ident not in ids:
                    continue
                require(not flags & 1, f"{ident}: synthetic curated point is not GeoNames")
                points[f"geonames:{ident}"] = {
                    "source_name": data[strings + no:strings + no + nl].decode(),
                    "country_name": data[strings + co:strings + co + cl].decode(),
                    "latitude_deg": lat / 1e7,
                    "longitude_deg": lon / 1e7,
                }
    require(len(points) == len(ids), "Missing GeoNames source IDs")
    return points


def wikidata_point(source, snapshots=None):
    ident = source["entity_id"]
    filename = source.get("snapshot", f"{ident}.json")
    require(re.fullmatch(r"[A-Za-z0-9_-]+\.json", filename), f"{ident}: invalid snapshot filename")
    path = ASSETS / "wikidata" / filename
    snapshots = {} if snapshots is None else snapshots
    if filename not in snapshots:
        snapshots[filename] = (sha256(path), json.loads(path.read_text())["entities"])
    checksum, entities = snapshots[filename]
    require(checksum == source["sha256"], f"{ident}: entity checksum mismatch")
    entity = entities[ident]
    require(entity["lastrevid"] == source["revision"], f"{ident}: revision mismatch")
    coordinates = [claim for claim in entity["claims"]["P625"]
                   if claim["rank"] != "deprecated" and "datavalue" in claim["mainsnak"]]
    preferred = [claim for claim in coordinates if claim["rank"] == "preferred"]
    coordinates = preferred or coordinates
    if "coordinate_statement" in source:
        coordinates = [claim for claim in coordinates if claim["id"] == source["coordinate_statement"]]
    require(len(coordinates) == 1, f"{ident}: coordinate ambiguity needs review")
    point = coordinates[0]["mainsnak"]["datavalue"]["value"]
    require(point["globe"] == "http://www.wikidata.org/entity/Q2", f"{ident}: not Earth")
    countries = {claim["mainsnak"]["datavalue"]["value"]["id"]
                 for claim in entity["claims"].get("P17", [])
                 if claim["rank"] != "deprecated" and "datavalue" in claim["mainsnak"]}
    require(source["country_entity"] in countries, f"{ident}: country mismatch")
    return {
        "source_name": entity["labels"][source.get("label_language", "en")]["value"],
        "country_name": source["country_name"],
        "latitude_deg": point["latitude"],
        "longitude_deg": point["longitude"],
    }


def corridor_caps(area):
    """Triangle inequality: each segment lies within its midpoint's half-length."""
    def unit(point):
        lat, lon = map(math.radians, (point["latitude_deg"], point["longitude_deg"]))
        return (math.cos(lat) * math.cos(lon), math.cos(lat) * math.sin(lon), math.sin(lat))
    for start, end in zip(area["points"], area["points"][1:]):
        a, b = unit(start), unit(end)
        c = (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])
        angle = math.atan2(math.sqrt(sum(v*v for v in c)), sum(x*y for x, y in zip(a, b)))
        require(1e-12 < angle < math.pi - 1e-12, "corridor has coincident or antipodal consecutive points")
        mid = tuple(x+y for x, y in zip(a, b))
        yield {"latitude_deg": math.degrees(math.atan2(mid[2], math.hypot(mid[0], mid[1]))),
               "longitude_deg": math.degrees(math.atan2(mid[1], mid[0])),
               "outer_radius_m": angle * EARTH_RADIUS_M / 2 + area["outer_half_width_m"]}


def point_in_ring(point, ring):
    x, y = point
    inside = False
    for (ax, ay), (bx, by) in zip(ring, ring[1:]):
        if (ay > y) != (by > y) and x < ax + (y-ay)*(bx-ax)/(by-ay):
            inside = not inside
    return inside


def projected_boundary_distance(point, rings):
    x, y = point
    closest = math.inf
    for ring in rings:
        for (ax, ay), (bx, by) in zip(ring, ring[1:]):
            dx, dy = bx-ax, by-ay
            length2 = dx*dx + dy*dy
            t = max(0, min(1, ((x-ax)*dx + (y-ay)*dy)/length2)) if length2 else 0
            closest = min(closest, math.hypot(x-ax-t*dx, y-ay-t*dy))
    return closest


def ring_index(ring):
    """Conservative y buckets; coordinates and edge order remain unchanged."""
    low, high = min(p[1] for p in ring), max(p[1] for p in ring)
    count = min(128, max(1, math.isqrt(len(ring) - 1)))
    rows = [[] for _ in range(count)]
    def row(y):
        return max(0, min(count - 1, math.floor((y - low) / (high - low) * count)))
    for edge, (a, b) in enumerate(zip(ring, ring[1:])):
        for index in range(row(min(a[1], b[1])), row(max(a[1], b[1])) + 1):
            rows[index].append(edge)
    return low, high, rows


def compile_polygon(source, ident):
    require(isinstance(source, dict) and source.get("source_url", "").startswith("https://")
            and bool(source.get("note")), f"{ident}: polygon needs source and interpretation")
    require("source_snapshot" in source and "source_sha256" in source,
            f"{ident}: polygon requires a pinned snapshot and checksum")
    path = verify_source_snapshot(source, ident, "polygon")
    fade = source.get("boundary_falloff_m")
    require(type(fade) in (int, float) and math.isfinite(fade) and 0 < fade <= 20000,
            f"{ident}: invalid polygon boundary falloff")
    data = json.loads(path.read_text())
    # GeoJSON defaults to WGS84 longitude/latitude. Never silently reinterpret a
    # legacy projected-CRS declaration or choose one feature from a collection.
    require(isinstance(data, dict) and "crs" not in data, f"{ident}: polygon requires WGS84 GeoJSON")
    geometry = data.get("geometry") if data.get("type") == "Feature" else data
    require(isinstance(geometry, dict) and "crs" not in geometry and geometry.get("type") in ("Polygon", "MultiPolygon"),
            f"{ident}: expected one GeoJSON Polygon or MultiPolygon")
    coordinates = geometry.get("coordinates")
    require(isinstance(coordinates, list) and bool(coordinates), f"{ident}: empty polygon geometry")
    components = [coordinates] if geometry["type"] == "Polygon" else coordinates
    polygons = []
    def unit(coordinate):
        lon, lat = map(math.radians, coordinate)
        return (math.cos(lat)*math.cos(lon), math.cos(lat)*math.sin(lon), math.sin(lat))
    def dot(a, b):
        return sum(x*y for x, y in zip(a, b))
    for component in components:
        require(isinstance(component, list) and bool(component), f"{ident}: polygon has no exterior")
        for ring in component:
            require(isinstance(ring, list) and len(ring) >= 4 and ring[0] == ring[-1],
                    f"{ident}: polygon rings must be closed with at least four coordinates")
            for coordinate in ring:
                require(isinstance(coordinate, list) and len(coordinate) == 2
                        and all(type(v) in (int, float) and math.isfinite(v) for v in coordinate)
                        and abs(coordinate[0]) <= 180 and abs(coordinate[1]) <= 90,
                        f"{ident}: invalid polygon longitude/latitude")
            require(all(a != b for a, b in zip(ring, ring[1:])), f"{ident}: duplicate consecutive polygon points")
        vectors = [unit(c) for c in component[0][:-1]]
        center = tuple(sum(v[i] for v in vectors) for i in range(3))
        norm = math.sqrt(dot(center, center))
        require(norm > 1e-12, f"{ident}: polygon has no bounded local center")
        center = tuple(v/norm for v in center)
        lon = math.atan2(center[1], center[0])
        lat = math.atan2(center[2], math.hypot(center[0], center[1]))
        east = (-math.sin(lon), math.cos(lon), 0.0)
        north = (-math.sin(lat)*math.cos(lon), -math.sin(lat)*math.sin(lon), math.cos(lat))
        projected = []
        radius = 0
        for ring in component:
            points = []
            for coordinate in ring:
                v = unit(coordinate)
                denominator = dot(center, v)
                angle = math.atan2(math.hypot(dot(east, v), dot(north, v)), denominator)
                radius = max(radius, angle*EARTH_RADIUS_M)
                require(angle*EARTH_RADIUS_M <= 20000, f"{ident}: polygon exceeds 20 km local cap")
                points.append((EARTH_RADIUS_M*dot(east, v)/denominator,
                               EARTH_RADIUS_M*dot(north, v)/denominator))
            twice_area = sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(points, points[1:]))
            require(abs(twice_area) > 1e-6, f"{ident}: polygon ring has zero area")
            projected.append(points)
        # A scanline-derived interior focus avoids placing the staging camera in
        # a concavity or hole. This build-time search does not simplify rings.
        ys = sorted({p[1] for ring in projected for p in ring})
        best, clearance = None, 0
        for lo, hi in zip(ys, ys[1:]):
            y = (lo+hi)/2
            xs = sorted(a[0]+(y-a[1])*(b[0]-a[0])/(b[1]-a[1])
                        for ring in projected for a,b in zip(ring, ring[1:]) if (a[1] > y) != (b[1] > y))
            for left, right in zip(xs, xs[1:]):
                candidate = ((left+right)/2, y)
                if point_in_ring(candidate, projected[0]) and not any(point_in_ring(candidate, h) for h in projected[1:]):
                    distance = projected_boundary_distance(candidate, projected)
                    if distance > clearance:
                        best, clearance = candidate, distance
        require(best is not None, f"{ident}: polygon has no interior outside holes")
        focus = tuple(center[i]+east[i]*best[0]/EARTH_RADIUS_M+north[i]*best[1]/EARTH_RADIUS_M for i in range(3))
        polygons.append({"center": center, "east": east, "north": north,
                         "cap_cos": math.cos((radius+1e-5)/EARTH_RADIUS_M),
                         "rings": projected,
                         "representative_center": (math.degrees(math.atan2(focus[2], math.hypot(focus[0], focus[1]))),
                                                   math.degrees(math.atan2(focus[1], focus[0]))),
                         "latitude_deg": math.degrees(lat), "longitude_deg": math.degrees(lon),
                         "outer_radius_m": radius+1e-5})
    return {"polygons": polygons, "boundary_falloff_m": fade}


def spatial_cells(point):
    """Conservative spherical-cap bounds, including dateline and polar caps."""
    if "polygons" in point:
        return sorted({cell for polygon in point["polygons"] for cell in spatial_cells(polygon)})
    if "points" in point:
        return sorted({cell for cap in corridor_caps(point) for cell in spatial_cells(cap)})
    lat, lon = point["latitude_deg"], point["longitude_deg"]
    radius = point["outer_radius_m"] / EARTH_RADIUS_M
    lat_delta = math.degrees(radius)
    min_row = max(0, math.floor((lat - lat_delta + 90) / 2))
    max_row = min(89, math.floor((lat + lat_delta + 90) / 2))
    if abs(lat) + lat_delta >= 90:
        columns = range(180)
    else:
        lon_delta = math.degrees(math.asin(min(1, math.sin(radius) / math.cos(math.radians(lat)))))
        lo = math.floor((lon - lon_delta + 180) / 2)
        hi = math.floor((lon + lon_delta + 180) / 2)
        columns = sorted({col % 180 for col in range(lo, hi + 1)})
    return [row * 180 + col for row in range(min_row, max_row + 1) for col in columns]


def municipal_point(source):
    """Derive a district focus from the pinned LPC building-footprint bounds."""
    require(source["source_id"] == "nyclpc:LP-2683", "Unknown municipal source")
    path = ASSETS / "nyclpc" / "LP-2683.geojson"
    require(sha256(path) == source["sha256"], "LPC artifact checksum mismatch")
    data = json.loads(path.read_text())
    require(data["crs"]["properties"]["name"] == "EPSG:4326", "LPC coordinate system mismatch")
    features = data["features"]
    require(features and len(features) == source["feature_count"], "LPC feature count mismatch")
    points = []
    identities = set()
    for feature in features:
        props = feature["properties"]
        require(props["Hist_Dist"] == source["source_name"], "LPC district mismatch")
        require(props["OBJECTID"] not in identities, "Duplicate LPC feature")
        identities.add(props["OBJECTID"])
        geometry = feature["geometry"]
        require(geometry["type"] in ("Polygon", "MultiPolygon"), "Unsupported LPC geometry")
        polygons = geometry["coordinates"] if geometry["type"] == "MultiPolygon" else [geometry["coordinates"]]
        for polygon in polygons:
            for ring in polygon:
                for lon, lat in ring:
                    require(math.isfinite(lon) and math.isfinite(lat)
                            and -74 < lon < -73.8 and 40.6 < lat < 40.8, "LPC point outside Brooklyn")
                    points.append((lon, lat))
    require(points, "Empty LPC geometry")
    return {
        "source_name": source["source_name"],
        "country_name": "United States",
        "latitude_deg": (min(p[1] for p in points) + max(p[1] for p in points)) / 2,
        "longitude_deg": (min(p[0] for p in points) + max(p[0] for p in points)) / 2,
    }


def compile_catalog(catalog):
    require(catalog["schema_version"] == 1, "Unsupported catalog schema")
    source = (ROOT / "crates/vearth-gen/src/architecture_styles.rs").read_text()
    style_keys = set(re.findall(r'\w+\s*=>\s*\("([a-z0-9-]+)"', source))
    require(len(style_keys) >= 40, "Could not extract the typed style catalog")
    entries = catalog["places"]
    geo_ids = {int(row["source_id"].split(":")[1]) for row in entries
               if row["source_id"].startswith("geonames:")}
    points = geonames_points(geo_ids, catalog["geonames_sha256"])
    snapshots = {}
    for source in catalog["wikidata_sources"]:
        ident = "wikidata:" + source["entity_id"]
        require(ident not in points, f"Duplicate source {ident}")
        points[ident] = wikidata_point(source, snapshots)
    for source in catalog.get("municipal_sources", []):
        ident = source["source_id"]
        require(ident not in points, f"Duplicate source {ident}")
        points[ident] = municipal_point(source)
    seen = set()
    rows = []
    for entry in entries:
        ident = entry["source_id"]
        key = entry["style_key"]
        require(key in style_keys, f"{ident}: unknown style {key}")
        require((ident, key) not in seen, f"Duplicate influence {ident}/{key}")
        seen.add((ident, key))
        require(ident in points, f"Unknown point {ident}")
        point = points[ident]
        for field in ["source_name", "country_name"]:
            require(entry[field] == point[field], f"{ident}: {field} differs from source: {point[field]}")
        require(entry["extent_kind"] == "authored-art-influence", f"{ident}: extent must disclose authorship")
        require(entry["evidence_scope"] in ("place-character", "compatible-regional-transfer"),
                f"{ident}: evidence scope must distinguish local character from regional transfer")
        require(entry["evidence_urls"] and entry["art_interpretation"], f"{ident}: missing evidence/interpretation")
        require(all(url.startswith("https://") for url in entry["evidence_urls"]), f"{ident}: invalid evidence URL")
        scope_tier = entry.get("scope_tier", "district")
        require(scope_tier in ("district", "city"), f"{ident}: invalid scope_tier")
        row = {**entry, **point, "scope_tier": scope_tier}
        # Settlement search points identify the place, but can lie outside its
        # historic fabric. Keep that provenance separate from a sourced focus.
        if focus := entry.get("influence_center"):
            require(focus.get("source_url", "").startswith("https://") and bool(focus.get("note")),
                    f"{ident}: influence center needs source and interpretation")
            for field, limit in [("latitude_deg", 90), ("longitude_deg", 180)]:
                value = focus.get(field)
                require(type(value) in (int, float) and math.isfinite(value) and abs(value) <= limit,
                        f"{ident}: invalid influence center {field}")
                row["source_" + field] = point[field]
                row[field] = value
        polygon = entry.get("influence_polygon")
        require(polygon is None or (isinstance(polygon, dict) and not any(field in entry for field in
                ("influence_center", "influence_areas", "influence_corridor"))),
                f"{ident}: use one geographic area representation")
        corridor = entry.get("influence_corridor")
        require(corridor is None or (isinstance(corridor, dict) and
                "influence_center" not in entry and "influence_areas" not in entry),
                f"{ident}: use one geographic area representation")
        radii = [] if corridor is not None or polygon is not None else ["core_radius_m", "outer_radius_m"]
        for field in radii + ["palette_confidence", "character_confidence",
                      "contemporary_infill", "max_height_m", "max_footprint_m2"]:
            require(not isinstance(row[field], bool) and isinstance(row[field], (int, float))
                    and math.isfinite(row[field]), f"{ident}: invalid {field}")
        if corridor is None and polygon is None:
            require(0 <= row["core_radius_m"] < row["outer_radius_m"] <= 20_000, f"{ident}: invalid radii")
        require(0 < row["max_height_m"] <= 1000 and 0 < row["max_footprint_m2"] <= 100_000,
                f"{ident}: invalid scale hints")
        for field in ["palette_confidence", "character_confidence", "contemporary_infill"]:
            require(0 <= row[field] <= 1, f"{ident}: {field} outside 0..1")
        require(-90 <= row["latitude_deg"] <= 90 and -180 <= row["longitude_deg"] <= 180,
                f"{ident}: invalid geographic coordinate")
        areas = entry.get("influence_areas")
        require(areas is None or (isinstance(areas, list) and bool(areas) and "influence_center" not in entry),
                f"{ident}: use one nonempty geographic area representation")
        if areas is not None:
            for area in areas:
                require(area.get("source_url", "").startswith("https://") and bool(area.get("note")),
                        f"{ident}: area needs source and interpretation")
                for field in ("latitude_deg", "longitude_deg", "core_radius_m", "outer_radius_m"):
                    require(type(area.get(field)) in (int, float) and math.isfinite(area[field]),
                            f"{ident}: invalid area {field}")
                require(abs(area["latitude_deg"]) <= 90 and abs(area["longitude_deg"]) <= 180
                        and 0 <= area["core_radius_m"] < area["outer_radius_m"] <= 20000,
                        f"{ident}: invalid area bounds")
        if polygon is not None:
            row["areas"] = [compile_polygon(polygon, ident)]
        elif corridor is not None:
            require(corridor.get("source_url", "").startswith("https://") and bool(corridor.get("note")),
                    f"{ident}: corridor needs source and interpretation")
            verify_source_snapshot(corridor, ident)
            corridor_points = corridor.get("points")
            require(isinstance(corridor_points, list) and len(corridor_points) >= 2,
                    f"{ident}: corridor needs at least two points")
            for point in corridor_points:
                require(isinstance(point, dict), f"{ident}: invalid corridor point")
                for field, limit in [("latitude_deg", 90), ("longitude_deg", 180)]:
                    require(type(point.get(field)) in (int, float) and math.isfinite(point[field])
                            and abs(point[field]) <= limit, f"{ident}: invalid corridor {field}")
            for field in ("core_half_width_m", "outer_half_width_m"):
                require(type(corridor.get(field)) in (int, float) and math.isfinite(corridor[field]),
                        f"{ident}: invalid corridor {field}")
            require(0 <= corridor["core_half_width_m"] < corridor["outer_half_width_m"] <= 20000,
                    f"{ident}: invalid corridor widths")
            # Match the existing local spherical-cap indexing scale. Additional
            # control points represent real bends, not a way to enlarge widths.
            require(all(cap["outer_radius_m"] <= 20000 for cap in corridor_caps(corridor)),
                    f"{ident}: corridor segment cap exceeds 20 km")
            row["areas"] = [corridor]
        else:
            row["areas"] = areas if areas is not None else [{field: row[field] for field in
                ("latitude_deg", "longitude_deg", "core_radius_m", "outer_radius_m")}]
        rows.append(row)
    rows.sort(key=lambda row: (row["source_id"], row["style_key"]))
    require(len(rows) < 65536, "Place IDs exceed u16")
    return rows


def generate(rows):
    style_source = (ROOT / "crates/vearth-gen/src/architecture_styles.rs").read_text()
    variants = {key: variant for variant, key in
                re.findall(r'(\w+)\s*=>\s*\("([a-z0-9-]+)"', style_source)}
    text = ["// @generated by tools/build-architecture-places.py; do not edit.",
            "// GeoNames CC BY 4.0; Wikidata CC0; NYC LPC open data. See places-SOURCES.md.",
            "// Authored envelopes: repository LICENSE.",
            "const PLACES: &[ArchitecturalPlace] = &["]
    fields = ["source_id", "source_name", "country_name", "style_key", "palette_confidence", "character_confidence",
              "contemporary_infill", "max_height_m", "max_footprint_m2"]
    for row in rows:
        text.append("    ArchitecturalPlace {")
        for key in fields:
            value = json.dumps(row[key], ensure_ascii=False) if isinstance(row[key], str) else repr(float(row[key]))
            text.append(f"        {key}: {value},")
        text.append(f"        style: ArchitectureStyle::{variants[row['style_key']]},")
        text.append(f"        scope_tier: PlaceScope::{row['scope_tier'].title()},")
        text.append("        areas: &[")
        for area in row["areas"]:
            if "polygons" in area:
                text.append("            InfluenceArea::Polygon { polygons: &[")
                for polygon in area["polygons"]:
                    text.append("                PolygonArea {")
                    for field in ("center", "east", "north"):
                        text.append(f"                    {field}: {[float(value) for value in polygon[field]]!r},")
                    text.append(f"                    cap_cos: {polygon['cap_cos']!r},")
                    text.append(f"                    representative_center: {polygon['representative_center']!r},")
                    text.append("                    rings: &[")
                    for ring in polygon["rings"]:
                        text.append("                        &[" + ", ".join(repr(tuple(point)) for point in ring) + "],")
                    text.append("                    ],")
                    text.append("                    ring_indices: &[")
                    for ring in polygon["rings"]:
                        low, high, buckets = ring_index(ring)
                        text.append(f"                        RingIndex {{ min_y: {low!r}, max_y: {high!r}, rows: &[")
                        for bucket in buckets:
                            text.append("                            &[" + ", ".join(map(str, bucket)) + "],")
                        text.append("                        ] },")
                    text.append("                    ],")
                    text.append("                },")
                text.append(f"            ], boundary_falloff_m: {float(area['boundary_falloff_m'])!r} " + "},")
            elif "points" in area:
                points = ", ".join(f"({float(p['latitude_deg'])!r}, {float(p['longitude_deg'])!r})"
                                   for p in area["points"])
                text.append("            InfluenceArea::Corridor { points: &[" + points + "], "
                            + f"core_half_width_m: {float(area['core_half_width_m'])!r}, "
                            + f"outer_half_width_m: {float(area['outer_half_width_m'])!r} " + "},")
            else:
                values = ", ".join(f"{field}: {float(area[field])!r}" for field in
                    ("latitude_deg", "longitude_deg", "core_radius_m", "outer_radius_m"))
                text.append("            InfluenceArea::Circle { " + values + " },")
        text.append("        ],")
        text.append("    },")
    text.append("];")
    buckets = {}
    for ident, row in enumerate(rows):
        for cell in sorted({cell for area in row["areas"] for cell in spatial_cells(area)}):
            buckets.setdefault(cell, []).append(ident)
    text.append("const BUCKETS: &[(u16, u32, u16)] = &[")
    candidates = []
    for cell, ids in sorted(buckets.items()):
        require(len(ids) < 65536, "Bucket exceeds u16")
        text.append(f"    ({cell}, {len(candidates)}, {len(ids)}),")
        candidates.extend(ids)
    text.append("];")
    text.append("const CANDIDATES: &[u16] = &[")
    for start in range(0, len(candidates), 16):
        text.append("    " + ", ".join(map(str, candidates[start:start + 16])) + ",")
    text.append("];")
    raw = "\n".join(text) + "\n"
    return subprocess.run(["rustfmt", "--edition", "2021", "--emit", "stdout"],
                          input=raw, text=True, capture_output=True, check=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify without writing")
    args = parser.parse_args()
    catalog = json.loads(CATALOG.read_text())
    rows = compile_catalog(catalog)
    output = generate(rows)
    manifest = json.dumps({
        "schema_version": 1,
        "catalog_sha256": sha256(CATALOG),
        "generated_rust_sha256": hashlib.sha256(output.encode()).hexdigest(),
        "geonames_sha256": catalog["geonames_sha256"],
        "place_count": len(rows),
        "country_territory_count": len({row["country_name"] for row in rows}),
        "style_count": len({row["style_key"] for row in rows}),
        "wikidata_entity_count": len(catalog["wikidata_sources"]),
    }, indent=2) + "\n"
    if args.check:
        require(OUTPUT.read_text() == output, "Generated atlas is stale; run builder")
        require(MANIFEST.read_text() == manifest, "Place manifest is stale; run builder")
    else:
        OUTPUT.write_text(output)
        MANIFEST.write_text(manifest)
    print(f"{len(rows)} places, {len({row['country_name'] for row in rows})} countries/territories, "
          f"{len({row['style_key'] for row in rows})} styles; SHA-256 {hashlib.sha256(output.encode()).hexdigest()}")


if __name__ == "__main__":
    main()
