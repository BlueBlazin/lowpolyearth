#!/usr/bin/env python3
"""Source-contract and spherical index checks for the architectural place builder."""

import copy
import importlib.util
import json
import math
from pathlib import Path
import random
import re
import subprocess
import unittest
from unittest import mock
import tempfile

SPEC = importlib.util.spec_from_file_location(
    "architecture_places", Path(__file__).with_name("build-architecture-places.py")
)
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


REQUESTED_LOCALITIES = set("""Venice
Lisbon
Barcelona
Istanbul
Fez
Havana
Cartagena
Beijing
Luang Prabang
Srinagar
London
Edinburgh
Bath
Dublin
Brussels
Prague
Budapest
Kraków
Tallinn
Riga
Stockholm
Copenhagen
Rome
Florence
Naples
Algiers
Ghardaïa
Cairo
Harar
Mombasa
Damascus
Aleppo
Jerusalem
Mardin
Tbilisi
Bukhara
Samarkand
Khiva
Udaipur
Varanasi
Lucknow
Puducherry
Galle
Lahore
Lijiang
Pingyao
Seoul
Bangkok
Quebec City
Montreal
New York City
Philadelphia
Mexico City
Puebla
Buenos Aires
Lima
Sydney
Melbourne
Dunedin
Antwerp
Vienna
Salzburg
Český Krumlov
Gdańsk
Vilnius
Helsinki
Bologna
Siena
Matera
Alberobello
Meknes
Rabat
Ghadames
Massawa
Island of Mozambique
Muscat
Shibam
Bushehr
Amritsar
Karaikudi
Dhaka
Dali
Suzhou
Quanzhou
Tainan
Yangon
Phuket
Guanajuato
Morelia
Oaxaca
Mérida
Zacatecas
Santiago de Cuba
Napier
Oamaru
Hamburg
Lübeck
Bremen
Heidelberg
Rothenburg ob der Tauber
Bamberg
Regensburg
Dresden
Nuremberg
Leipzig
Lyon
Dijon
Rouen
Verona
Lecce
Turin
Wrocław
Toruń
Sibiu
Sighișoara
Yerevan
Sheki
Baku
Nizwa
Kashgar
Lhasa
Shaxi
Fenghuang
Hongcun
Nara
Kurashiki
Shirakawa-go
Hakodate
Gyeongju
Bandung
Semarang
Padang
Santiago
Hobart
San Cristóbal de las Casas
San José""".splitlines())


