#!/usr/bin/env python3
"""Capture the authored city catalog in the actual streamed world.

Run --survey-only first to cache source terrain and inspect camera recipes.
Default execution captures the twenty primary city views. --only also accepts
supplemental names; --include-supplemental adds Shinjuku and Fremont to a full run.
The runtime's companion report is retained beside each PNG and console log.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "crates/vearth-gen/assets/landmarks/city-surfaces/catalog.json"
SIGNS = ROOT / "crates/vearth-gen/assets/cities/signs.json"
RECIPES = ROOT / "tools/city_catalog/captures.json"


def read_json(path):
    return json.loads(Path(path).read_text())


def record_capture(path, item):
    """Serialize independent capture batches without losing finished entries."""
    path = Path(path)
    with path.with_suffix(path.suffix+".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        previous = read_json(path).get("captures",[]) if path.exists() else []
        accumulated = {entry["name"]:entry for entry in previous}
        accumulated[item["name"]] = item
        data = dict(schema=1,world=True,captures=sorted(accumulated.values(),
                    key=lambda entry:entry["recipe"]["order"]))
        with tempfile.NamedTemporaryFile(mode="w",dir=path.parent,
                prefix=path.name+".",suffix=".tmp",delete=False) as temporary:
            json.dump(data,temporary,indent=2)
            temporary.write("\n")
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary.name,path)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def destination(lat, lon, bearing, distance):
    """WGS84 local radii keep sub-kilometre camera offsets in metric units."""
    phi = math.radians(lat)
    eccentricity2 = 6.6943799901413165e-3
    denominator = 1-eccentricity2*math.sin(phi)**2
    meridian = 6378137*(1-eccentricity2)/denominator**1.5
    prime = 6378137/math.sqrt(denominator)
    angle = math.radians(bearing)
    return (lat+math.degrees(distance*math.cos(angle)/meridian),
            lon+math.degrees(distance*math.sin(angle)/(prime*math.cos(phi))))


def run_logged(command, log_path, timeout, env):
    start = time.monotonic()
    with open(log_path, "w") as log:
        log.write("Arguments: " + json.dumps([str(a) for a in command]) + "\n")
        log.flush()
        try:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            raise RuntimeError(f"process exceeded {timeout}s; see {log_path}") from exc
    if result.returncode:
        raise RuntimeError(f"process exited {result.returncode}; see {log_path}")
    return round(time.monotonic()-start, 3)


def terrain_sample(lat, lon, label, output, survey_app, timeout, env, ground_only):
    key = f"{lat:.8f},{lon:.8f}"
    cached = output / "terrain" / (hashlib.sha256(key.encode()).hexdigest()[:16]+".json")
    # Earlier sign authoring surveys contain the same provider elevation sample.
    candidates = [cached, output / f"{label}-survey.json",
                  ROOT / "output/city-catalog" / f"{label}-survey.json"]
    for path in candidates:
        if not path.exists():
            continue
        item = read_json(path)
        if not isinstance(item.get("dataset"),str) or item["dataset"].startswith("terrain-"):
            continue
        if (abs(item.get("lat", 1e9)-lat) < 1e-8 and
                abs(item.get("lon", 1e9)-lon) < 1e-8 and
                isinstance(item.get("ground_m"), (int,float)) and
                math.isfinite(item["ground_m"])):
            return {k:item[k] for k in ("lat","lon","ground_m","dataset")}
    cached.parent.mkdir(parents=True, exist_ok=True)
    command = [str(survey_app), f"{lat:.10f}", f"{lon:.10f}", str(cached)]
    if ground_only:
        command.append("--ground-only")
    run_logged(command, cached.with_suffix(".log"), timeout, env)
    item = read_json(cached)
    if not isinstance(item.get("dataset"),str) or item["dataset"].startswith("terrain-"):
        raise RuntimeError(f"provider fallback cannot validate terrain for {label}: {cached}")
    if not isinstance(item.get("ground_m"), (int,float)) or not math.isfinite(item["ground_m"]):
        raise RuntimeError(f"invalid terrain elevation for {label}: {cached}")
    return {k:item[k] for k in ("lat","lon","ground_m","dataset")}


def model_ground(entry, name, output, survey_app, timeout, env):
    """Resolve authored pad datums with the runtime's eight-point median."""
    patches = entry.get("terrain_flatten", [])
    if entry.get("base") != "terrain" or not patches:
        return None
    bases=[]
    heading=math.radians(entry["heading"])
    for index,patch in enumerate(patches):
        fixed=patch.get("fixed_base_geo")
        if fixed is not None:
            bases.append(float(fixed))
            continue
        cx,cz=patch["center_xz"]
        hx,hz=[half+patch["feather_m"]+patch["sample_clearance_m"]
               for half in patch["half_extent"]]
        heights=[]
        for j,(mx,mz) in enumerate([(cx-hx,cz-hz),(cx,cz-hz),(cx+hx,cz-hz),
                (cx+hx,cz),(cx+hx,cz+hz),(cx,cz+hz),(cx-hx,cz+hz),(cx-hx,cz)]):
            east=mx*math.cos(heading)+mz*math.sin(heading)
            north=-mx*math.sin(heading)+mz*math.cos(heading)
            lat,lon=destination(entry["lat"],entry["lon"],
                math.degrees(math.atan2(east,north)),math.hypot(east,north))
            sample=terrain_sample(lat,lon,f"{name}-pad-{index}-{j}",output,
                                  survey_app,timeout,env,True)
            heights.append(sample["ground_m"])
        bases.append(statistics.median(heights))
    return statistics.median(bases)


