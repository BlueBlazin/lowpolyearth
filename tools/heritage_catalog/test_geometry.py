"""Regression checks for metric heritage authoring primitives."""
import unittest
import numpy as np
from heritage_catalog.geometry import Mesh, face
from city_catalog.expansion_heritage import _face as legacy_face


class FaceTests(unittest.TestCase):
    def test_collapsed_scanline_edge_keeps_visible_roof(self):
        strip = np.array([(0, 0, 0), (0, 0, 0), (2, 0, 1), (0, 0, 1)], float)
        for axis in range(3):
            p = np.roll(strip, axis, axis=1)
            normal = np.roll([0, 1, 0], axis)
            mesh = Mesh()
            face(mesh, p, (180, 180, 180), normal=normal)
            self.assertEqual(len(mesh.triangles), 1)
            triangle = mesh.triangles[0][0]
            area = np.cross(triangle[1]-triangle[0], triangle[2]-triangle[0])
            self.assertGreater(np.dot(area, normal), 0)
            self.assertAlmostEqual(np.linalg.norm(area)/2, 1.0)

    def test_regular_face_preserves_existing_triangulation(self):
        p = [(0, 0, 0), (2, 0, 0), (2, 0, 1), (0, 0, 1)]
        for normal in [(0, 1, 0), (0, -1, 0)]:
            before, after = Mesh(), Mesh()
            legacy_face(before, p, (180, 180, 180), normal=normal)
            face(after, p, (180, 180, 180), normal=normal)
            np.testing.assert_array_equal(
                [t[0] for t in before.triangles], [t[0] for t in after.triangles])


class ExcavationBoundaryTests(unittest.TestCase):
    def test_biete_excavation_joins_across_streamed_lods(self):
        from heritage_catalog.temples import _biete

        boundaries = []
        for lod in range(3):
            boundary = [p for p, _, _ in _biete(lod).triangles
                        if np.linalg.norm(p[:, [0, 2]], axis=1).min() >= 12]
            boundaries.append(np.asarray(boundary))
        self.assertGreater(len(boundaries[0]), 100)
        for boundary in boundaries[1:]:
            np.testing.assert_array_equal(boundaries[0], boundary)

    def test_biete_outer_support_faces_outward(self):
        from heritage_catalog.temples import _biete

        skirts = [p for p, _, material in _biete(0).triangles
                  if material == 8 and np.linalg.norm(p[:, [0, 2]], axis=1).min() > 25]
        self.assertGreater(len(skirts), 0)
        for p in skirts:
            normal = np.cross(p[1]-p[0], p[2]-p[0])
            self.assertGreater(np.dot(normal[[0, 2]], p[:, [0, 2]].mean(axis=0)), 0)


if __name__ == '__main__':
    unittest.main()
