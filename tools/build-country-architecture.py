#!/usr/bin/env python3
"""Validate every bundled country's material specification and compile static Rust.

No network or runtime JSON parsing. --check verifies exact atlas coverage,
source records, coordinated finish roles, duplicate justifications and hashes.
"""
import argparse
import copy
import hashlib
import json
import math
import re
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'crates/vearth-gen/assets/architecture'
ATLAS = ROOT / 'crates/vearth-data/assets/admin-regions-v1.bin.z'
OUTPUT = ASSETS / 'countries-generated.rs'
MANIFEST = ASSETS / 'countries-manifest.json'
FORMS = {'BrickPitch', 'RenderPitch', 'StonePitch', 'TimberPitch', 'RenderSlab',
         'BrickSlab', 'MasonryMetal', 'TimberMetal', 'EarthFlat', 'EarthPitch', 'CourtyardRender',
         'TropicalRenderPitch', 'StoneSlab'}
ROOF_ENCODINGS = {'CEMENT_SHEET_ROOF': ('ROOF', 1), 'TURF_ROOF': ('ROOF', 2),
                  'BITUMEN_ROOF': ('ROOF', 3)}
WALL_ENCODINGS = {'AZULEJO_WALL': ('NAMAKO_WALL', 1),
                  'AZULEJO_PATTERN_WALL': ('NAMAKO_WALL', 2)}
FINISH_ENCODINGS = ROOF_ENCODINGS | WALL_ENCODINGS
ROOFS = {'TILE_ROOF', 'SLATE_ROOF', 'SHINGLE_ROOF', 'METAL_ROOF', 'THATCH_ROOF'} | ROOF_ENCODINGS.keys()
FACADE_SYSTEMS = {'ClearOffice', 'SilverOffice', 'BronzeOffice', 'StoneOffice', 'HorizontalOffice', 'Residential', 'RecessedResidential', 'ConcreteFrame'}
USE_BITS = {'Unknown': 1, 'Residential': 2, 'Commercial': 4, 'Civic': 8, 'Industrial': 16}
ROLES = ('render', 'brick', 'stone', 'timber', 'earth')
WALL_MATERIALS = {'PLASTER_WALL', 'BRICK_WALL', 'STONE_WALL', 'TIMBER_WALL', 'SIDING_WALL', 'PANEL_WALL', 'EARTHEN_PLASTER', 'WEATHERED_LIMESTONE', 'VEINED_MARBLE', 'NAMAKO_WALL'} | WALL_ENCODINGS.keys()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def atlas_countries():
    data = zlib.decompress(ATLAS.read_bytes())
    require(data[:4] == b'VAR1', 'unsupported administrative atlas')
    version, scale, count = struct.unpack_from('<III', data, 4)
    require(version == 1 and scale > 0, 'unsupported administrative atlas version')
    at = 16
    names = set()

    def read(fmt):
        nonlocal at
        value = struct.unpack_from(fmt, data, at)[0]
        at += struct.calcsize(fmt)
        return value

    def string():
        nonlocal at
        size = read('<H')
        value = data[at:at + size].decode('utf-8')
        at += size
        return value

    for _ in range(count):
        at += 24  # stable id and bounding box
        string()
        names.add(string())
        for _ in range(read('<H')):
            for _ in range(read('<H')):
                size = read('<I')
                at += size * 8
    require(at == len(data), 'trailing/truncated administrative atlas')
    return names, count


def atlas_subdivisions():
    data = zlib.decompress(ATLAS.read_bytes())
    count = struct.unpack_from('<I', data, 12)[0]
    at = 16; names = set()
    def read(fmt):
        nonlocal at
        value = struct.unpack_from(fmt, data, at)[0]; at += struct.calcsize(fmt); return value
    def string():
        nonlocal at
        size = read('<H'); value = data[at:at+size].decode(); at += size; return value
    for _ in range(count):
        at += 24; name = string(); country = string(); names.add((country, name))
        for _ in range(read('<H')):
            for _ in range(read('<H')):
                size = read('<I')
                at += size * 8
    return names


