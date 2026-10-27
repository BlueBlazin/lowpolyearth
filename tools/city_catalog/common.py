"""Metric geometry primitives for the authored city catalog's LMS assets."""
import math
import numpy as np


class Mesh:
    def __init__(self):
        self.triangles = []

    def triangle(self, a, b, c, color, material=11):
        points = np.asarray([a, b, c], dtype=float)
        if not np.isfinite(points).all():
            raise ValueError("non-finite authored point")
        if np.linalg.norm(np.cross(points[1]-points[0], points[2]-points[0])) < 1e-9:
            return
        self.triangles.append((points, tuple(color), material))

    def quad(self, a, b, c, d, color, material=11):
        self.triangle(a, b, c, color, material)
        self.triangle(a, c, d, color, material)

    def box(self, center, size, color, material=11):
        c, r = np.asarray(center), np.asarray(size)*0.5
        if np.any(r <= 0):
            raise ValueError("box dimensions must be positive")
        p = [c+r*np.asarray(v) for v in [(-1,-1,-1),(1,-1,-1),(1,1,-1),(-1,1,-1),(-1,-1,1),(1,-1,1),(1,1,1),(-1,1,1)]]
        for face in [(0,3,2,1),(4,5,6,7),(0,4,7,3),(1,2,6,5),(0,1,5,4),(3,7,6,2)]:
            self.quad(*(p[i] for i in face), color, material)

    def beam(self, a, b, radius, color, material=16, segments=8):
        a, b = np.asarray(a, dtype=float), np.asarray(b, dtype=float)
        axis = b-a
        length = np.linalg.norm(axis)
        if length < 1e-8 or radius <= 0:
            raise ValueError("beam must have positive length and radius")
        axis /= length
        side = np.cross(axis, [0,1,0] if abs(axis[1]) < 0.9 else [1,0,0])
        side /= np.linalg.norm(side)
        up = np.cross(axis, side)
        ring = [radius*(math.cos(i*math.tau/segments)*side+math.sin(i*math.tau/segments)*up) for i in range(segments)]
        for i in range(segments):
            u,v=ring[i],ring[(i+1)%segments]
            self.quad(a+u,a+v,b+v,b+u,color,material)
            self.triangle(a,a+v,a+u,color,material)
            self.triangle(b,b+u,b+v,color,material)

    def ellipsoid(self, center, radii, color, material=16, segments=24, rings=12):
        center,radii=np.asarray(center),np.asarray(radii)
        def point(i,j):
            p=-math.pi/2+math.pi*i/rings
            t=math.tau*j/segments
            return center+radii*np.array([math.cos(p)*math.cos(t),math.sin(p),math.cos(p)*math.sin(t)])
        for i in range(rings):
            for j in range(segments):
                self.quad(point(i,j),point(i+1,j),point(i+1,j+1),point(i,j+1),color,material)

    def merge(self, other):
        self.triangles.extend(other.triangles)

    def transform(self, matrix):
        matrix=np.asarray(matrix)
        for i,(points,color,material) in enumerate(self.triangles):
            p=np.c_[points,np.ones(3)]@matrix.T
            self.triangles[i]=(p[:,:3],color,material)
        return self
