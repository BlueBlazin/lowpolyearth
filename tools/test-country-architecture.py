#!/usr/bin/env python3
"""Negative contracts for the compiled country material catalog."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location('country_build', Path(__file__).with_name('build-country-architecture.py'))
COMPILER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COMPILER)


class CountryCatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.entries = sorted([e for p in sorted((COMPILER.ASSETS/'countries').glob('*.json'))
                              for e in json.loads(p.read_text())], key=lambda e: e['country'])
        cls.countries, _ = COMPILER.atlas_countries()

    def test_generated_bytes_match_complete_inspectable_sources(self):
        COMPILER.build(check=True)

    def test_missing_country_or_unsubstantiated_duplicate_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'country coverage mismatch'):
            COMPILER.validate(self.entries[:-1], self.countries)
        entries = copy.deepcopy(self.entries)
        entries[1].update({k: copy.deepcopy(entries[0][k]) for k in
                           ('urban', 'town', 'rural', 'walls', 'roofs', 'flat_roofs', 'trim', 'joinery')})
        entries[1]['shared_with'] = []
        entries[1]['sharing_reason'] = ''
        with self.assertRaisesRegex(ValueError, 'requires explicit justified sharing'):
            COMPILER.validate(entries, self.countries)

    def test_invalid_data_and_false_review_claims_are_rejected(self):
        for mutate, message in [
            (lambda e: e.update(evidence=[]), 'missing evidence'),
            (lambda e: e['roofs'][0].update(color=[255, -1, 80]), 'invalid RGB'),
            (lambda e: e['urban'][0].update(weight=0), 'positive relative weight'),
            (lambda e: e.update(review_status='visually-reviewed', visual_references=[]), 'visual review needs references'),
        ]:
            entries = copy.deepcopy(self.entries)
            mutate(entries[0])
            with self.assertRaisesRegex(ValueError, message):
                COMPILER.validate(entries, self.countries)

    def test_single_and_many_alternatives_are_allowed_without_padding(self):
        for count in (1, 3, 7):
            entries = copy.deepcopy(self.entries)
            entries[0]['walls']['timber'] = [dict(color=[100+i, 100, 100], weight=i+1) for i in range(count)]
            COMPILER.validate(entries, self.countries)

    def test_archetype_bindings_do_not_expand_into_unrelated_roof_choices(self):
        entry = copy.deepcopy(self.entries[0])
        entry['roofs'] = [dict(material='TILE_ROOF', color=[180, 70, 40], weight=1)]
        choice = dict(form='StonePitch', weight=1,
                      walls=[dict(material='STONE_WALL', color=[130, 135, 132], weight=1)],
                      roofs=[dict(material='SLATE_ROOF', color=[75, 80, 83], weight=1)],
                      roof_shapes=[dict(shape='Gabled', weight=1)])
        bound = COMPILER.archetype_data(entry, choice, 'rural')
        self.assertEqual(bound['walls'], choice['walls'])
        self.assertEqual(bound['roofs'], choice['roofs'])
        self.assertEqual(bound['roof_shapes'], choice['roof_shapes'])
        # Editing the country's default cannot contaminate the bound slate house.
        entry['roofs'][0]['material'] = 'METAL_ROOF'
        self.assertEqual(COMPILER.archetype_data(entry, choice, 'rural'), bound)
        choice['roofs'] = [dict(material='THATCH_ROOF', color=[150, 120, 75], weight=1)]
        self.assertEqual(COMPILER.archetype_data(entry, choice, 'town')['roofs'], choice['roofs'])

    def test_named_finishes_compile_without_changing_the_base_material_abi(self):
        entries = copy.deepcopy(self.entries)
        for role, name, material, variant in [
                ('roofs', 'CEMENT_SHEET_ROOF', 'ROOF', 1),
                ('roofs', 'TURF_ROOF', 'ROOF', 2),
                ('roofs', 'BITUMEN_ROOF', 'ROOF', 3),
                ('walls', 'AZULEJO_WALL', 'NAMAKO_WALL', 1),
                ('walls', 'AZULEJO_PATTERN_WALL', 'NAMAKO_WALL', 2)]:
            finish = dict(material=name, color=[103, 117, 89], weight=1)
            entries[0]['urban'][0][role] = [finish]
            COMPILER.validate(entries, self.countries)
            output = COMPILER.compile_rust(entries).decode()
            self.assertIn('material: material::'+material+', color: [103, 117, 89], variant: '+str(variant), output)
            finish['variant'] = 1
            with self.assertRaisesRegex(ValueError, 'already owns its packed variant'):
                COMPILER.validate(entries, self.countries)
            finish.pop('variant')

    def test_finish_roles_reject_fields_that_would_be_ignored_or_misrendered(self):
        for role, finish in [('walls', dict(material='TILE_ROOF', color=[100,100,100], weight=1)),
                             ('trim', dict(material='BRICK_WALL', color=[100,100,100], weight=1)),
                             ('flat_roofs', dict(variant=1, color=[100,100,100], weight=1))]:
            entries = copy.deepcopy(self.entries)
            entries[0]['urban'][0][role] = [finish]
            with self.assertRaises(ValueError):
                COMPILER.validate(entries, self.countries)

    def test_facade_contract_requires_explicit_compatible_use_and_scale(self):
        entries = copy.deepcopy(self.entries)
        choice = entries[0]['urban'][0]
        choice.update(form='RenderSlab', facade='StoneOffice', applicability={
            'uses': ['Commercial', 'Unknown'], 'min_height_m': 36, 'max_height_m': 220,
            'min_area_m2': 400, 'max_area_m2': 10000})
        COMPILER.validate(entries, self.countries)
        output = COMPILER.compile_rust(entries).decode()
        self.assertIn('facade: Some(crate::highrise::System::StoneOffice)', output)
        self.assertIn('uses: 5, ordinary_stock: false', output)
        for mutate, error in [
            (lambda c: c.pop('applicability'), 'explicit applicability'),
            (lambda c: c['applicability'].update(uses=['Residential']), 'observed residential'),
            (lambda c: c['applicability'].update(max_height_m=30), 'invalid applicability scale'),
            (lambda c: c.update(facade='InventedGlass'), 'unknown facade'),
            (lambda c: c.update(form='TimberPitch'), 'compatible mineral envelope'),
        ]:
            bad = copy.deepcopy(entries)
            mutate(bad[0]['urban'][0])
            with self.assertRaisesRegex(ValueError, error):
                COMPILER.validate(bad, self.countries)



if __name__ == '__main__':
    unittest.main()
