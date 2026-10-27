"""Metric silhouettes and material invariants for the additional collection."""
import unittest
import numpy as np
from skyscraper_catalog import americas
from skyscraper_catalog.inventory import NAMES
from build_skyscraper_catalog import existing_names
from skyscraper_catalog.geometry import Mesh, curtain_wall
from skyscraper_catalog.americas_common import facade, rect


def roof_at(mesh,x,z):
    hits=[]
    for p,_,_ in mesh.triangles:
        a,b,c=p[:,[0,2]]
        ab,ac=b-a,c-a
        determinant=ab[0]*ac[1]-ab[1]*ac[0]
        if abs(determinant)<1e-7:continue
        q=np.array([x,z])-a
        u=(q[0]*ac[1]-q[1]*ac[0])/determinant
        v=(ab[0]*q[1]-ab[1]*q[0])/determinant
        if min(u,v)>=-1e-6 and u+v<=1+1e-6:
            hits.append(p[0,1]+u*(p[1,1]-p[0,1])+v*(p[2,1]-p[0,1]))
    return max(hits)


class AdditionalCollectionTests(unittest.TestCase):
    def test_ordinary_piers_use_native_wall_finish(self):
        for lod in range(3):
            mesh=Mesh()
            facade(mesh,rect(40,30),rect(25,18),0,300,4,3,lod)
            materials={material for _,_,material in mesh.triangles}
            self.assertEqual(materials,{87,103})

    def test_native_facade_is_one_nonoverlapping_surface(self):
        for lod in range(3):
            mesh=Mesh()
            curtain_wall(mesh,(-20,0),(20,0),0,400,4,2,
                         (100,125,140),(115,120,120),lod,facade_material=103)
            area=sum(np.linalg.norm(np.cross(p[1]-p[0],p[2]-p[0]))*.5
                     for p,_,_ in mesh.triangles)
            self.assertAlmostEqual(area,40*400,places=5)
            self.assertTrue(all(np.cross(p[1]-p[0],p[2]-p[0])[2]<0
                                for p,_,_ in mesh.triangles))

    def test_selection_has_fifty_nine_new_names(self):
        self.assertEqual(len(set(NAMES)),59)
        self.assertFalse(set(NAMES)&existing_names())

    def test_crown_silhouette_matches_architect_west_elevation(self):
        from skyscraper_catalog.crown_sydney import reference, _ring
        data=reference()
        projection=np.asarray(data['source_roof_curves']['projection_matrix'])[0]
        inverse=np.linalg.inv(np.asarray(data['plan_to_world_matrix']))
        # Independent black facade edges on drawing00915-E-3000-TP,
        # at RL212.2,257.2,267.2;1:300 scale, RL275 maps to y410.94pt.
        edges={212.2:(762.748815,1135.568121),
               257.2:(792.332252,1100.451754),
               267.2:(799.776899,1089.934661)}
        for section in data['tower_groups'][1]['sections']:
            if section['rl_m'] not in edges:continue
            expected=(np.array(edges[section['rl_m']])-919.25)/9.4488188976
            for lod in range(3):
                xz=_ring(section,data,lod)[:,[0,2]]
                uv=(xz-data['plan_translation_xz_m'])@inverse.T
                x=uv@projection
                np.testing.assert_allclose([x.min(),x.max()],expected,atol=(.75,.75,1.0)[lod],rtol=0)

    def test_crown_roof_height_and_foundations(self):
        from skyscraper_catalog.extra_sydney_chicago import _crown
        for lod in range(3):
            mesh=_crown(lod)
            points=np.concatenate([p for p,_,_ in mesh.triangles])
            self.assertAlmostEqual(points[:,1].max(),271.3,places=3)
            self.assertLess(points[:,1].min(),-.5)
            self.assertTrue(np.isfinite(points).all())

    def test_crown_frames_partition_nonplanar_facade_triangles(self):
        from skyscraper_catalog.crown_sydney import _triangle_panel
        p=np.array([[0,0,0],[1,7,2],[5,0,1]],float)
        mesh=Mesh()
        _triangle_panel(mesh,p,[(0,1),(1,2),(2,0)],(130,145,150),(140,155,160),.1)
        normal=np.cross(p[1]-p[0],p[2]-p[0])
        expected=np.linalg.norm(normal)/2
        actual=sum(np.linalg.norm(np.cross(q[1]-q[0],q[2]-q[0]))/2
                   for q,_,_ in mesh.triangles)
        self.assertAlmostEqual(actual,expected,places=7)
        for q,_,_ in mesh.triangles:
            np.testing.assert_allclose((q-p[0])@normal,0,atol=1e-10)

    def test_willis_directional_setbacks_survive_every_lod(self):
        # Surveyed northwestern/southeastern, northeastern/southwestern,
        # cruciform, and two-tube crown sections are direction-sensitive.
        probes=[(-22.86,22.86,200),(22.86,-22.86,200),
                (22.86,22.86,260),(-22.86,-22.86,260),
                (0,22.86,355),(22.86,0,355),(0,-22.86,355),
                (-22.86,0,442.1),(0,0,442.1)]
        for lod in range(3):
            mesh=americas.build('willis-tower',lod)
            for x,z,height in probes:
                self.assertAlmostEqual(roof_at(mesh,x+6,z+4),height,places=4)
            points=np.concatenate([p for p,_,_ in mesh.triangles])
            self.assertAlmostEqual(points[:,1].max(),527,places=4)
            self.assertLess(points[:,1].min(),-3)
            self.assertFalse({11,12}&{material for _,_,material in mesh.triangles})
            # The custom tower uses the surrounding city's filtered office
            # facade, so a dense per-window grid cannot reappear unnoticed.
            self.assertIn(103,{material for _,_,material in mesh.triangles})
            self.assertLess(len(mesh.triangles),10_000)


if __name__=='__main__':unittest.main()