def camera_pose(recipe, catalog_entry, ground, eye_ground, model_base=None):
    base = catalog_entry.get("base", "terrain")
    base_alt = ground["ground_m"] if base == "terrain" else float(base)
    if model_base is not None:
        base_alt = model_base
    target_alt = base_alt+recipe["target_height_m"]
    elevation = math.radians(recipe["elevation_deg"])
    horizontal = recipe["distance_m"]*math.cos(elevation)
    lat,lon = destination(recipe["target_lat"],recipe["target_lon"],
                          recipe["azimuth_deg"],horizontal)
    altitude = target_alt+recipe["distance_m"]*math.sin(elevation)
    minimum = eye_ground["ground_m"]+recipe.get("min_eye_agl_m", 5)
    # Raise only for measured terrain. Re-aiming retains the authored target.
    raised = max(0, minimum-altitude)
    altitude = max(altitude, minimum)
    def ecef(latitude,longitude,height):
        phi,lam=math.radians(latitude),math.radians(longitude)
        n=6378137/math.sqrt(1-6.6943799901413165e-3*math.sin(phi)**2)
        return ((n+height)*math.cos(phi)*math.cos(lam),
                (n+height)*math.cos(phi)*math.sin(lam),
                (n*(1-6.6943799901413165e-3)+height)*math.sin(phi))
    eye=ecef(lat,lon,altitude)
    target=ecef(recipe["target_lat"],recipe["target_lon"],target_alt)
    delta=tuple(b-a for a,b in zip(eye,target))
    phi,lam=math.radians(lat),math.radians(lon)
    east=(-math.sin(lam),math.cos(lam),0)
    north=(-math.sin(phi)*math.cos(lam),-math.sin(phi)*math.sin(lam),math.cos(phi))
    up=(math.cos(phi)*math.cos(lam),math.cos(phi)*math.sin(lam),math.sin(phi))
    e,n,u=(sum(a*b for a,b in zip(delta,axis)) for axis in (east,north,up))
    heading=math.degrees(math.atan2(e,n))%360
    pitch=math.degrees(math.atan2(math.hypot(e,n),-u))
    if not 0 <= pitch < 90:
        raise RuntimeError(f"camera for {recipe['name']} points above horizon")
    return dict(lat_deg=lat,lon_deg=lon,alt_m=altitude,heading_deg=heading,
                pitch_deg=pitch,target_alt_m=target_alt,model_base_m=base_alt,terrain_raise_m=raised,
                eye_ground_m=eye_ground["ground_m"],target_ground_m=ground["ground_m"],
                target_dataset=ground["dataset"],eye_dataset=eye_ground["dataset"])