def rgb(value, name):
    require(isinstance(value, list) and len(value) == 3
            and all(type(v) is int and 0 <= v <= 255 for v in value), f'{name}: invalid RGB')


def weighted(values, name, key):
    require(isinstance(values, list), f'{name}: alternatives must be a list')
    for item in values:
        require(type(item.get('weight')) is int and 0 < item['weight'] <= 65535,
                f'{name}: positive relative weight required')
        if key == 'color':
            rgb(item.get('color'), name)
            require(item.get('material') is None or item['material'] in WALL_MATERIALS | ROOFS, f'{name}: unknown physical material')
            require(type(item.get('variant', 0)) is int and 0 <= item.get('variant', 0) <= 3,
                    f'{name}: invalid surface variant')
            require(item.get('material') not in FINISH_ENCODINGS or not item.get('variant', 0),
                    f'{name}: named finish already owns its packed variant')
        elif key == 'form':
            require(item.get('form') in FORMS, f'{name}: invalid form')
            if 'applicability' in item:
                applicability = item['applicability']
                require(isinstance(applicability, dict) and not (set(applicability) - {'uses', 'min_height_m', 'max_height_m', 'min_area_m2', 'max_area_m2'}),
                        f'{name}: invalid applicability fields')
                uses = applicability.get('uses')
                require(isinstance(uses, list) and bool(uses) and len(set(uses)) == len(uses)
                        and all(use in USE_BITS for use in uses), f'{name}: explicit observed uses required')
                require('Industrial' not in uses, f'{name}: industrial source uses dedicated envelope')
                for axis in ('height_m', 'area_m2'):
                    low, high = applicability.get('min_' + axis, 0), applicability.get('max_' + axis, 1e30)
                    require(all(type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1e30 for v in (low, high))
                            and low < high, f'{name}: invalid applicability scale')
            if 'facade' in item:
                require(item['facade'] in FACADE_SYSTEMS, f'{name}: unknown facade system')
                require('applicability' in item, f'{name}: authored facade needs explicit applicability')
                if item['facade'].endswith('Office'):
                    require('Residential' not in item['applicability']['uses'],
                            f'{name}: office facade cannot admit observed residential use')
                require(item['form'] not in {'TimberPitch', 'TimberMetal', 'EarthFlat', 'EarthPitch'},
                        f'{name}: facade system requires a compatible mineral envelope')

            for role in ('walls','roofs','flat_roofs','trim','joinery'):
                if role in item:
                    require(role == 'roofs' or bool(item[role]), f'{name}: empty archetype {role}')
                    weighted(item[role], f'{name}/{role}', 'color')
            for roof in item.get('roofs',[]):
                require(roof.get('material') in ROOFS, f'{name}: invalid archetype roof material')
            for wall in item.get('walls', []):
                require(wall.get('material') is None or wall['material'] in WALL_MATERIALS,
                        f'{name}: roof material cannot be used as an archetype wall')
            for role in ('flat_roofs', 'trim', 'joinery'):
                for finish in item.get(role, []):
                    require('material' not in finish and not finish.get('variant', 0),
                            f'{name}/{role}: role accepts pigment only')
            require(item.get('style','contemporary') in STYLE_KEYS, f'{name}: unknown style key')
            require(type(item.get('expression',0)) is int and 0 <= item.get('expression',0) <= 255, f'{name}: invalid expression')
            if 'roof_shapes' in item:
                require(bool(item['roof_shapes']), f'{name}: empty roof shape prior')
                for shape in item['roof_shapes']:
                    require(shape.get('shape') in SHAPES and type(shape.get('weight')) is int and 0 < shape['weight'] <= 65535, f'{name}: invalid roof shape prior')


