#!/usr/bin/env python3
"""Bake indexed landmark geometry directly from the recorded source meshes.

The LMS1 payload stores three independent triangle LODs in model-space metres.
Each little-endian level starts with u32 vertex/index counts, then 28-byte
position/normal/material-colour vertices (<6f4B) and u32 indices. Its zlib
wrapper bounds runtime decode and preserves the source palette/material IDs.
No voxel grid or sampled occupancy participates in this export.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import struct
import zlib
from pathlib import Path
import numpy as np
import trimesh
import landmark_sources as source

ROOT = Path(__file__).resolve().parents[1]
AUTHORED = {
 'sydney-opera-house': ('opera','sydney-opera-house/sydney_opera_house.glb'),
 'angkor-wat':('angkor-wat','angkor-wat/angkorwat.glb'),
 'buckingham-palace':('buckingham-palace','buckingham-palace/buckingham.glb'),
 'burj-al-arab':('burj-al-arab','burj-al-arab/burjalarab.glb'),
 'burj-khalifa':('burj-khalifa-authored','burj-khalifa/burjkhalifa.glb'),
 'brooklyn-bridge':('brooklyn-bridge-authored','brooklyn-bridge/brooklyn.glb'),
 'chrysler-building':('chrysler-building','chrysler-building/chrysler.glb'),
 'cn-tower':('cn-tower','cn-tower/cntower.glb'),
 'flatiron-building':('flatiron-building','flatiron-building/flatiron.glb'),
 'golden-gate-bridge':('golden-gate-bridge-authored','golden-gate-bridge/goldengate.glb'),
 'golden-temple':('golden-temple','golden-temple/goldentemple.glb'),
 'himeji-castle':('himeji-castle','himeji-castle/himeji.glb'),
 'moscow-kremlin':('kremlin','kremlin/kremlin.glb'),
 'leaning-tower-of-pisa':('pisa','leaning-tower-of-pisa/pisa.glb'),
 'louvre-pyramid':('louvre-pyramid','louvre-pyramid/louvre.glb'),
 'marina-bay-sands':('marina-bay-sands','marina-bay-sands/mbs.glb'),
 'merdeka-118':('merdeka-118','merdeka-118/merdeka118.glb'),
 'notre-dame':('notre-dame','notre-dame/notredame.glb'),
 'nyhavn':('nyhavn','nyhavn/nyhavn.glb'),
 'one-world-trade-center':('one-world-trade-center','one-world-trade-center/onewtc.glb'),
 'parthenon':('parthenon','parthenon/parthenon.glb'),
 'petronas-towers':('petronas-towers','petronas-towers/petronas.glb'),
 'st-basils-cathedral':('st-basils','st-basils-cathedral/stbasils.glb'),
 'steinway-tower':('steinway-tower','steinway-tower/steinway.glb'),
 'taipei-101':('taipei-101','taipei-101/taipei101.glb'),
 'the-gherkin':('the-gherkin','the-gherkin/gherkin.glb'),
 'the-shard':('the-shard','the-shard/shard.glb'),
 'tower-bridge':('tower-bridge-authored','tower-bridge/towerbridge.glb'),
 'white-house':('white-house','white-house/whitehouse.glb'),
 'palace-of-westminster':('parliament','palace-of-westminster/westminster.glb'),
 'tokyo-tower':('tokyo-tower','tokyo-tower/tokyotower.glb'),
}
RECORDED = {'statue-of-liberty':'liberty',
 'christ-the-redeemer':'christ','london-eye':'london-eye','space-needle':'space-needle',
 'giza-necropolis':'giza','colosseum':'colosseum','brandenburg-gate':'brandenburg',
 'eiffel-tower':'eiffel','empire-state-building':'empire-state','arc-de-triomphe':'arc'}
VERTEX = np.dtype([('position','<f4',(3,)),('normal','<f4',(3,)),('color','u1',(4,))])


def digest(path):
 return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def analytic_gateway():
 # Exact section of the registered catenary solid, including the 100 m cut.
 def section(y):
  return np.arccosh((192.0-y)/42.2+1.0)*40.0,8.2-5.6*(y/192.0)
 lo,hi=0.0,192.0
 for _ in range(60):
  y=(lo+hi)*0.5;center,radius=section(y)
  if center>radius:lo=y
  else:hi=y
 merge=(lo+hi)*0.5
 meshes=[]
 def loft(heights,side):
  vertices=[]
  for y in heights:
   center,radius=section(y)
   if side:
    a,b=sorted([side*center-radius,side*center+radius]);a=max(a,-100.0);b=min(b,100.0)
   else:a,b=-center-radius,center+radius
   for x,z in [(-radius,a),(radius,a),(radius,b),(-radius,b)]:vertices.append([x,y,z])
  faces=[]
  for station in range(len(heights)-1):
   for edge in range(4):
    a=station*4+edge;b=station*4+(edge+1)%4;faces.extend([[a,b,b+4],[a,b+4,a+4]])
  faces.extend([[0,2,1],[0,3,2]])
  n=(len(heights)-1)*4;faces.extend([[n,n+1,n+2],[n,n+2,n+3]])
  mesh=trimesh.Trimesh(vertices=vertices,faces=faces,process=False)
  trimesh.repair.fix_normals(mesh,multibody=True);meshes.append((mesh,'gateway'))
 heights=merge*(1.0-(1.0-np.linspace(0.0,1.0,193))**2)
 loft(heights,-1);loft(heights,1);loft(np.linspace(merge,192.0,17),0)
 return meshes


def analytic_harbour():
 # Registered model's metric curves and member dimensions; no pitch fattening.
 parts=[]
 def add(mesh,name): parts.append((mesh,name))
 def box(center,extents,name):
  add(trimesh.creation.box(extents=extents,transform=trimesh.transformations.translation_matrix(center)),name)
 def beam(a,b,radius,name):
  add(trimesh.creation.cylinder(radius=radius,segment=np.asarray([a,b]),sections=8),name)
 def deck(z): return 49.0-21.0*(max(abs(z)-420.0,0.0)/148.0)**2
 zs=np.linspace(-568.0,568.0,285)
 vertices=[]
 for z in zs:
  for x,dy in [(-21.,-3.),(21.,-3.),(21.,3.),(-21.,3.)]:vertices.append([x,deck(z)+dy,z])
 faces=[]
 for i in range(len(zs)-1):
  for side in range(4):
   a=i*4+side;b=i*4+(side+1)%4;faces.extend([[a,b,b+4],[a,b+4,a+4]])
 faces.extend([[0,2,1],[0,3,2]])
 n=(len(zs)-1)*4;faces.extend([[n,n+1,n+2],[n,n+2,n+3]])
 deck_mesh=trimesh.Trimesh(vertices=vertices,faces=faces,process=False)
 trimesh.repair.fix_normals(deck_mesh,multibody=True);add(deck_mesh,'deck')
 for x in [-14.,14.]:
  for height,drop,r in [(134.,85.,3.),(116.,67.,2.7)]:
   zs=np.linspace(-260.,260.,131)
   for a,b in zip(zs,zs[1:]):beam([x,height-drop*(a/251.5)**2,a],[x,height-drop*(b/251.5)**2,b],r,'steel')
  for z in np.arange(-252.,253.,18.):
   bot=116.-67.*(z/251.5)**2;top=134.-85.*(z/251.5)**2
   box([x,(bot+top)/2,z+0.8],[6.,top-bot,1.6],'steel')
   if bot>52.:box([x,(52.+bot)/2,z+0.55],[2.2,bot-52.,1.1],'steel')
 for z in np.arange(-252.,253.,36.):
  y=134.-85.*(z/251.5)**2;box([0.,y,z+0.8],[28.,3.2,1.6],'steel')
 for z in [-270.,270.]:
  for x in [-19.,19.]:
   box([x,41.,z],[18.,82.,26.],'granite');box([x,85.5,z],[14.,7.,22.],'granite')
 for z in np.arange(-540.,541.,60.):
  if abs(z)<=290.:continue
  for x in [-18.,18.]:beam([x,0.,z],[x,47.,z],2.6,'granite')
 return parts


def encode_level(meshes,kind,ratio):
 records=[];all_indices=[];base=0
 for mesh,material in meshes:
  if not len(mesh.faces):continue
  current=mesh
  if ratio<1.0 and len(mesh.faces)>64:
   # Independent source components cannot collapse into a neighbouring roof,
   # material group, bridge void, or window opening.
   target=max(32,int(len(mesh.faces)*ratio))
   reduced=mesh.copy()
   reduced.merge_vertices(merge_tex=True,merge_norm=True)
   current=reduced.simplify_quadric_decimation(face_count=target,aggression=7)
  vertices=np.asarray(current.vertices,dtype=np.float64)
  faces=np.asarray(current.faces,dtype=np.int64)
  normal=np.asarray(current.face_normals,dtype=np.float64)
  good=np.all(np.isfinite(vertices[faces]),axis=(1,2)) & (current.area_faces>1e-12)
  faces=faces[good];normal=normal[good]
  centers=vertices[faces].mean(axis=1)
  if kind=='gateway':colors=np.tile([204,208,214,16],(len(faces),1))
  elif kind=='harbour':colors=np.tile({'granite':[188,172,148,8],'steel':[92,96,102,16],'deck':[83,86,92,16]}[material],(len(faces),1))
  else:
   palette=source.PALETTES[kind]
   indexes=[source.palette_for_point(kind,p,material) for p in centers]
   colors=np.asarray([(*palette[i][1],palette[i][0]) for i in indexes],dtype=np.uint8)
  corner=np.empty(len(faces)*3,dtype=VERTEX)
  corner['position']=vertices[faces].reshape(-1,3)
  corner_normals=np.asarray(current.vertex_normals,dtype=np.float64)[faces].copy()
  length=np.linalg.norm(corner_normals,axis=2)
  valid=np.isfinite(corner_normals).all(axis=2)&(length>1e-8)
  # Opposing or degenerate source normals can cancel at shared vertices.
  # Only those undefined corners use the owning finite face normal.
  corner_normals[valid]/=length[valid,None]
  corner_normals[~valid]=np.repeat(normal[:,None,:],3,axis=1)[~valid]
  corner['normal']=corner_normals.reshape(-1,3)
  corner['color']=np.repeat(colors,3,axis=0)
  unique,inverse=np.unique(corner,return_inverse=True)
  records.append(unique);all_indices.append(inverse.astype('<u4')+base);base+=len(unique)
 if not records:raise ValueError('source has no finite triangles')
 vertices=np.concatenate(records);indices=np.concatenate(all_indices)
 assert len(vertices)<=4_000_000 and len(indices)<=12_000_000
 return struct.pack('<II',len(vertices),len(indices))+vertices.tobytes()+indices.tobytes(),len(vertices),len(indices)//3


def main():
 parser=argparse.ArgumentParser()
 parser.add_argument('--models',type=Path,required=True)
 parser.add_argument('--recovered',type=Path,required=True)
 parser.add_argument('--output',type=Path,default=ROOT/'crates/vearth-gen/assets/landmarks/surfaces')
 parser.add_argument('--only',nargs='*')
 args=parser.parse_args();args.output.mkdir(parents=True,exist_ok=True)
 recovered={r['kind']:r for r in json.loads(args.recovered.read_text()) if 'path'in r}
 manifest=[]; failures=[]
 names=sorted(set(AUTHORED)|set(RECORDED)|{'washington-monument','gateway-arch','sydney-harbour-bridge'})
 for name in names:
  if args.only and name not in args.only:continue
  provenance={};kind=''
  try:
   if name=='sydney-harbour-bridge':
    kind='harbour';meshes=analytic_harbour();provenance={'source':'analytic registry bridge curves','license':'project license'}
   elif name=='gateway-arch':
    kind='gateway';meshes=analytic_gateway();provenance={'source':'analytic registry catenary','license':'project license'}
   elif name=='washington-monument':
    kind='washington';meshes=source.transformed_scene(Path('.'),kind);provenance={'source':'surveyed dimensions in converter','license':'project license'}
   else:
    if name in AUTHORED:
     kind,relative=AUTHORED[name];path=args.models/relative
     provenance={'source':relative,'sha256':digest(path),'license':'existing landmark SOURCES.md'}
    else:
     kind=RECORDED[name];r=recovered[kind];path=Path(r['path'])
     assert digest(path)==r['sha256'];provenance={key:r[key] for key in ['url','sha256']};provenance['license']='existing landmark SOURCES.md'
    meshes=source.transformed_scene(path,kind)
   levels=[];counts=[]
   for ratio in [1.0,0.35,0.10]:
    payload,nv,nt=encode_level(meshes,kind,ratio);levels.append(payload);counts.append([nv,nt])
   payload=b'LMS1'+struct.pack('<I',len(levels))+b''.join(levels)
   compressed=zlib.compress(payload,9)
   target=args.output/(name+'.lms');target.write_bytes(struct.pack('<I',len(payload))+compressed)
   record={'name':name,'kind':kind,**provenance,'output':target.name,'output_sha256':digest(target),'levels':counts}
   (args.output/(name+'.source.json')).write_text(json.dumps(record,indent=2)+'\n')
   manifest.append(record);print(name,counts,len(compressed),flush=True)
  except Exception as e:
   failures.append(name)
   print('FAILED',name,type(e).__name__,str(e),flush=True)
 if failures: raise SystemExit('Failed landmark exports: '+', '.join(failures))
 manifest=[json.loads(path.read_text()) for path in sorted(args.output.glob('*.source.json'))]
 (args.output/'export-summary.json').write_text(json.dumps(manifest,indent=2)+'\n')

if __name__=='__main__':main()
