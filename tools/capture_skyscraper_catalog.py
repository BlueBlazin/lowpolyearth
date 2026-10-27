#!/usr/bin/env python3
"""Capture additional skyscrapers using the production landmark capture path."""
from pathlib import Path
import sys
import capture_heritage_catalog as capture

capture.CATALOG=(Path(__file__).resolve().parents[1]/
    'crates/vearth-gen/assets/landmarks/skyscraper-surfaces/catalog.json')

if __name__=='__main__':
    if not any(a=='--output' or a.startswith('--output=') for a in sys.argv):
        sys.argv.extend(['--output',str(capture.ROOT/'output/skyscrapers/world')])
    capture.main()