class ArchitecturePlaceTests(unittest.TestCase):
    def test_ring_index_covers_crossings_and_distance_neighborhoods(self):
        ring = [(math.cos(i * math.tau / 513) * 900,
                 math.sin(i * math.tau / 513) * 1400) for i in range(513)]
        ring.append(ring[0])
        low, high, rows = BUILDER.ring_index(ring)
        self.assertTrue(all(len(set(row)) == len(row) for row in rows))
        for index, row in enumerate(rows):
            bottom = low + (high - low) * index / len(rows)
            top = low + (high - low) * (index + 1) / len(rows)
            for edge, (a, b) in enumerate(zip(ring, ring[1:])):
                if min(a[1], b[1]) < top and max(a[1], b[1]) > bottom:
                    self.assertIn(edge, row)

    @classmethod
    def setUpClass(cls):
        cls.catalog = json.loads(BUILDER.CATALOG.read_text())
        cls.rows = BUILDER.compile_catalog(cls.catalog)

    def test_source_point_roundtrip_and_generated_output(self):
        self.assertEqual(BUILDER.generate(self.rows), BUILDER.OUTPUT.read_text())
        self.assertGreaterEqual(len({row["country_name"] for row in self.rows}), 40)
        for row in self.rows:
            self.assertEqual(row["extent_kind"], "authored-art-influence")
            self.assertTrue(row["evidence_urls"])

    def test_requested_localities_have_individual_profiles_and_bounded_scopes(self):
        local = [row for row in self.rows if "locality_name" in row]
        self.assertTrue(REQUESTED_LOCALITIES <= {row["locality_name"] for row in local})
        self.assertGreaterEqual(len(local), len(REQUESTED_LOCALITIES))
        self.assertEqual(len({(row["source_id"], row["style_key"]) for row in self.rows}), len(self.rows))
        self.assertGreaterEqual(len(self.rows), 282)
        for row in local:
            self.assertTrue(row["coverage"])
            if row["scope_tier"] == "district":
                for area in row["areas"]:
                    if "polygons" in area:
                        self.assertTrue(all(p["outer_radius_m"] <= 20000 for p in area["polygons"]))
                    else:
                        self.assertLessEqual(area.get("outer_half_width_m", area.get("outer_radius_m")), 2200)
            self.assertGreaterEqual(row["contemporary_infill"], 0)
            self.assertLess(row["contemporary_infill"], 0.5)

    def test_scope_tier_is_explicit_and_defaults_without_reclassifying_old_entries(self):
        catalog = copy.deepcopy(self.catalog)
        entry = catalog["places"][0]
        entry.pop("scope_tier", None)
        key = (entry["source_id"], entry["style_key"])
        def compiled():
            return next(row for row in BUILDER.compile_catalog(catalog)
                        if (row["source_id"], row["style_key"]) == key)
        district = compiled()
        self.assertEqual(district["scope_tier"], "district")
        entry["scope_tier"] = "city"
        city = compiled()
        self.assertEqual(city["scope_tier"], "city")
        self.assertEqual(city["areas"], district["areas"])
        self.assertIn("scope_tier: PlaceScope::City,", BUILDER.generate([city]))
        self.assertIn("scope_tier: PlaceScope::District,", BUILDER.generate([district]))
        for invalid in [None, True, "neighborhood", "City", 1]:
            entry["scope_tier"] = invalid
            with self.assertRaisesRegex(ValueError, "invalid scope_tier"):
                BUILDER.compile_catalog(catalog)

    def test_snapshot_paths_and_explicit_coordinate_selection_are_validated(self):
        source = copy.deepcopy(next(s for s in self.catalog["wikidata_sources"]
                                    if "coordinate_statement" in s))
        source["coordinate_statement"] = "missing-statement"
        with self.assertRaisesRegex(ValueError, "coordinate ambiguity"):
            BUILDER.wikidata_point(source)
        source["snapshot"] = "../places.json"
        with self.assertRaisesRegex(ValueError, "invalid snapshot filename"):
            BUILDER.wikidata_point(source)

    def test_municipal_focus_is_derived_from_verified_district_geometry(self):
        source = copy.deepcopy(self.catalog["municipal_sources"][0])
        point = BUILDER.municipal_point(source)
        self.assertTrue(40.690 < point["latitude_deg"] < 40.696)
        self.assertTrue(-73.955 < point["longitude_deg"] < -73.945)
        source["source_name"] = "Another district"
        with self.assertRaisesRegex(ValueError, "district mismatch"):
            BUILDER.municipal_point(source)

    def test_unknown_style_cannot_enter_the_runtime_catalog(self):
        catalog = copy.deepcopy(self.catalog)
        catalog["places"][0]["style_key"] = "unimplemented-style"
        with self.assertRaisesRegex(ValueError, "unknown style"):
            BUILDER.compile_catalog(catalog)

    def test_historic_focus_preserves_search_identity_and_requires_provenance(self):
        row = next(r for r in self.rows if r["source_id"] == "geonames:3168320")
        self.assertAlmostEqual(row["latitude_deg"], 43.467914)
        self.assertAlmostEqual(row["source_latitude_deg"], 43.4738)
        self.assertNotEqual(row["longitude_deg"], row["source_longitude_deg"])
        catalog = copy.deepcopy(self.catalog)
        entry = next(r for r in catalog["places"] if r["source_id"] == row["source_id"])
        del entry["influence_center"]["source_url"]
        with self.assertRaisesRegex(ValueError, "influence center needs source"):
            BUILDER.compile_catalog(catalog)

    def test_country_or_name_mismatch_is_not_silently_accepted(self):
        catalog = copy.deepcopy(self.catalog)
        catalog["places"][0]["country_name"] = "Wrong country"
        with self.assertRaisesRegex(ValueError, "country_name differs"):
            BUILDER.compile_catalog(catalog)

    def test_extent_must_be_authored_and_finite(self):
        for field, value, error in [
            ("extent_kind", "surveyed-heritage-boundary", "extent must disclose"),
            ("outer_radius_m", float("nan"), "invalid outer_radius_m"),
            ("character_confidence", 1.1, "outside 0..1"),
        ]:
            catalog = copy.deepcopy(self.catalog)
            catalog["places"][0][field] = value
            with self.assertRaisesRegex(ValueError, error):
                BUILDER.compile_catalog(catalog)

    def test_corridor_source_contract_and_static_generation(self):
        catalog = copy.deepcopy(self.catalog)
        entry = catalog["places"][0]
        entry.pop("influence_center", None)
        entry.pop("influence_areas", None)
        entry.pop("core_radius_m", None)
        entry.pop("outer_radius_m", None)
        corridor = {"points": [{"latitude_deg": 0, "longitude_deg": 179.99},
                               {"latitude_deg": 0, "longitude_deg": -179.99}],
                    "core_half_width_m": 50, "outer_half_width_m": 100,
                    "source_url": "https://example.com/survey", "note": "Synthetic contract fixture."}
        entry["influence_corridor"] = corridor
        rows = BUILDER.compile_catalog(catalog)
        row = next(r for r in rows if (r["source_id"], r["style_key"]) == (entry["source_id"], entry["style_key"]))
        self.assertEqual(row["areas"], [corridor])
        self.assertIn("InfluenceArea::Corridor", BUILDER.generate(rows))
        for replacement, error in [
            ({"points": []}, "at least two points"),
            ({"points": [corridor["points"][0]] * 2}, "coincident or antipodal"),
            ({"points": [{"latitude_deg": 0, "longitude_deg": 0},
                          {"latitude_deg": 0, "longitude_deg": 180}]}, "coincident or antipodal"),
            ({"points": [{"latitude_deg": 0, "longitude_deg": 0},
                          {"latitude_deg": 0, "longitude_deg": 1}]}, "exceeds 20 km"),
            ({"points": [{"latitude_deg": float("nan"), "longitude_deg": 0},
                          {"latitude_deg": 0, "longitude_deg": 0}]}, "invalid corridor latitude"),
            ({"outer_half_width_m": 49}, "invalid corridor widths"),
            ({"source_url": ""}, "needs source"),
        ]:
            entry["influence_corridor"] = {**corridor, **replacement}
            with self.assertRaisesRegex(ValueError, error):
                BUILDER.compile_catalog(catalog)
        entry["influence_corridor"] = corridor
        entry["influence_center"] = {"latitude_deg": 0, "longitude_deg": 0,
                                     "source_url": "https://example.com/survey", "note": "Conflicting shape."}
        with self.assertRaisesRegex(ValueError, "one geographic area representation"):
            BUILDER.compile_catalog(catalog)

    def test_corridor_snapshots_are_paired_hashed_and_confined_to_assets(self):
        # Real pinned sources exercise the compiler; temporary files isolate the
        # path boundary, including symlinks, without touching source snapshots.
        corridors = [r["influence_corridor"] for r in self.rows if "influence_corridor" in r]
        self.assertTrue(corridors)
        for corridor in corridors:
            BUILDER.verify_source_snapshot(corridor, "fixture")
        source = corridors[0]
        catalog = copy.deepcopy(self.catalog)
        entry = next(r for r in catalog["places"] if "influence_corridor" in r)
        entry["influence_corridor"]["source_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            BUILDER.compile_catalog(catalog)
        for change, error in [
            ({"source_snapshot": "../places.json"}, "invalid corridor snapshot path"),
            ({"source_snapshot": str(BUILDER.CATALOG)}, "invalid corridor snapshot path"),
            ({"source_snapshot": "osm/../places.json"}, "invalid corridor snapshot path"),
            ({"source_snapshot": "osm\\fixture.json"}, "invalid corridor snapshot path"),
            ({"source_snapshot": None}, "invalid corridor snapshot path"),
            ({"source_snapshot": "osm/missing-fixture.json"}, "snapshot is missing"),
            ({"source_sha256": "0" * 64}, "checksum mismatch"),
            ({"source_sha256": "no-checksum"}, "invalid corridor snapshot checksum"),
        ]:
            with self.assertRaisesRegex(ValueError, error):
                BUILDER.verify_source_snapshot({**source, **change}, "fixture")
        for key in ["source_snapshot", "source_sha256"]:
            incomplete = dict(source)
            del incomplete[key]
            with self.assertRaisesRegex(ValueError, "must be paired"):
                BUILDER.verify_source_snapshot(incomplete, "fixture")
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            assets = parent / "assets"
            assets.mkdir()
            outside = parent / "outside.json"
            outside.write_text("{}")
            (assets / "escape.json").symlink_to(outside)
            with mock.patch.object(BUILDER, "ASSETS", assets):
                with self.assertRaisesRegex(ValueError, "escapes assets"):
                    BUILDER.verify_source_snapshot({"source_snapshot": "escape.json",
                                                      "source_sha256": BUILDER.sha256(outside)}, "fixture")

    def test_corridor_index_covers_segment_sides_bends_and_dateline_endcaps(self):
        # Independent spherical navigation samples the whole finite line, then
        # its width in every direction. Midpoint caps may over-index, never miss.
        def destination(lat, lon, angle, bearing):
            target_lat = math.asin(math.sin(lat)*math.cos(angle) + math.cos(lat)*math.sin(angle)*math.cos(bearing))
            target_lon = lon + math.atan2(math.sin(bearing)*math.sin(angle)*math.cos(lat),
                                         math.cos(angle)-math.sin(lat)*math.sin(target_lat))
            return target_lat, target_lon
        for line in [[(0, 179.99), (0, -179.99), (0.02, -179.99)],
                     [(55.679, 12.588), (55.677, 12.596)],
                     [(89.99, 178), (89.99, -178)],
                     [(-2.01, 1.99), (-1.99, 2.01)]]:
            area = {"points": [{"latitude_deg": lat, "longitude_deg": lon} for lat, lon in line],
                    "outer_half_width_m": 150}
            cells = set(BUILDER.spatial_cells(area))
            for (lat_a, lon_a), (lat_b, lon_b) in zip(line, line[1:]):
                lat_a, lon_a, lat_b, lon_b = map(math.radians, (lat_a, lon_a, lat_b, lon_b))
                bearing = math.atan2(math.sin(lon_b-lon_a)*math.cos(lat_b),
                                    math.cos(lat_a)*math.sin(lat_b)-math.sin(lat_a)*math.cos(lat_b)*math.cos(lon_b-lon_a))
                hav = math.sin((lat_b-lat_a)/2)**2 + math.cos(lat_a)*math.cos(lat_b)*math.sin((lon_b-lon_a)/2)**2
                angle = 2*math.asin(math.sqrt(hav))
                for fraction in [0, 0.1, 0.5, 0.9, 1]:
                    lat, lon = destination(lat_a, lon_a, angle*fraction, bearing)
                    for direction in range(36):
                        qlat, qlon = destination(lat, lon, 149.999/BUILDER.EARTH_RADIUS_M, direction*math.tau/36)
                        qlon = (math.degrees(qlon)+180)%360-180
                        cell = min(89, int((math.degrees(qlat)+90)//2))*180 + min(179, int((qlon+180)//2))
                        self.assertIn(cell, cells, (line, fraction, direction))
            if abs(line[0][0]) < 80:
                self.assertLessEqual(len(cells), 4, "A local corridor must not index a world-spanning longitude range")

    def test_real_polygon_generated_bases_typecheck_as_f64(self):
        rows = [row for row in self.rows if any("polygons" in area for area in row["areas"])]
        self.assertTrue(rows)
        generated = BUILDER.generate(rows)
        vectors = re.findall(r"(?:center|east|north): (\[[^\]]+\])", generated)
        self.assertTrue(vectors)
        # Exercise Rust's literal type rules on the actual source-derived bases;
        # integer zero in a generated f64 vector otherwise breaks the game build.
        code = "const BASES: &[[f64; 3]] = &[" + ",".join(vectors) + "]; fn main() { let _ = BASES; }"
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(["rustc", "--edition", "2024", "--crate-name", "polygon_vector_contract",
                                     "--emit", "metadata", "--out-dir", directory, "-"],
                                    input=code, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_pinned_polygons_preserve_holes_components_and_local_dateline_index(self):
        exterior = [[179.99, -0.01], [-179.99, -0.01], [-179.99, 0.01], [179.99, 0.01], [179.99, -0.01]]
        hole = [[179.995, -0.002], [-179.995, -0.002], [-179.995, 0.002], [179.995, 0.002], [179.995, -0.002]]
        second = [[179.95, 0.02], [179.96, 0.02], [179.96, 0.03], [179.95, 0.03], [179.95, 0.02]]
        geometry = {"type": "MultiPolygon", "coordinates": [[exterior, hole], [second]]}
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(BUILDER, "ASSETS", Path(directory)):
            path = Path(directory) / "boundary.json"
            def compile_shape(shape):
                path.write_text(json.dumps(shape))
                return BUILDER.compile_polygon({"source_snapshot": path.name, "source_sha256": BUILDER.sha256(path),
                    "source_url": "https://example.com/boundary", "note": "Synthetic shape fixture.",
                    "boundary_falloff_m": 20}, "fixture")
            result = compile_shape({"type": "Feature", "geometry": geometry, "properties": {}})
            self.assertEqual([list(map(len, p["rings"])) for p in result["polygons"]], [[5, 5], [5]])
            cells = set(BUILDER.spatial_cells(result))
            self.assertLessEqual(len(cells), 4)
            def project(p, lat, lon):
                lat, lon = map(math.radians, [lat, lon])
                q = [math.cos(lat)*math.cos(lon), math.cos(lat)*math.sin(lon), math.sin(lat)]
                denominator = sum(a*b for a,b in zip(q, p["center"]))
                return tuple(BUILDER.EARTH_RADIUS_M*sum(a*b for a,b in zip(q, p[axis]))/denominator for axis in ["east", "north"])
            first = result["polygons"][0]
            hole_center = project(first, 0, 180)
            self.assertTrue(BUILDER.point_in_ring(hole_center, first["rings"][0]))
            self.assertTrue(BUILDER.point_in_ring(hole_center, first["rings"][1]))
            for component, compiled in zip(geometry["coordinates"], result["polygons"]):
                focus = project(compiled, *compiled["representative_center"])
                self.assertTrue(BUILDER.point_in_ring(focus, compiled["rings"][0]))
                self.assertFalse(any(BUILDER.point_in_ring(focus, h) for h in compiled["rings"][1:]))
                # Ring vertices and dense interior samples exercise the cap's
                # longitude wrapping and inclusion of the polygon interior.
                samples = [(lat, lon) for ring in component for lon,lat in ring]
                for x in range(-9, 10):
                    for y in range(-9, 10):
                        samples.append((y/1000, (180+x/1000+180)%360-180))
                for lat, lon in samples:
                    point = project(compiled, lat, lon)
                    if any((lat, lon) == (c[1], c[0]) for ring in component for c in ring) or BUILDER.point_in_ring(point, compiled["rings"][0]):
                        cell = min(89, int((lat+90)//2))*180 + min(179, int((((lon+180)%360)-180+180)//2))
                        self.assertIn(cell, cells)
            for bad, error in [
                ({"type": "FeatureCollection", "features": []}, "expected one GeoJSON"),
                ({"type": "Polygon", "coordinates": [[*exterior[:-1]]]}, "rings must be closed"),
                ({"type": "Polygon", "coordinates": [[[0,0],[1,0],[1,1],[0,0]]]}, "exceeds 20 km"),
                ({"type": "Polygon", "coordinates": [[[0,0],[0,0],[0,0],[0,0]]]}, "duplicate consecutive"),
                ({**geometry, "crs": {"name": "EPSG:27700"}}, "requires WGS84"),
            ]:
                with self.assertRaisesRegex(ValueError, error):
                    compile_shape(bad)

    def test_spherical_cap_index_covers_random_edges_and_polar_dateline_cases(self):
        rng = random.Random(0xA7_1A5)
        points = [cap for row in self.rows for area in row["areas"] if "points" not in area
                  for cap in area.get("polygons", [area])] + [
            {"latitude_deg": 0.0, "longitude_deg": 179.999, "outer_radius_m": 20000},
            {"latitude_deg": 89.99, "longitude_deg": -178.0, "outer_radius_m": 20000},
            {"latitude_deg": -89.99, "longitude_deg": 42.0, "outer_radius_m": 20000},
        ]
        for point in points:
            cells = set(BUILDER.spatial_cells(point))
            lat, lon = map(math.radians, [point["latitude_deg"], point["longitude_deg"]])
            for _ in range(100):
                angular = point["outer_radius_m"] / BUILDER.EARTH_RADIUS_M * rng.random()
                bearing = rng.random() * math.tau
                target_lat = math.asin(math.sin(lat) * math.cos(angular)
                                      + math.cos(lat) * math.sin(angular) * math.cos(bearing))
                target_lon = lon + math.atan2(math.sin(bearing) * math.sin(angular) * math.cos(lat),
                                             math.cos(angular) - math.sin(lat) * math.sin(target_lat))
                target_lon = (math.degrees(target_lon) + 180) % 360 - 180
                row = min(89, int((math.degrees(target_lat) + 90) // 2))
                col = min(179, int((target_lon + 180) // 2))
                self.assertIn(row * 180 + col, cells, point)


if __name__ == "__main__":
    unittest.main()
