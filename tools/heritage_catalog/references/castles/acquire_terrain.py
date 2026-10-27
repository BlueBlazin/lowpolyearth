"""Acquire production ground samples around the calibrated castle anchors."""
import json
import os
from pathlib import Path
import subprocess
import sys

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[3]
sys.path.insert(0,str(ROOT/'tools'))
from heritage_catalog.castles import ENTRIES
os.chdir(ROOT)
for entry in ENTRIES:
    site=Path(entry['audit']['source_file']).stem.removesuffix('-source')
    extent={'neuschwanstein':100,'edinburgh':180,'windsor':300,'conwy':90,'malbork':420,'segovia':110,'alhambra':550,'eltz':100,'chillon':90,'bran':80,'predjama':80,'matsumoto':55,'osaka':65}[site]
    target=HERE/f'{site}-terrain.json'
    env=os.environ.copy();env['VEARTH_ASYNC_PROVIDERS']='0'
    cmd=['target/release/city-survey',str(entry['lat']),str(entry['lon']),str(target),'--ground-only','--extent',str(extent),'--ground-grid','33']
    p=subprocess.run(cmd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=240)
    print(site,p.returncode,p.stdout[-300:],flush=True)
    if p.returncode:raise SystemExit(p.returncode)
