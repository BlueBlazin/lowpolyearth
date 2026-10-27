"""Re-extract geographic plans and review plots from the retained OSM responses."""
import json,math,gzip,hashlib,xml.etree.ElementTree as E
from pathlib import Path
import numpy as np
import matplotlib;matplotlib.use('Agg')
import matplotlib.pyplot as plt
P=Path(__file__).resolve().parent
sites={'derawar':(28.768,71.3343,350),'qaitbay':(31.21384,29.88568,140),'krak':(34.7568,36.2944,190),'amber':(26.98575,75.85075,240),'gwalior':(26.2296,78.1680,2200),'kumbhalgarh':(25.1489,73.5801,7000),'red-fort':(28.6562,77.2410,650),'fasil':(12.6075,37.4695,310),'el-morro':(18.4703,-66.1237,180),'barajas':(10.4226,-75.5397,240),'citadelle':(19.5731,-72.2433,180),'hwaseong':(37.2825,127.0145,1600)}
for name,(lat,lon,extent) in sites.items():
 raw=gzip.decompress((P/(name+'-osm.xml.gz')).read_bytes());r=E.fromstring(raw);ns={n.attrib['id']:(float(n.attrib['lat']),float(n.attrib['lon'])) for n in r.findall('node')}
 phi=math.radians(lat); den=1-6.6943799901413165e-3*math.sin(phi)**2
 scale=(math.pi/180*6378137/math.sqrt(den)*math.cos(phi),math.pi/180*6378137*(1-6.6943799901413165e-3)/den**1.5)
 members={}
 for rel in r.findall("relation"):
  t={x.attrib["k"]:x.attrib["v"] for x in rel.findall("tag")}
  if t.get("historic") or t.get("building"):
   for x in rel.findall("member"):
    if x.attrib["type"]=="way":members[x.attrib["ref"]]={**t,"relation_id":rel.attrib["id"],"relation_role":x.attrib.get("role","")}
 ways=[]
 for w in r.findall('way'):
  t={x.attrib['k']:x.attrib['v'] for x in w.findall('tag')}
  t={**members.get(w.attrib['id'],{}),**t}
  relevant=t.get('building') or t.get('building:part') or t.get('barrier') in ('wall','city_wall') or t.get('historic') in ('castle','fort')
  if not relevant:continue
  pts=[((ns[x.attrib['ref']][1]-lon)*scale[0],(ns[x.attrib['ref']][0]-lat)*scale[1]) for x in w.findall('nd')]
  p=np.asarray(pts)
  if np.linalg.norm(p.mean(axis=0))>extent:continue
  if name=='hwaseong' and not (t.get('historic') or t.get('barrier')=='city_wall'):continue
  ways.append({'id':int(w.attrib['id']),'tags':t,'xz':np.round(p,3).tolist()})
 j={'source':'https://www.openstreetmap.org/api/0.6/map','retrieved':'2026-09-13','license':'OpenStreetMap contributors, ODbL 1.0','attribution':'© OpenStreetMap contributors','anchor':[lat,lon],'coordinate_system':'local east/north metres; equirectangular at anchor latitude','ways':ways,'source_archive':name+'-osm.xml.gz','source_xml_sha256':hashlib.sha256(raw).hexdigest()}
 (P/(name+'-plan.json')).write_text(json.dumps(j,separators=(',',':'))+'\n')
 fig,ax=plt.subplots(figsize=(10,10))
 for w in ways:
  p=np.array(w['xz']);t=w['tags'];barrier=t.get('barrier') or t.get('historic') in ('castle','fort')
  ax.plot(p[:,0],p[:,1],color='brown' if barrier else '#51767e',lw=1.3 if barrier else .55)
  if t.get('name:en') or t.get('name') or (name in ['qaitbay','krak','amber','fasil','citadelle','el-morro','barajas']):
   ax.text(*p.mean(axis=0),str(w['id'])+' '+t.get('name:en',t.get('name',''))[:22],fontsize=5)
 ax.set_aspect('equal');ax.set_title(name+' mapped plan: east/right, north/up');ax.grid(alpha=.2)
 ax.set_xlim(-extent,extent);ax.set_ylim(-extent,extent)
 fig.savefig(P/(name+'-plan.png'),dpi=150);plt.close(fig)
 print(name,len(ways),[(w['id'],np.ptp(np.asarray(w['xz']),axis=0).round().tolist()) for w in ways if len(w['xz'])>200],flush=True)
