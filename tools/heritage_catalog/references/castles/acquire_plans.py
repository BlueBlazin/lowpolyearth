"""Fetch compact public OSM map extracts for named castle footprint auditing."""
import gzip
import hashlib
import json
import math
from pathlib import Path
import urllib.request
import xml.etree.ElementTree as ET

HERE=Path(__file__).resolve().parent
for source in sorted(HERE.glob('*-source.json')):
    d=json.loads(source.read_text());name=source.stem.removesuffix('-source')
    lat,lon,e=d['lat'],d['lon'],d['query_half_extent_m']
    north=e/111195;east=e/(111195*math.cos(math.radians(lat)))
    url=f'https://api.openstreetmap.org/api/0.6/map?bbox={lon-east},{lat-north},{lon+east},{lat+north}'
    archive=HERE/f'{name}-osm.xml.gz'
    if archive.exists():raw=gzip.decompress(archive.read_bytes())
    else:
        with urllib.request.urlopen(urllib.request.Request(url,headers={'User-Agent':'Vearth metric landmark research'}),timeout=90) as response:raw=response.read()
        archive.write_bytes(gzip.compress(raw,mtime=0))
    root=ET.fromstring(raw)
    nodes={n.attrib['id']:(float(n.attrib['lat']),float(n.attrib['lon'])) for n in root.findall('node')}
    members={}
    for r in root.findall('relation'):
        tags={x.attrib['k']:x.attrib['v'] for x in r.findall('tag')}
        if tags.get('building') or tags.get('historic'):
            for member in r.findall('member'):
                if member.attrib['type']=='way':members[member.attrib['ref']]={**tags,'relation_id':r.attrib['id'],'relation_role':member.attrib.get('role','')}
    ways=[]
    for w in root.findall('way'):
        tags={**members.get(w.attrib['id'],{}),**{t.attrib['k']:t.attrib['v'] for t in w.findall('tag')}}
        if not (tags.get('building') or tags.get('building:part') or tags.get('barrier') in ['wall','city_wall','retaining_wall'] or tags.get('historic') in ['castle','fort']):continue
        p=[nodes[n.attrib['ref']] for n in w.findall('nd')]
        pts=[[(lo-lon)*111195*math.cos(math.radians(lat)),(la-lat)*111195] for la,lo in p]
        ways.append({'id':int(w.attrib['id']),'tags':tags,'xz':[[round(x,3),round(z,3)] for x,z in pts]})
    out={'source':url,'retrieved':'2026-09-13','anchor':[lat,lon],'attribution':'© OpenStreetMap contributors, ODbL 1.0','coordinate_system':'local east/north metres','source_xml_sha256':hashlib.sha256(raw).hexdigest(),'ways':ways}
    (HERE/f'{name}-plan.json').write_text(json.dumps(out,separators=(',',':'))+'\n')
    print(name,len(ways),flush=True)
