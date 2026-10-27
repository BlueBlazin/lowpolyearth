"""Refresh production DEM samples at the exact recorded architectural plan points."""
import json,os,subprocess
from pathlib import Path
import numpy as np
P=Path(__file__).resolve().parent
os.chdir(P.parents[3])
for name in ['derawar','qaitbay','krak','amber','gwalior','kumbhalgarh','red-fort','fasil','el-morro','barajas','citadelle','hwaseong']:
 j=json.loads((P/(name+'-plan.json')).read_text());pts=[[0,0]]
 for w in j['ways']:
  t=w['tags'];p=np.array(w['xz'])
  if t.get('barrier') or t.get('historic') in ('castle','fort') or t.get('relation_id'):
   for a,b in zip(p,p[1:]):
    count=max(1,int(np.linalg.norm(b-a)/20))
    for k in range(count):pts.append((a+(b-a)*k/count).tolist())
  pts.append(p.mean(axis=0).tolist())
  for a in [p.min(axis=0),p.max(axis=0)]:pts.append(a.tolist())
 pts=np.unique(np.round(pts,2),axis=0).tolist()
 (P/(name+'-ground-points.json')).write_text(json.dumps(pts,separators=(',',':')))
 out=P/(name+'-terrain.json')
 env=os.environ.copy();env['VEARTH_ASYNC_PROVIDERS']='0'
 cmd=['target/release/city-survey',*map(str,j['anchor']),str(out),'--ground-points',str(P/(name+'-ground-points.json')),'--ground-only']
 p=subprocess.run(cmd,env=env,timeout=240,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
 print(name,len(pts),p.returncode,p.stdout[-350:],flush=True)