def validate(entries, countries):
    names = [entry['country'] for entry in entries]
    require(len(set(names)) == len(names), 'duplicate country specification')
    require(set(names) == countries,
            f'country coverage mismatch: missing={sorted(countries-set(names))}, extra={sorted(set(names)-countries)}')
    signatures = {}
    for entry in entries:
        name = entry['country']
        for field in ('label', 'character', 'scope'):
            require(isinstance(entry.get(field), str) and bool(entry[field].strip()), f'{name}: missing {field}')
        require(bool(entry.get('evidence')), f'{name}: missing evidence')
        for source in entry['evidence']:
            require(source.get('url', '').startswith('https://'), f'{name}: evidence URL must be HTTPS')
            for field in ('title', 'scope', 'note'):
                require(isinstance(source.get(field), str) and bool(source[field].strip()),
                        f'{name}: missing source {field}')
        for context in ('urban', 'town', 'rural'):
            require(bool(entry[context]), f'{name}: empty {context} construction mix')
            weighted(entry[context], f'{name}/{context}', 'form')
        require(set(entry['walls']) <= set(ROLES), f'{name}: unknown material role keys')
        for role in ROLES:
            weighted(entry['walls'].get(role, []), f'{name}/{role}', 'color')
            for wall in entry['walls'].get(role, []):
                require(wall.get('material') is None or wall['material'] in WALL_MATERIALS,
                        f'{name}/{role}: roof material cannot be used as a wall')
        require(bool(entry['roofs']), f'{name}: empty pitched roof mix')
        weighted(entry['roofs'], f'{name}/roofs', 'color')
        for roof in entry['roofs']:
            require(roof['material'] in ROOFS, f'{name}: invalid pitched roof material')
        for role in ('flat_roofs', 'trim', 'joinery'):
            require(bool(entry[role]), f'{name}: empty {role}')
            weighted(entry[role], f'{name}/{role}', 'color')
            for finish in entry[role]:
                require('material' not in finish and not finish.get('variant', 0),
                        f'{name}/{role}: role accepts pigment only')
        require(entry.get('review_status') in ('pending', 'stock-reviewed', 'visually-reviewed'),
                f'{name}: missing review status')
        for role in ('construction', 'finishes'):
            require(bool(entry.get('weight_basis', {}).get(role, '').strip()),
                    f'{name}: weights need scoped evidence or an explicit pending status')
        for reference in entry.get('visual_references', []):
            require(reference.get('url', '').startswith('https://'), f'{name}: invalid visual reference')
            require(bool(reference.get('observed', '').strip()) and bool(reference.get('scope', '').strip()),
                    f'{name}: visual reference needs inspected observations and scope')
        if entry['review_status'] == 'visually-reviewed':
            require(bool(entry.get('visual_references')), f'{name}: visual review needs references')
        if entry['review_status'] != 'pending':
            form_roles = {'BrickPitch':'brick', 'BrickSlab':'brick', 'StonePitch':'stone', 'StoneSlab':'stone',
                          'TimberPitch':'timber', 'TimberMetal':'timber', 'EarthFlat':'earth', 'EarthPitch':'earth'}
            for context in ('urban', 'town', 'rural'):
                for choice in entry[context]:
                    role = form_roles.get(choice['form'], 'render')
                    require(bool(choice.get('walls',entry['walls'].get(role))), f'{name}: reviewed {choice["form"]} needs {role} finishes')
        shared = entry.get('shared_with', [])
        require(isinstance(shared, list) and set(shared) <= countries - {name}, f'{name}: invalid shared_with')
        if shared:
            require(bool(entry.get('sharing_reason', '').strip()), f'{name}: sharing needs a physical-material justification')
        signature = json.dumps({k: entry[k] for k in ('urban', 'town', 'rural', 'walls', 'roofs', 'flat_roofs', 'trim', 'joinery')}, sort_keys=True)
        for other in signatures.get(signature, []):
            require(other in shared and bool(entry.get('sharing_reason', '').strip()),
                    f'{name}: identical specification to {other} requires explicit justified sharing')
        signatures.setdefault(signature, []).append(name)


def rust_string(value):
    return json.dumps(value, ensure_ascii=False)


def rust_array(value):
    return json.dumps(value, separators=(', ', ': '))


def region_profile(country, region):
    result = copy.deepcopy(country)
    result.pop('regions', None)
    for key, value in region.items():
        if key == 'walls':
            result['walls'].update(value)
        elif key != 'names':
            result[key] = value
    result['shared_with'] = []
    result['sharing_reason'] = ''
    return result


