#!/usr/bin/env python3
"""Capture the nationwide Great Wall in the production world, with source reports."""
import argparse
import json
import math
import os
from pathlib import Path
from capture_city_catalog import ROOT, camera_pose, destination, terrain_sample, run_logged, validate_capture, digest

VIEWS = [
    dict(name='badaling', target_lat=40.3565273,target_lon=116.0117547,distance_m=290,azimuth_deg=175,elevation_deg=24,target_height_m=5),
    dict(name='badaling-close',target_lat=40.35479,target_lon=116.00685,distance_m=60,azimuth_deg=150,elevation_deg=18,target_height_m=5),
    dict(name='mutianyu',target_lat=40.4311994,target_lon=116.5715615,distance_m=280,azimuth_deg=150,elevation_deg=28,target_height_m=5),
    dict(name='jiayuguan',target_lat=39.8227976,target_lon=98.2073979,distance_m=240,azimuth_deg=120,elevation_deg=25,target_height_m=4),
    dict(name='shanhaiguan',target_lat=40.0100,target_lon=119.7540,distance_m=250,azimuth_deg=140,elevation_deg=26,target_height_m=5),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--only',action='append')
    parser.add_argument('--output-dir',type=Path,default=ROOT/'output/great-wall')
    args=parser.parse_args()
    output=args.output_dir.resolve();output.mkdir(parents=True,exist_ok=True)
    env=os.environ.copy();env['VEARTH_ASYNC_PROVIDERS']='0';env.pop('VEARTH_LANDMARK_CALIBRATION',None)
    for recipe in VIEWS:
        if args.only and recipe['name'] not in args.only: continue
        name=recipe['name']; recipe={**recipe,'time_utc':'2026-09-12T07:00:00Z'}
        ground=terrain_sample(recipe['target_lat'],recipe['target_lon'],name,output,ROOT/'target/release/city-survey',300,env,True)
        eye=destination(recipe['target_lat'],recipe['target_lon'],recipe['azimuth_deg'],recipe['distance_m']*math.cos(math.radians(recipe['elevation_deg'])))
        eye_ground=terrain_sample(*eye,name+'-eye',output,ROOT/'target/release/city-survey',300,env,True)
        pose=camera_pose(recipe,{'base':'terrain'},ground,eye_ground)
        at=','.join(f'{pose[k]:.10f}' for k in ('lat_deg','lon_deg','alt_m','heading_deg','pitch_deg'))
        png=output/(name+'.png'); report=output/(name+'.report.json')
        command=[str(ROOT/'target/release/vearth-app'),'--screenshot',str(png),'--at',at,'--size','1600x1000','--time',recipe['time_utc'],'--time-scale','0','--surface-month','9','--weather','clear','--timeout','300','--screenshot-report',str(report)]
        png.unlink(missing_ok=True);report.unlink(missing_ok=True)
        print(f'Rendering {name}: {at}',flush=True)
        elapsed=run_logged(command,output/(name+'.log'),420,env)
        result=validate_capture(png,report,(1600,1000))
        (output/(name+'.capture.json')).write_text(json.dumps(dict(recipe=recipe,pose=pose,command=command,elapsed_seconds=elapsed,network_sha256=digest(ROOT/'crates/vearth-gen/assets/great-wall/network.gwl'),app_sha256=digest(ROOT/'target/release/vearth-app'),generation=result['generation']),indent=2)+'\n')
        print(f'Saved {png}',flush=True)

if __name__=='__main__':main()