def validate_capture(png, report_path, size):
    with open(png, "rb") as stream:
        head = stream.read(24)
    if len(head)!=24 or head[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">II",head[16:24]) != size:
        raise RuntimeError(f"invalid PNG or unexpected dimensions: {png}")
    report = read_json(report_path)
    if (report.get("surface",{}).get("width"),report.get("surface",{}).get("height")) != size:
        raise RuntimeError(f"invalid runtime report: {report_path}")
    missing = report.get("generation",{}).get("missing_cells")
    if missing is None or missing != 0:
        raise RuntimeError(f"world capture has {missing} missing cells: {report_path}")
    # Runtime camera_agl_m uses the containing cell's maximum elevation, which
    # includes neighboring skyscrapers. Point clearance is surveyed separately.
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", action="append", help="name or comma-separated names")
    parser.add_argument("--app", type=Path, default=ROOT/"target/release/vearth-app")
    parser.add_argument("--survey-app", type=Path, default=ROOT/"target/debug/city-survey")
    parser.add_argument("--output-dir", type=Path, default=ROOT/"output/city-catalog")
    parser.add_argument("--recipes", type=Path, default=RECIPES)
    parser.add_argument("--size", default="1600x1000")
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--survey-only", action="store_true")
    parser.add_argument("--include-supplemental", action="store_true")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be a positive finite number of seconds")
    size = tuple(int(value) for value in args.size.split("x"))
    if len(size)!=2 or min(size)<=0:
        parser.error("--size must be WIDTHxHEIGHT with positive dimensions")
    output = args.output_dir.resolve();output.mkdir(parents=True,exist_ok=True)
    recipes = read_json(args.recipes)["captures"]
    selected = {name for names in args.only or [] for name in names.split(",")}
    unknown = selected-{r["name"] for r in recipes}
    if unknown:
        parser.error("unknown capture names: "+", ".join(sorted(unknown)))
    recipes = [r for r in recipes if r["name"] in selected] if selected else [
        r for r in recipes if r["primary"] or args.include_supplemental]
    catalog = {e["name"]:e for e in read_json(CATALOG)}
    districts = {e["name"]:e for e in read_json(SIGNS)["districts"]}
    env = os.environ.copy();env["VEARTH_ASYNC_PROVIDERS"]="0"
    manifest_path=output/("capture-poses.json" if args.survey_only else "capture-results.json")
    results=[]
    for recipe in recipes:
        name = recipe["name"]
        print(f"[{recipe['order']:02}] {name}: surveying camera",flush=True)
        entry = catalog.get(name, districts.get(name))
        if entry is None:
            raise RuntimeError(f"capture {name} has no runtime catalog entry")
        ground = terrain_sample(recipe["target_lat"],recipe["target_lon"],name,
                    output,args.survey_app.resolve(),args.timeout,env,True)
        horizontal = recipe["distance_m"]*math.cos(math.radians(recipe["elevation_deg"]))
        lat,lon = destination(recipe["target_lat"],recipe["target_lon"],
                              recipe["azimuth_deg"],horizontal)
        eye_ground = terrain_sample(lat,lon,name+"-eye",output,args.survey_app.resolve(),
                                     args.timeout,env,True)
        base = model_ground(entry,name,output,args.survey_app.resolve(),args.timeout,env)
        pose = camera_pose(recipe,entry,ground,eye_ground,base)
        png=output/f"{name}.png";report_path=output/f"{name}.json";log=output/f"{name}.log"
        at=",".join(f"{pose[k]:.10f}" for k in
                    ("lat_deg","lon_deg","alt_m","heading_deg","pitch_deg"))
        command=[str(args.app.resolve()),"--screenshot",str(png),"--at",at,
                 "--size",args.size,"--time",recipe["time_utc"],"--time-scale","0",
                 "--surface-month","9","--weather","clear","--timeout",str(args.timeout),
                 "--screenshot-report",str(report_path)]
        item=dict(name=name,city=recipe["city"],primary=recipe["primary"],
                  recipe=recipe,pose=pose,command=command,image=str(png),report=str(report_path),
                  catalog_sha256=digest(CATALOG),signs_sha256=digest(SIGNS),
                  recipes_sha256=digest(args.recipes))
        if not args.survey_only:
            print(f"[{recipe['order']:02}] {name}: rendering world",flush=True)
            item["app_sha256"]=digest(args.app.resolve())
            # Never let an earlier image masquerade as output from a failed run.
            png.unlink(missing_ok=True);report_path.unlink(missing_ok=True)
            item["wall_seconds"]=run_logged(command,log,args.timeout+120,env)
            report=validate_capture(png,report_path,size)
            item["image_sha256"]=digest(png)
            item["generation"]=report["generation"]
        results.append(item)
        record_capture(manifest_path,item)
        print(f"[{recipe['order']:02}] {name}: {'pose ready' if args.survey_only else 'saved'}",flush=True)
    print(f"{len(results)} {'camera poses' if args.survey_only else 'world screenshots'}: {output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError,ValueError,KeyError,RuntimeError) as exc:
        print(f"capture_city_catalog: {exc}",file=sys.stderr)
        sys.exit(1)
