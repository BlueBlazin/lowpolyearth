"""The production highrise palette and bounded facade panel convention."""
from pathlib import Path
import numpy as np

ROOT=Path(__file__).resolve().parents[2]
DEPENDENCIES=[ROOT/'crates/vearth-gen/src/highrise.rs',ROOT/'crates/vearth-gen/src/building_surface.rs']
# System order is the runtime's contiguous 103..110 facade material range.
GLASS=((91,129,145),(137,157,165),(129,113,91),(78,99,108),
       (85,114,126),(77,104,118),(77,104,118),(73,96,106))

def muted_frame(color):
    return tuple(round(v if v>=110 else 96+.45*v)for v in color)

def panel_tint(material,wall):
    index=(material-103)%8
    return tuple(round(.62*g+.38*w)for g,w in zip(GLASS[index],wall))

def panel_count(width,bay,lod):
    return min(12 if lod==0 else 8,max(1,round(width/max(4.,bay))))
