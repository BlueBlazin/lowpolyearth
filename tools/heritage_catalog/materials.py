"""Physical material ids shared with vearth_gen::material and the world atlas."""
import re
from pathlib import Path

WALL = 11
ROOF = 12
WINDOW = 15
STEEL = 16
MARBLE = 17
BRICK_WALL = 81
STONE_WALL = 82
PLASTER_WALL = 83
TIMBER_WALL = 84
TILE_ROOF = 85
SLATE_ROOF = 86
PANEL_WALL = 87
METAL_ROOF = 88
SIDING_WALL = 89
SHINGLE_ROOF = 90
FLAT_ROOF = 91
THATCH_ROOF = 92
NAMAKO_WALL = 93
SCULPTURE_METAL = 95
WEATHERED_LIMESTONE = 119
VEINED_MARBLE = 120
EARTHEN_PLASTER = 121


def validate_runtime_ids():
    source=Path(__file__).resolve().parents[2]/'crates/vearth-gen/src/lib.rs'
    constants={name:int(value)for name,value in re.findall(r'pub const (\w+): u8 = (\d+);',source.read_text())}
    for name,value in globals().items():
        if name.isupper() and isinstance(value,int):
            if constants.get(name)!=value:
                raise ValueError(f'{name}: heritage material id disagrees with runtime')


def retag(mesh, mapping):
    """Apply an explicit site's material specification without changing tint."""
    mesh.triangles=[(p,c,mapping.get(material,material)) for p,c,material in mesh.triangles]
    return mesh
