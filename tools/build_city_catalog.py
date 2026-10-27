#!/usr/bin/env python3
"""Bake the handcrafted city models into the engine's indexed LMS1 format.

Run with --check to verify reproducibility, source hashes, and runtime registry.
Reference URLs document proportions and placement; geometry is authored here.
"""
import argparse
import hashlib
import importlib
import json
from pathlib import Path
import struct
import sys
import zlib
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tools/city_catalog'
OUTPUT = ROOT / 'crates/vearth-gen/assets/landmarks/city-surfaces'
REGISTRY = ROOT / 'crates/vearth-gen/src/landmarks/city_generated.rs'
sys.path.insert(0, str(SOURCE))
VERTEX = np.dtype([('position','<f4',(3,)),('normal','<f4',(3,)),('color','u1',(4,))])


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(mesh):
    points = np.asarray([triangle[0] for triangle in mesh.triangles], dtype=np.float64)
    colors = np.asarray([triangle[1] for triangle in mesh.triangles])
    materials = np.asarray([triangle[2] for triangle in mesh.triangles])
    normal = np.cross(points[:, 1]-points[:, 0], points[:, 2]-points[:, 0])
    lengths = np.linalg.norm(normal, axis=1)
    if not np.isfinite(points).all() or not np.isfinite(lengths).all() or (lengths <= 0).any():
        raise ValueError('invalid geometry')
    normal /= lengths[:, None]
    if colors.shape != (len(points), 3) or not np.isfinite(colors).all() or (colors < 0).any() or (colors > 255).any():
        raise ValueError('invalid color')
    if (materials <= 0).any() or (materials >= 128).any():
        raise ValueError('invalid material')
    vertices = np.empty(len(points)*3, dtype=VERTEX)
    vertices['position'] = points.reshape(-1, 3)
    vertices['normal'] = np.repeat(normal, 3, axis=0)
    vertices['color'] = np.repeat(np.column_stack((colors, materials)), 3, axis=0)
    unique, inverse = np.unique(vertices, return_inverse=True)
    indices = inverse.astype('<u4')
    payload = struct.pack('<II', len(unique), len(indices))+unique.tobytes()+indices.tobytes()
    return payload, {'vertices':len(unique),'triangles':len(indices)//3}, vertices['position']


def rust_float(value):
    return repr(float(value))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    outputs={}
    entries=[]
    for module_name in ['iconic','sculptural','waterfront', 'expansion_modern','expansion_heritage','expansion_civic','outdoors']:
        module=importlib.import_module(module_name)
        for entry in module.ENTRIES:
            name=entry['name']
            max_source_span=entry.get('max_source_building_span_m')
            assert max_source_span is None or (np.isfinite(max_source_span) and max_source_span>0)
            levels=[]
            stats=[]
            points=[]
            for lod in range(3):
                data,stat,p=encode(module.build(name,lod))
                assert stat['triangles'] > 0, name
                levels.append(data);stats.append(stat);points.append(p)
            raw=b'LMS1'+struct.pack('<I',3)+b''.join(levels)
            payload=struct.pack('<I',len(raw))+zlib.compress(raw,9)
            outputs[OUTPUT/(name+'.lms')]=payload
            p=np.concatenate(points).astype(np.float64)
            low=np.round(np.floor(p.min(axis=0)*10)/10-0.2,1).tolist()
            high=np.round(np.ceil(p.max(axis=0)*10)/10+0.2,1).tolist()
            pad=float(entry.get('activation_pad',0.0))
            assert pad >= 0 and np.isfinite(pad)
            for axis in (0,2):
                low[axis]=round(low[axis]-pad,1)
                high[axis]=round(high[axis]+pad,1)
            activation=entry.get('activation_bounds_xz')
            if activation is not None:
                for j,axis in enumerate((0,2)):
                    low[axis]=min(low[axis],float(activation[0][j]))
                    high[axis]=max(high[axis],float(activation[1][j]))
            for patch in entry.get('terrain_flatten', []):
                center=np.asarray(patch['center_xz'],dtype=float)
                half=np.asarray(patch['half_extent'],dtype=float)
                feather=float(patch['feather_m'])
                clearance=float(patch['sample_clearance_m'])
                assert center.shape==(2,) and half.shape==(2,)
                assert np.isfinite(center).all() and np.isfinite(half).all() and (half>0).all()
                assert np.isfinite(feather) and feather>0 and np.isfinite(clearance) and clearance>=0
                fixed=patch.get('fixed_base_geo')
                assert fixed is None or np.isfinite(fixed)
                for j,axis in enumerate((0,2)):
                    low[axis]=min(low[axis],float(center[j]-half[j]-feather))
                    high[axis]=max(high[axis],float(center[j]+half[j]+feather))
            record={**entry,'bbox_min':low,'bbox_max':high,'levels':stats,
                'geometry':'original metric geometry authored for Vearth; visual interpretation of referenced places',
                'license':'repository LICENSE; references are not imported mesh assets',
                'source':str((SOURCE/(module_name+'.py')).relative_to(ROOT)),
                'source_sha256':digest((SOURCE/(module_name+'.py')).read_bytes()),
                'primitives_sha256':digest((SOURCE/'common.py').read_bytes()),
                'generator_sha256':digest(Path(__file__).read_bytes()),
                'output_sha256':digest(payload)}
            outputs[OUTPUT/(name+'.source.json')]=(json.dumps(record,indent=2)+'\n').encode()
            entries.append(record)
            print(name,stats,flush=True)
    assert len({e['name'] for e in entries})==len(entries)==44
    rust=['// Generated by tools/build_city_catalog.py; geographic placement and bounded source meshes.',
          'use super::{Base, Landmark, Suppress, TerrainFlatten};', 'pub(super) const CITY_MODELS: &[Landmark] = &[']
    for e in entries:
        base='Base::Terrain' if e['base']=='terrain' else f'Base::Fixed({rust_float(e["base"])})'
        suppression=e.get('suppress_bbox')
        suppression='None' if suppression is None else f'Some(({list(map(float,suppression[0]))}, {list(map(float,suppression[1]))}))'
        rust.extend(['Landmark {',f'name: "{e["name"]}",',
          f'lat_deg:{rust_float(e["lat"])}, lon_deg:{rust_float(e["lon"])}, heading_deg:{rust_float(e["heading"])},',
          f'base:{base}, bbox_min:{e["bbox_min"]}, bbox_max:{e["bbox_max"]},',
          'suppress:Suppress {'+', '.join(f'{key}:{str(e["suppress_"+key]).lower()}' for key in ['buildings','roads','trees'])+'},',
          f'suppress_bbox_xz:{suppression},', '},'])
    rust.append('];')
    rust.extend(["pub(super) fn terrain_flatten(name: &str) -> &'static [TerrainFlatten] {",'match name {'])
    for e in entries:
        patches=e.get('terrain_flatten', [])
        if not patches: continue
        rust.append('"'+e['name']+'" => &[')
        for patch in patches:
            fixed=patch.get('fixed_base_geo')
            fixed='None' if fixed is None else 'Some('+rust_float(fixed)+')'
            rust.append('TerrainFlatten {'
                +'center_xz:'+str(list(map(float,patch['center_xz'])))+','
                +'half_extent:'+str(list(map(float,patch['half_extent'])))+','
                +'feather_m:'+rust_float(patch['feather_m'])+','
                +'sample_clearance_m:'+rust_float(patch['sample_clearance_m'])+','
                +'fixed_base_geo:'+fixed+',},')
        rust.append('],')
    rust.extend(['_ => &[],','}','}'])
    rust.extend(['pub(super) fn max_source_building_span(name: &str) -> Option<f64> {','Some(match name {'])
    for e in entries:
        span=e.get('max_source_building_span_m')
        if span is not None:
            rust.append('"'+e['name']+'" => '+rust_float(span)+',')
    rust.extend(['_ => return None,','})','}'])
    rust.extend(['#[cfg(not(target_arch = "wasm32"))]', 'pub(super) fn bytes(name: &str) -> Option<(&\'static str, &\'static [u8])> {','Some(match name {'])
    for e in entries:
        name=e['name']
        rust.append(f'"{name}" => ("{name}",vearth_platform::asset_bytes!("../../assets/landmarks/city-surfaces/{name}.lms")),')
    rust.extend(['_ => return None,','})','}'])
    rust.extend(["pub(super) fn road_exclusion_polygons(name: &str) -> Option<&'static [&'static [[f64; 2]]]> {",'Some(match name {'])
    for e in entries:
        polygons=e.get('road_exclusion_polygons')
        if polygons is None: continue
        rings=[]
        for polygon in polygons:
            p=np.asarray(polygon,dtype=np.float64)
            assert p.shape[1]==2 and len(p)>=3 and np.isfinite(p).all()
            a=np.roll(p,-1,axis=0)-p; b=np.roll(p,-2,axis=0)-np.roll(p,-1,axis=0)
            cross=a[:,0]*b[:,1]-a[:,1]*b[:,0]
            assert (cross>1e-8).all() or (cross < -1e-8).all(), 'road exclusions must be strictly convex'
            assert (p[:,0]>=e['bbox_min'][0]).all() and (p[:,0]<=e['bbox_max'][0]).all()
            assert (p[:,1]>=e['bbox_min'][2]).all() and (p[:,1]<=e['bbox_max'][2]).all()
            rings.append('&'+repr(p.tolist()))
        rust.append('"'+e['name']+'" => &['+','.join(rings)+'],')
    rust.extend(['_ => return None,','})','}'])

    # rustfmt is part of the workspace toolchain; format generated source before comparison.
    import subprocess
    formatted=subprocess.run(['rustfmt','--edition','2024'],input=('\n'.join(rust)+'\n').encode(),capture_output=True,check=True).stdout
    outputs[REGISTRY]=formatted
    outputs[OUTPUT/'catalog.json']=(json.dumps(entries,indent=2)+'\n').encode()
    for path,data in outputs.items():
        if args.check:
            if not path.exists() or path.read_bytes()!=data:
                raise SystemExit(f'stale generated asset: {path.relative_to(ROOT)}')
        else:
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes(data)


if __name__=='__main__':
    main()
