"""Resample retained official DGM1 into model coordinates; no network calls.

Run python3 tools/heritage_catalog/references/castles/extract_eltz_relief.py.
Terrain: ©GeoBasis-DE / LVermGeoRP2026, dl-de/by-2-0, www.lvermgeo.rlp.de
[Daten bearbeitet].
"""
import sys,importlib.util,json,math,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
import math

def utm(lat,lon):
 a=6378137.;e2=.00669438002290;k=.9996
 p=math.radians(lat);l=math.radians(lon-9);ep=e2/(1-e2)
 N=a/math.sqrt(1-e2*math.sin(p)**2);T=math.tan(p)**2;C=ep*math.cos(p)**2;A=math.cos(p)*l
 M=a*((1-e2/4-3*e2**2/64-5*e2**3/256)*p-(3*e2/8+3*e2**2/32+45*e2**3/1024)*math.sin(2*p)+(15*e2**2/256+45*e2**3/1024)*math.sin(4*p)-35*e2**3/3072*math.sin(6*p))
 return 500000+k*N*(A+(1-T+C)*A**3/6+(5-18*T+T*T+72*C-58*ep)*A**5/120),k*(M+N*math.tan(p)*(A*A/2+(5-T+9*C+4*C*C)*A**4/24+(61-58*T+T*T+600*C-330*ep)*A**6/720))

root=Path(__file__).resolve().parent;a=np.array(Image.open(root/'eltz-dgm1.tif'))
lat,lon=50.205073020369625,7.33657543917753;anchor=np.array(utm(lat,lon));c,s=math.cos(math.radians(9)),math.sin(math.radians(9))
def sample(x,z):
 e,n=c*x+s*z,-s*x+c*z
 q=utm(lat+n/111195,lon+e/(111195*math.cos(math.radians(lat))))
 j,i=q[0]-381000-.5,5563000-q[1]-.5;u,v=j%1,i%1;j,i=int(j),int(i)
 return float((1-v)*((1-u)*a[i,j]+u*a[i,j+1])+v*((1-u)*a[i+1,j]+u*a[i+1,j+1]))
xs=list(range(-192,193,2));zs=xs
h=[[round(sample(x,z),3) for x in xs] for z in zs]
d={'source':'https://geobasis-rlp.de/data/dgm1/current/tif/dgm1_32_381_5562_1_rp_2024.tif','source_sha256':hashlib.sha256((root/'eltz-dgm1.tif').read_bytes()).hexdigest(),'metalink':'https://geobasis-rlp.de/data/dgm1/current/meta4/dgm1_tif_07.meta4','attribution':'©GeoBasis-DE / LVermGeoRP2026, dl-de/by-2-0, www.lvermgeo.rlp.de [Daten bearbeitet]','coordinate_system':'Local model xz metres at heading9deg, DHHN2016 elevations sampled bilinearly from ETRS89/UTM32N1m pixels; local equirectangular convention matches engine','anchor_lat_lon':[lat,lon],'anchor_utm32_m':anchor.tolist(),'x_m':xs,'z_m':zs,'height_m':h,'controls':[{'local_xz_m':[x,z],'height_m':round(sample(x,z),3)} for x,z in [(0,0),(0,-30),(0,30),(0,50),(0,75),(-50,0),(50,0),(-100,0),(100,0),(0,-100)]]}
(root/'eltz-relief.json').write_text(json.dumps(d,separators=(',',':'))+'\n');print(d['controls'])