STYLE_KEYS = dict((key, name) for name, key in re.findall(r'^\s*(\w+) => \("([^"]+)"',
    (ROOT/'crates/vearth-gen/src/architecture_styles.rs').read_text(), re.M))
FORM_ROLES = {'BrickPitch':'brick', 'BrickSlab':'brick', 'StonePitch':'stone', 'StoneSlab':'stone',
              'TimberPitch':'timber', 'TimberMetal':'timber', 'EarthFlat':'earth', 'EarthPitch':'earth'}
ROLE_MATERIALS = dict(render='PLASTER_WALL', brick='BRICK_WALL', stone='STONE_WALL', timber='SIDING_WALL', earth='EARTHEN_PLASTER')
SHAPES = {'Flat', 'Gabled', 'Hipped', 'Pyramidal', 'Mansard', 'Gambrel', 'Skillion', 'Dome', 'Onion', 'Sawtooth'}
FLAT_FORMS = {'RenderSlab', 'BrickSlab', 'StoneSlab', 'EarthFlat', 'CourtyardRender'}


def archetype_data(entry, choice, settlement):
    form = choice['form']; role = FORM_ROLES.get(form, 'render')
    walls = choice.get('walls', entry['walls'].get(role, []))
    if not walls:
        # Pending evidence never borrows an unrelated country's palette.
        color = [177, 137, 92] if role == 'earth' else [195, 195, 190]
        walls = [dict(color=color, material=ROLE_MATERIALS[role], weight=1)]
    roof = choice.get('roofs', entry['roofs'])
    if 'roofs' not in choice:
        if form in ('MasonryMetal', 'TimberMetal'):
            roof = [r for r in roof if r['material'] == 'METAL_ROOF']
        if settlement != 'rural' or role not in ('timber', 'earth'):
            roof = [r for r in roof if r['material'] != 'THATCH_ROOF']
    shapes = choice.get('roof_shapes', [dict(shape='Flat', weight=1)] if form in FLAT_FORMS
                        else [dict(shape='Gabled', weight=1), dict(shape='Hipped', weight=1)])
    return dict(name=choice.get('label',form), form=form, style=choice.get('style','contemporary'),
                expression=choice.get('expression',0), applicability=choice.get('applicability'), facade=choice.get('facade'), walls=walls, wall_material=ROLE_MATERIALS[role],
                roofs=roof, roof_shapes=shapes, **{key:choice.get(key,entry[key]) for key in ('flat_roofs','trim','joinery')})


