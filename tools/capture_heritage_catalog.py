#!/usr/bin/env python3
"""Capture baked heritage landmarks in the production world with evidence hashes.

Use --only for named sites and --overview for complete fortress circuits.
The executable hash isolates generated-cell caches between modeling revisions.
"""
import argparse
from datetime import datetime, timedelta, timezone
import json
import math
import os
from pathlib import Path

from capture_city_catalog import (
    camera_pose, destination, digest, model_ground, run_logged, terrain_sample,
    validate_capture,
)

ROOT=Path(__file__).resolve().parents[1]
CATALOG=ROOT/'crates/vearth-gen/assets/landmarks/heritage-surfaces/catalog.json'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--only',nargs='+',required=True)
    parser.add_argument('--catalog', type=Path, default=CATALOG)
    parser.add_argument('--app',type=Path,default=ROOT/'target/release/vearth-app')
    parser.add_argument('--survey',type=Path,default=ROOT/'target/release/city-survey',help='Raw provider-ground survey executable')
    parser.add_argument('--overview',action='store_true')
    parser.add_argument('--target-xz',type=float,nargs=2)
    parser.add_argument('--target-height',type=float)
    parser.add_argument('--azimuth',type=float)
    parser.add_argument('--elevation',type=float)
    parser.add_argument('--distance',type=float)
    parser.add_argument('--output',type=Path,default=ROOT/'output/heritage/world')
    parser.add_argument('--runtime-root',type=Path,help='Isolate generated-cell cache for concurrent capture batches')
    parser.add_argument('--timeout',type=float,default=240)
    parser.add_argument('--time',help='UTC ISO timestamp; default is local solar noon')
    args=parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout<=0:
        parser.error('timeout must be finite and positive')
    if args.target_xz and not all(math.isfinite(v) for v in args.target_xz):
        parser.error('target coordinates must be finite')
    for key in ('azimuth','elevation','distance','target_height'):
        value=getattr(args,key)
        if value is not None and not math.isfinite(value):
            parser.error(key+' must be finite')
    if args.distance is not None and args.distance<=0:
        parser.error('distance must be positive')
    if args.elevation is not None and not 0<args.elevation<90:
        parser.error('elevation must be between 0 and 90 degrees')
    catalog = args.catalog.resolve()
    entries={e['name']:e for e in json.loads(catalog.read_text())}
    unknown=set(args.only)-set(entries)
    if unknown:parser.error('unbaked sites: '+', '.join(sorted(unknown)))
    output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    app=args.app.resolve();survey=args.survey.resolve()
    app_hash=digest(app)
    env=os.environ.copy();env['VEARTH_ASYNC_PROVIDERS']='0'
    runtime=args.runtime_root.resolve() if args.runtime_root else ROOT/'output/heritage/runtime'
    env['VEARTH_RUNTIME_ROOT']=str(runtime/app_hash[:16])
    for name in args.only:
        if digest(app)!=app_hash:
            raise RuntimeError('capture executable changed; restart with a fixed build')
        e=entries[name];capture=e.get('capture',{})
        low,high=e['bbox_min'],e['bbox_max']
        x,z=(0,0) if not args.overview else ((low[0]+high[0])/2,(low[2]+high[2])/2)
        if args.target_xz is not None:x,z=args.target_xz
        heading=math.radians(e['heading'])
        east=x*math.cos(heading)+z*math.sin(heading)
        north=-x*math.sin(heading)+z*math.cos(heading)
        lat,lon=destination(e['lat'],e['lon'],math.degrees(math.atan2(east,north)),math.hypot(east,north))
        distance=capture.get('distance',350)
        if args.overview:distance=max(distance,math.hypot(high[0]-low[0],high[2]-low[2])*1.5)
        recipe=dict(name=name,target_lat=lat,target_lon=lon,
            target_height_m=args.target_height if args.target_height is not None else capture.get('target_height',max(8,(high[1]+low[1])/2)),
            distance_m=args.distance or distance,
            azimuth_deg=args.azimuth if args.azimuth is not None else (e['heading']+capture.get('azimuth',215))%360,
            elevation_deg=args.elevation or capture.get('elevation',28),min_eye_agl_m=8)
        label=name+('-overview' if args.overview else '')
        if args.azimuth is not None:label+='-az'+str(args.azimuth)
        print(label+': surveying production ground',flush=True)
        ground=terrain_sample(lat,lon,label,output,survey,args.timeout,env,True)
        eye_lat,eye_lon=destination(lat,lon,recipe['azimuth_deg'],recipe['distance_m']*math.cos(math.radians(recipe['elevation_deg'])))
        eye=terrain_sample(eye_lat,eye_lon,label+'-eye',output,survey,args.timeout,env,True)
        base=model_ground(e,name,output,survey,args.timeout,env)
        pose=camera_pose(recipe,e,ground,eye,base)
        png=output/(label+'.png');report=output/(label+'.json');log=output/(label+'.log')
        at=','.join(f'{pose[k]:.10f}' for k in ('lat_deg','lon_deg','alt_m','heading_deg','pitch_deg'))
        local_noon=datetime(2026,3,20,12,tzinfo=timezone.utc)-timedelta(hours=e['lon']/15)
        render_time=args.time or local_noon.isoformat(timespec='seconds').replace('+00:00','Z')
        command=[str(app),'--screenshot',str(png),'--at',at,'--size','1600x1000',
            '--time',render_time,'--time-scale','0',
            '--surface-month','9','--weather','clear','--timeout',str(args.timeout),
            '--screenshot-report',str(report)]
        png.unlink(missing_ok=True);report.unlink(missing_ok=True)
        print(label+': rendering world',flush=True)
        elapsed=run_logged(command,log,args.timeout+120,env)
        result=validate_capture(png,report,(1600,1000))
        evidence=dict(name=name,recipe=recipe,pose=pose,command=command,elapsed_seconds=elapsed,
            app_sha256=app_hash,asset_sha256=digest(catalog.parent/(name+'.lms')),
            survey_sha256=digest(survey),
            source_sha256=digest(catalog.parent/(name+'.source.json')),
            image_sha256=digest(png),generation=result['generation'])
        (output/(label+'.capture.json')).write_text(json.dumps(evidence,indent=2)+'\n')
        print(label+': saved',flush=True)


if __name__=='__main__':
    main()
