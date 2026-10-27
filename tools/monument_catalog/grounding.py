"""Bake foundation contacts against frozen terrain without changing the terrain."""
import json
from pathlib import Path

import numpy as np
from city_catalog.common import Mesh

GROUND = Path(__file__).parent/'references/ground'


class Survey:
    def __init__(self, entry):
        self.path = GROUND/(entry['name']+'.json')
        self.data = json.loads(self.path.read_text())
        for key in ('name', 'lat', 'lon', 'heading'):
            assert self.data[key] == entry[key], (entry['name'], 'stale survey', key)
        self.x = np.asarray(self.data['x_m'])
        self.z = np.asarray(self.data['z_m'])
        self.heights = np.asarray(self.data['heights_m'])
        self.base = float(self.data['ground_m'])
        assert self.heights.shape == (len(self.z), len(self.x))
        assert np.isfinite(self.heights).all()

    def height(self, xz):
        p = np.asarray(xz)
        x, z = p[..., 0], p[..., 1]
        assert ((x >= self.x[0]) & (x <= self.x[-1])).all()
        assert ((z >= self.z[0]) & (z <= self.z[-1])).all()
        i = np.clip(np.searchsorted(self.x, x)-1, 0, len(self.x)-2)
        j = np.clip(np.searchsorted(self.z, z)-1, 0, len(self.z)-2)
        u = (x-self.x[i])/(self.x[i+1]-self.x[i])
        v = (z-self.z[j])/(self.z[j+1]-self.z[j])
        return ((1-u)*(1-v)*self.heights[j, i]+u*(1-v)*self.heights[j, i+1]
                +(1-u)*v*self.heights[j+1, i]+u*v*self.heights[j+1, i+1])


def components(module, name, lod):
    return module.build_components(name, lod) if hasattr(module, 'build_components') else [module.build(name, lod)]


def support_shifts(parts, survey, entry=None):
    shifts = []
    for part in parts:
        p = np.concatenate([t[0] for t in part.triangles])
        feet = p[p[:, 1] <= .05]
        assert len(feet), 'every rigid component must have architectural feet'
        shifts.append(float(survey.height(feet[:, [0, 2]]).max()-survey.base))
    # A surveyed gate may embed its uphill lower shaft to preserve realistic
    # proportions, provided its documented walking clearance remains open.
    override = (entry or {}).get('surveyed_component_shifts_m')
    if override is not None:
        assert len(override) == len(shifts) and np.isfinite(override).all()
        return list(override)
    return shifts


def fit(parts, shifts, survey):
    """Keep connected upper structures rigid; extend only existing buried toes.

    The same LOD0-derived shifts apply to every LOD. Independent stones receive
    independent heights; lintel-connected stone groups retain their alignment.
    No surface, slab, or terrain correction is created between the feet.
    """
    assert len(parts) == len(shifts)
    mesh = Mesh()
    for part, shift in zip(parts, shifts):
        for points, color, material in part.triangles:
            fitted = points.copy()
            fitted[:, 1] += shift
            toes = points[:, 1] < -.001
            if toes.any():
                fitted[toes, 1] = points[toes, 1] + np.minimum(shift,
                    survey.height(points[toes][:, [0, 2]])-survey.base-.5)
            mesh.triangles.append((fitted, color, material))
    return mesh