def compile_rust(entries, places=()):
    lines = ['// Generated by tools/build-country-architecture.py; edit countries/*.json.',
             '#[rustfmt::skip]', 'pub static COUNTRY_ARCHITECTURE: &[CountryArchitecture] = &[']
    def finish(item, default_material):
        material, variant = FINISH_ENCODINGS.get(item.get('material'),
            (item.get('material', default_material), item.get('variant', 0)))
        return 'Finish { material: material::' + material + ', color: ' + rust_array(item['color']) + ', variant: ' + str(variant) + ' }'
    def choices(values, encode):
        return '&[' + ', '.join('Weighted::new(' + encode(v) + ', ' + str(v['weight']) + ')' for v in values) + ']'
    pool = []; interned = {}
    def shared_slice(values, encode, rust_type):
        literal = choices(values,encode); key = (rust_type,literal)
        if key not in interned:
            symbol = 'FINISH_CHOICES_' + str(len(interned))
            interned[key] = symbol
            pool.append('#[rustfmt::skip]\nstatic '+symbol+': &[Weighted<'+rust_type+'>] = '+literal+';')
        return interned[key]
    archetypes = {}
    def archetype(a):
        fields = ['name: '+rust_string(a['name']), 'form: ResidentialForm::'+a['form'],
                  'style: ArchitectureStyle::'+STYLE_KEYS[a['style']], 'expression: '+str(a['expression'])]
        applicability = a.get('applicability')
        if applicability is None:
            fields.append('applicability: Applicability::ORDINARY_STOCK')
        else:
            values = ['uses: '+str(sum(USE_BITS[u] for u in applicability['uses'])), 'ordinary_stock: false']
            for field in ('min_height_m', 'max_height_m', 'min_area_m2', 'max_area_m2'):
                values.append(field+': '+repr(float(applicability.get(field, 0 if field.startswith('min_') else 1e30))))
            fields.append('applicability: Applicability { '+', '.join(values)+' }')
        fields.append('facade: '+('Some(crate::highrise::System::'+a['facade']+')' if a.get('facade') else 'None'))
        fields.append('roof_shapes: '+shared_slice(a['roof_shapes'], lambda v:'RoofShape::'+v['shape'], 'RoofShape'))
        for role,material in [('walls',a['wall_material']),('roofs','ROOF'),('flat_roofs','FLAT_ROOF')]:
            fields.append(role+': '+shared_slice(a[role],lambda v:finish(v,material), 'Finish'))
        for role in ('trim','joinery'):
            fields.append(role+': '+shared_slice(a[role],lambda v:rust_array(v['color']), '[u8; 3]'))
        literal = 'BuildingArchetype { '+', '.join(fields)+' }'
        if literal not in archetypes:
            symbol = 'ARCHETYPE_' + str(len(archetypes)); archetypes[literal] = symbol
            pool.append('#[rustfmt::skip]\nstatic '+symbol+': BuildingArchetype = '+literal+';')
        return '&'+archetypes[literal]
    def fields(entry, prefix):
        lines.extend([prefix + f'country: {rust_string(entry["country"])},', prefix + f'label: {rust_string(entry["label"])},'])
        for role in ('urban', 'town', 'rural'):
            lines.append(prefix + f'{role}: '+choices(entry[role],lambda v:archetype(archetype_data(entry,v,role)))+',')
        infill = dict(form='RenderSlab', label='source-scale masonry infill',
                      walls=[w for w in entry['walls'].get('render',[]) if w.get('material','PLASTER_WALL') not in ('TIMBER_WALL','SIDING_WALL','EARTHEN_PLASTER')])
        lines.append(prefix+'infill: '+archetype(archetype_data(entry,infill,'urban'))+',')
    for entry in entries:
        lines.append('    CountryArchitecture {'); fields(entry,'        '); lines.append('    },')
    lines.extend(['];', '#[rustfmt::skip]', 'pub static REGIONAL_ARCHITECTURE: &[RegionalArchitecture] = &['])
    for entry in entries:
        for region in entry.get('regions', []):
            names = '&[' + ', '.join(rust_string(n) for n in region['names']) + ']'
            lines.append('    RegionalArchitecture { country: '+rust_string(entry['country'])+', names: '+names+', profile: CountryArchitecture {')
            fields(region_profile(entry,region),'        '); lines.append('    } },')
    lines.extend(['];', '#[rustfmt::skip]', 'pub static PLACE_ARCHITECTURE: &[PlaceArchitecture] = &['])
    for place in places:
        literal = choices(place['archetypes'], lambda choice: archetype(archetype_data(place, choice, 'rural')))
        lines.append('    PlaceArchitecture { source_id: '+rust_string(place['source_id'])+', style_key: '+rust_string(place['style_key'])+', archetypes: '+literal+' },')
    lines.extend(['];','']); return '\n'.join([lines[0], *pool, *lines[1:]]).encode()


