#!/usr/bin/env python3
"""Compile the entire pinned OSM Great Wall relation into a spatial source pack.

Run --check to verify the source hash and byte-for-byte generated output.
Coordinates are WGS84; widths/heights are metres. No gaps are connected.
"""
import argparse
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'crates/vearth-gen/assets/great-wall'
SOURCE = OUT / 'osm-relation-318110.json.gz'
TOWERS = OUT / 'osm-nearby-towers.json.gz'
R = 6371008.8


def delta(a, b):
    return ((b[1]-a[1])*math.pi/180*R*math.cos(math.radians((a[0]+b[0])/2)),
            (b[0]-a[0])*math.pi/180*R)


def number(tags, key, default, low, high):
    value = tags.get(key, '')
    if re.fullmatch(r'\d+(\.\d+)?( m)?', value):
        return max(low, min(high, float(value.removesuffix(' m'))))
    return default


def style(tags, lon):
    ancient = bool(re.search(r'\b(Han|Qin|Jin)\b|金界壕|秦长城|汉长城', tags.get('name', '')+' '+tags.get('name:en', ''), re.I))
    earth = tags.get('material') in ('soil', 'earth', 'adobe', 'aoil') or (
        tags.get('material') not in ('stone', 'brick') and (lon < 105 or ancient))
    ruined = tags.get('ruins', 'no') != 'no' or tags.get('historic') == 'ruins' or ancient
    return int(earth) + 2*int(ruined)


