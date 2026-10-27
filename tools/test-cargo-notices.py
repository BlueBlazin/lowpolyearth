#!/usr/bin/env python3
"""Regression checks for the packaging notice gate."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('cargo_notices', Path(__file__).with_name('cargo-notices.py'))
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class NoticeGateTests(unittest.TestCase):
    def setUp(self):
        self.snapshot = json.loads(notices.SNAPSHOT.read_text())
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        path = Path(self.temp.name)
        self.file = path / 'snapshot.json'
        output = path / 'notices.md'
        output.write_text(notices.render(self.snapshot))
        for name, value in [('SNAPSHOT', self.file), ('OUTPUT', output)]:
            patcher = patch.object(notices, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch.object(notices, 'packages', return_value=copy.deepcopy(self.snapshot['packages']))
        patcher.start()
        self.addCleanup(patcher.stop)

    def check_snapshot(self):
        self.file.write_text(json.dumps(self.snapshot))
        notices.check()

    def test_current_snapshot_passes(self):
        self.check_snapshot()

    def test_changed_lockfile_is_rejected(self):
        self.snapshot['inputs']['Cargo.lock'] = 'stale'
        with self.assertRaisesRegex(ValueError, 'inputs changed'):
            self.check_snapshot()

    def test_missing_dependency_is_rejected(self):
        self.snapshot['packages'].pop()
        with self.assertRaisesRegex(ValueError, 'coverage is stale'):
            self.check_snapshot()

    def test_missing_notice_is_rejected(self):
        self.snapshot['packages'][0]['notices'] = []
        with self.assertRaisesRegex(ValueError, 'missing notice'):
            self.check_snapshot()

    def test_altered_license_is_rejected(self):
        key = next(iter(self.snapshot['texts']))
        self.snapshot['texts'][key] += 'changed'
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.check_snapshot()

    def test_altered_rendering_is_rejected(self):
        notices.OUTPUT.write_text('incomplete notices')
        with self.assertRaisesRegex(ValueError, 'do not match'):
            self.check_snapshot()


if __name__ == '__main__':
    unittest.main()