def place_specs():
    path = ASSETS / 'place-finishes.json'
    places = json.loads(path.read_text())
    identities = {(e['source_id'], e['style_key']) for e in json.loads((ASSETS/'places.json').read_text())['places']}
    seen = set()
    for place in places:
        key = (place['source_id'], place['style_key'])
        require(key in identities and key not in seen, f'unknown/duplicate place finish {key}')
        seen.add(key)
        for field in ('label','scope','weight_basis','evidence','visual_references'):
            require(bool(place.get(field)), f'{key}: missing {field}')
        for field in ('construction', 'finishes'):
            require(isinstance(place['weight_basis'].get(field), str) and bool(place['weight_basis'][field].strip()),
                    f'{key}: missing scoped {field} weight basis')
        for source in place['evidence']:
            require(source.get('url', '').startswith('https://'), f'{key}: invalid evidence URL')
            for field in ('title', 'scope', 'note'):
                require(isinstance(source.get(field), str) and bool(source[field].strip()),
                        f'{key}: missing evidence {field}')
        for source in place['visual_references']:
            require(source.get('url', '').startswith('https://'), f'{key}: invalid visual reference URL')
            for field in ('scope', 'observed'):
                require(isinstance(source.get(field), str) and bool(source[field].strip()),
                        f'{key}: missing inspected {field}')
        require(place.get('review_status') in ('stock-reviewed','visually-reviewed'), f'{key}: reviewed place needs evidence status')
        require(bool(place.get('archetypes')), f'{key}: missing complete archetypes')
        weighted(place['archetypes'], str(key), 'form')
        for a in place['archetypes']:
            for field in ('label','style','walls','roofs','flat_roofs','trim','joinery','roof_shapes'):
                require(field in a, f'{key}: archetype missing {field}')
        # Place alternatives are fully bound, including the visible infill.
        # Empty defaults cannot silently supply a foreign geographic palette.
        place.update(walls={}, roofs=[], flat_roofs=[], trim=[], joinery=[])
    return sorted(places, key=lambda p:(p['source_id'],p['style_key'])), path


def build(check=False):
    countries, record_count = atlas_countries()
    shards = sorted((ASSETS / 'countries').glob('*.json'))
    entries = [entry for path in shards for entry in json.loads(path.read_text())]
    entries.sort(key=lambda entry: entry['country'])
    validate(entries, countries)
    # Exact atlas-name validation includes overrides, not just country coverage.
    atlas_names = atlas_subdivisions()
    used = set()
    for entry in entries:
        for region in entry.get('regions', []):
            require(bool(region.get('names')), 'region needs exact source names')
            for name in region['names']:
                key = (entry['country'], name)
                require(key in atlas_names and key not in used, f'stale/duplicate region {key}')
                used.add(key)
            for field in ('label', 'scope', 'evidence', 'weight_basis', 'review_status', 'visual_references'):
                require(field in region, f'{entry["country"]}: region missing {field}')
            merged = region_profile(entry, region)
            validate([merged], {entry['country']})
    places, places_path = place_specs()
    code = compile_rust(entries, places)
    manifest = {
        'schema_version': 3, 'country_territory_count': len(entries), 'admin_record_count': record_count, 'reviewed_subdivision_count': len(used),
        'evidence_url_count': len({s['url'] for e in entries for s in e['evidence']}),
        'atlas_sha256': digest(ATLAS.read_bytes()),
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'inputs': {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in shards},
        'generated_rust_sha256': digest(code),
        'style_keys_source_sha256': digest((ROOT/'crates/vearth-gen/src/architecture_styles.rs').read_bytes()),
        'place_finishes_sha256': digest(places_path.read_bytes()),
        'place_identities_sha256': digest((ASSETS/'places.json').read_bytes()),
        'reviewed_place_recipes': len(places),
        'compiled_archetypes': code.count(b'static ARCHETYPE_'),
        'review_status_counts': {status: sum(e['review_status'] == status for e in entries) for status in ('pending', 'stock-reviewed', 'visually-reviewed')},
        'interpretation': 'Variable-length relative weights. Per-country weight_basis distinguishes observed stock, visual interpretation, and unresolved review; compiling data is not visual acceptance.',
    }
    for path, data in [(OUTPUT, code), (MANIFEST, (json.dumps(manifest, indent=2, ensure_ascii=False)+'\n').encode())]:
        if check:
            require(path.exists() and path.read_bytes() == data, f'stale generated artifact: {path}')
        else:
            path.write_bytes(data)
    print(f'Country architecture: {len(entries)} explicit country/territory specifications, {record_count} atlas records, {manifest["evidence_url_count"]} source URLs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    build(parser.parse_args().check)