def area_axis(points):
    """Recover paired sides of a narrow mapped wall footprint, never its two rims."""
    ring = points[:-1]
    i = max(range(len(ring)), key=lambda j: math.hypot(*delta(ring[0],ring[j])))
    j = max(range(len(ring)), key=lambda j: math.hypot(*delta(ring[i],ring[j])))
    if i > j: i,j=j,i
    sides = [ring[i:j+1], list(reversed(ring[j:]+ring[:i+1]))]
    lengths = [[0.] for _ in sides]
    for side, lengths_ in zip(sides,lengths):
        for a,b in zip(side,side[1:]): lengths_.append(lengths_[-1]+math.hypot(*delta(a,b)))
    longest=max(x[-1] for x in lengths)
    if longest < 30 or min(x[-1] for x in lengths)<.85*longest: return None
    count=max(2,math.ceil(longest/12))
    paired=[]
    for k in range(count+1):
        pair=[]
        for side, lengths_ in zip(sides,lengths):
            d=lengths_[-1]*k/count
            import bisect
            n=min(len(side)-2,max(0,bisect.bisect_right(lengths_,d)-1))
            t=(d-lengths_[n])/max(1e-9,lengths_[n+1]-lengths_[n])
            pair.append(tuple(side[n][axis]+t*(side[n+1][axis]-side[n][axis]) for axis in (0,1)))
        paired.append(pair)
    widths=sorted(math.hypot(*delta(a,b)) for a,b in paired[1:-1])
    width=widths[len(widths)//2]
    if not .5<width<20 or widths[-1]>35: return None
    return [tuple((a[k]+b[k])/2 for k in (0,1)) for a,b in paired], width


def generate(source):
    data = json.loads(source)
    supplement = json.loads(gzip.decompress(TOWERS.read_bytes()))
    merged = {(e['type'],e['id']):e for e in data['elements']}
    merged.update({(e['type'],e['id']):e for e in supplement['elements']})
    elements = sorted(merged.values(), key=lambda e: (e['type'], e['id']))
    segments, towers, tower_centres = [], [], {}
    stats = {'wall_ways': 0, 'wall_km': 0., 'styles_km': [0.]*4, 'skipped_ditches': 0, 'area_axes': 0}
    seen_edges = set()
    bounds = [90.,180.,-90.,-180.]

    def tower(e, points):
        tags = e.get('tags', {})
        lat = sum(p[0] for p in points)/len(points)
        lon = sum(p[1] for p in points)/len(points)
        st = style(tags, lon)
        if len(points) > 1:
            en = [delta((lat,lon),p) for p in points]
            edges = [(en[(i+1)%len(en)][0]-p[0], en[(i+1)%len(en)][1]-p[1]) for i,p in enumerate(en)]
            dx,dz = max(edges,key=lambda p: math.hypot(*p))
            angle = math.atan2(dz,dx)
            c,s = math.cos(angle),math.sin(angle)
            local = [(x*c+z*s,-x*s+z*c) for x,z in en]
            low = [min(p[i] for p in local) for i in range(2)]
            high = [max(p[i] for p in local) for i in range(2)]
            width,depth = [high[i]-low[i] for i in range(2)]
            # Large forts retain their mapped perimeter walls, not a fabricated tower box.
            if min(width,depth)<2 or max(width,depth)>35: return
            ex,nz = (low[0]+high[0])/2,(low[1]+high[1])/2
            lon += (ex*c-nz*s)/R/math.cos(math.radians(lat))*180/math.pi
            lat += (ex*s+nz*c)/R*180/math.pi
        else:
            width=depth=9. if not st&1 else 8.
            angle=0.
        key=(math.floor(lat*10000), math.floor(lon*10000))
        for iy in range(key[0]-2,key[0]+3):
            for ix in range(key[1]-2,key[1]+3):
                if any(math.hypot(*delta((lat,lon),p))<18 for p in tower_centres.get((iy,ix),[])): return
        tower_centres.setdefault(key,[]).append((lat,lon))
        height=number(tags,'height', 4. if st&2 else (6. if st&1 else 10.5),2.,18.)
        towers.append(struct.pack('<2i4fIB',round(lat*1e7),round(lon*1e7),width,depth,angle,height,e['id']&0xffffffff,st))

    # Polygon dimensions take precedence over nearby point representations.
    for e in elements:
        if e['type']!='way': continue
        tags=e.get('tags',{}); g=e.get('geometry',[])
        closed=len(g)>3 and g[0]==g[-1]
        is_tower = tags.get('man_made')=='tower' or tags.get('historic') in ('tower','city_gate') or tags.get('ruins')=='tower'
        if closed and is_tower: tower(e,[(p['lat'],p['lon']) for p in g[:-1]])
    for e in elements:
        if e['type']=='node':
            tags=e.get('tags',{})
            if tags.get('man_made')=='tower' or tags.get('historic') in ('tower','city_gate') or tags.get('ruins')=='tower':
                tower(e,[(e['lat'],e['lon'])])
    for e in elements:
        if e['type']!='way': continue
        t=e.get('tags',{}); g=e.get('geometry',[])
        if t.get('barrier')=='ditch': stats['skipped_ditches']+=1; continue
        if t.get('man_made')=='tower' or t.get('historic') in ('tower','city_gate'): continue
        if not (t.get('barrier') in ('city_wall','wall') or t.get('historic')=='citywalls'): continue
        points=[]
        for p in g:
            q=(p['lat'],p['lon'])
            if not points or math.hypot(*delta(points[-1],q))>.1: points.append(q)
        if len(points)<2: continue
        st=style(t,points[0][1])
        area_width=None
        if points[0]==points[-1]:
            axis=area_axis(points)
            if axis is not None:
                points,area_width=axis
                stats['area_axes']+=1
        width=number(t,'width',area_width or (5.6 if st&1 else 6.8),2.,10.)
        height=number(t,'height', (1.8 if st&1 else 3.2) if st&2 else (3.4 if st&1 else 7.5),.8,12.)
        directions=[]
        for a,b in zip(points,points[1:]):
            dx,dz=delta(a,b); d=math.hypot(dx,dz); directions.append((-dz/d,dx/d))
        normals=[]
        for i in range(len(points)):
            before=directions[max(0,i-1)]; after=directions[min(i,len(directions)-1)]
            if points[0]==points[-1] and i in (0,len(points)-1): before,after=directions[-1],directions[0]
            dot=before[0]*after[0]+before[1]*after[1]
            # Limit the miter at hairpins instead of creating huge lateral spikes.
            denom=max(.5,1+dot)
            normals.append(((before[0]+after[0])/denom,(before[1]+after[1])/denom))
        distance=0.
        stats['wall_ways']+=1
        for i,(a,b) in enumerate(zip(points,points[1:])):
            d=math.hypot(*delta(a,b)); key=tuple(sorted((a,b)))
            if key in seen_edges: distance+=d; continue
            seen_edges.add(key)
            count=max(1,math.ceil(d/40))
            for j in range(count):
                u,v=j/count,(j+1)/count
                aa=tuple(a[k]+(b[k]-a[k])*u for k in range(2))
                bb=tuple(a[k]+(b[k]-a[k])*v for k in range(2))
                na=normals[i] if j==0 else directions[i]
                nb=normals[i+1] if j+1==count else directions[i]
                coords=[round(p*1e7) for p in (*aa,*bb)]
                segments.append(struct.pack('<4i4f3fIB',*coords,*na,*nb,width,height,(distance+d*u)%240.,e['id']&0xffffffff,st))
                for p in (aa,bb):
                    bounds[0]=min(bounds[0],p[0]); bounds[1]=min(bounds[1],p[1]);bounds[2]=max(bounds[2],p[0]);bounds[3]=max(bounds[3],p[1])
            distance+=d;stats['wall_km']+=d/1000;stats['styles_km'][st]+=d/1000
    raw=b'GWL1'+struct.pack('<II',len(segments),len(towers))+b''.join(segments)+b''.join(towers)
    packed=zlib.compress(raw,9)
    manifest={
        'source':'https://www.openstreetmap.org/relation/318110',
        'query':'[out:json][timeout:180];rel(318110);(._;>>;)->.all;(way.all;node.all[historic];node.all[man_made];node.all[barrier];rel.all;);out geom;',
        'timestamp_osm_base':data['osm3s']['timestamp_osm_base'],
        'copyright':'© OpenStreetMap contributors; database licensed under ODbL 1.0',
        'source_sha256':hashlib.sha256(source).hexdigest(),
        'tower_source_sha256':hashlib.sha256(gzip.decompress(TOWERS.read_bytes())).hexdigest(),
        'tower_timestamp_osm_base':supplement['osm3s']['timestamp_osm_base'],
        'tower_query':'[out:json][timeout:180];rel(318110);way(r)->.walls;(node(around.walls:35)["tower:type"="defensive"];way(around.walls:35)["tower:type"="defensive"];);out geom;',
        'pack_sha256':hashlib.sha256(packed).hexdigest(),
        'segments':len(segments),'towers':len(towers),'bounds_south_west_north_east':bounds,
        **{k:([round(x,3) for x in v] if isinstance(v,list) else round(v,3) if isinstance(v,float) else v) for k,v in stats.items()},
        'style_order':['masonry','earth','ruined masonry','ruined earth'],
        'coverage':'All mapped wall ways and tower members of the recursively expanded relation, supplemented by mapped defensive towers within 35 m of its wall ways. Disconnected routes remain disconnected. This is not an exhaustive reconstruction of all historical Great Wall remains.',
        'inference':'Untagged dimensions use conservative metre-scale defaults. Untagged material west of 105°E and named Han/Qin/Jin walls uses earth; those ancient named walls use ruined profiles. Narrow polygon wall footprints are reduced to paired-boundary centre lines. Individual tower point dimensions and all architectural detail are approximate. Ditches are omitted, not replaced by walls.'
    }
    return {OUT/'network.gwl':packed,OUT/'manifest.json':(json.dumps(manifest,indent=2,ensure_ascii=False)+'\n').encode()}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--import-source',type=Path)
    parser.add_argument('--import-towers',type=Path)
    args=parser.parse_args()
    if args.import_source:
        if args.check: parser.error('--check cannot import a source')
        SOURCE.write_bytes(gzip.compress(args.import_source.read_bytes(),mtime=0))
    if args.import_towers:
        if args.check: parser.error('--check cannot import towers')
        TOWERS.write_bytes(gzip.compress(args.import_towers.read_bytes(),mtime=0))
    outputs=generate(gzip.decompress(SOURCE.read_bytes()))
    for path,data in outputs.items():
        if args.check:
            if path.read_bytes()!=data: raise SystemExit(f'Generated asset differs: {path}')
        else: path.write_bytes(data)
    print(('Verified' if args.check else 'Built')+' nationwide Great Wall source pack')
    print(outputs[OUT/'manifest.json'].decode())

if __name__=='__main__': main()
