#!/usr/bin/env python3
"""Offline preservation bake for the landmark without its available original.

Only the named, hash-verified archived release assets are accepted. Adjacent
coplanar faces with the same material are merged without moving any boundary.
The application consumes only the resulting indexed LMS geometry; it has no
archive reader or occupancy mesher.
"""
import hashlib
import json
import struct
import zlib
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'crates/vearth-gen/assets/landmarks'
ARCHIVES = {
 'singapore-merlion': 'merlion.vxl',
}
VERTEX = np.dtype([('position','<f4',(3,)),('normal','<f4',(3,)),('color','u1',(4,))])


def rectangles(mask):
 """Merge identical horizontal runs vertically; exact integer face coverage."""
 active = {}
 for y in range(mask.shape[1] + 1):
  row = mask[:, y] if y < mask.shape[1] else np.zeros(mask.shape[0], dtype=mask.dtype)
  changes = np.flatnonzero(np.r_[True, row[1:] != row[:-1], True])
  runs = {(int(a), int(b), int(row[a])) for a, b in zip(changes, changes[1:]) if row[a]}
  for key in active.keys() - runs:
   start = active[key]
   yield (*key, start, y)
  active = {key: active.get(key, y) for key in runs}


def level_mesh(pitch, origin, dims, entries, palette):
 grid = np.zeros(dims, dtype=np.uint16)
 keys = entries['key'].astype(np.int64)
 x = keys % dims[0]; y = keys // dims[0] % dims[1]; z = keys // (dims[0] * dims[1])
 grid[x, y, z] = entries['palette'].astype(np.uint16) + 1
 vertices=[]; indices=[]; exposed_count=0; covered_count=0
 for axis in range(3):
  u = (axis+1)%3; v = (axis+2)%3
  for sign in [-1, 1]:
   for plane in range(dims[axis]):
    current = np.take(grid, plane, axis=axis)
    # np.take retains ascending axis order; transpose to cyclic u/v order.
    if u > v: current = current.T
    other_plane = plane + sign
    if 0 <= other_plane < dims[axis]:
     other = np.take(grid, other_plane, axis=axis)
     if u > v: other = other.T
     mask = np.where(other == 0, current, 0)
    else: mask = current
    exposed_count += np.count_nonzero(mask)
    for a,b,color,start,end in rectangles(mask):
     covered_count += (b-a)*(end-start)
     corners=[]
     for q,r in [(a,start),(b,start),(b,end),(a,end)]:
      p=np.zeros(3);p[axis]=plane+(sign>0);p[u]=q;p[v]=r
      corners.append(np.asarray(origin)+p*pitch)
     if sign<0:corners.reverse()
     normal=np.zeros(3);normal[axis]=sign
     mat,*rgb=palette[color-1];base=len(vertices)
     vertices.extend([(p,normal,(*rgb,mat)) for p in corners])
     indices.extend([base,base+1,base+2,base,base+2,base+3])
 assert covered_count == exposed_count
 records=np.empty(len(vertices),dtype=VERTEX)
 for i,(p,n,c) in enumerate(vertices):records[i]=(p,n,c)
 unique,inverse=np.unique(records,return_inverse=True)
 ix=inverse[np.asarray(indices,dtype=np.int64)].astype('<u4')
 return struct.pack('<II',len(unique),len(ix))+unique.tobytes()+ix.tobytes(),[len(unique),len(ix)//3]


def main():
 destination=ASSETS/'surfaces';destination.mkdir(exist_ok=True)
 for name,file in ARCHIVES.items():
  path=ASSETS/file;data=path.read_bytes();sha=hashlib.sha256(data).hexdigest()
  assert sha == '291ed2c3916ff7660063146129c5917fb2dc2ffb16d816506af0f1e7d7d27bb0', 'archived source changed'
  assert data[:6] == b'VXL1\x01\x00'
  colors,count=data[6:8];palette=np.frombuffer(data[8:8+colors*4],dtype='u1').reshape(-1,4)
  offset=8+colors*4;levels=[];stats=[]
  for i in range(count):
   pitch,*origin=struct.unpack_from('<4f',data,offset)
   dims=struct.unpack_from('<3H',data,offset+16);n=struct.unpack_from('<I',data,offset+22)[0]
   offset+=26
   entries=np.frombuffer(data,offset=offset,count=n,dtype=[('key','<u4'),('palette','u1')]);offset+=n*5
   if i<3:
    level,stat=level_mesh(pitch,origin,dims,entries,palette);levels.append(level);stats.append(stat)
  assert offset==len(data)
  raw=b'LMS1'+struct.pack('<I',len(levels))+b''.join(levels)
  output=struct.pack('<I',len(raw))+zlib.compress(raw,9)
  (destination/(name+'.lms')).write_bytes(output)
  record={'name':name,'source_archive':file,'source_archive_sha256':sha,
    'method':'offline exact exposed-boundary bake with coplanar same-material merging',
    'license':'unchanged existing landmark SOURCES.md; no replacement source imported',
    'original_status':'recorded original unavailable; archived shipped geometry preserved',
    'output':name+'.lms','output_sha256':hashlib.sha256(output).hexdigest(),'levels':stats}
  (destination/(name+'.source.json')).write_text(json.dumps(record,indent=2)+'\n')
  print(name,stats,len(output),flush=True)

if __name__=='__main__': main()
