"""Recorded landmark source transforms and material palettes for indexed export.

Coordinates are model-space metres. Surveyed fits, owner-authored origins,
material identity and source cropping are shared by the offline exporters.
"""
from __future__ import annotations
import zipfile
from pathlib import Path
import numpy as np
import trimesh

LIBERTY_PALETTE = (
    (17, (194, 184, 168)),  # granite / marble pedestal
    (16, (91, 151, 131)),   # oxidized copper
    (16, (223, 178, 71)),   # gilded flame
)


TAJ_PALETTE = (
    (17, (239, 235, 225)),  # white Makrana marble
    (8, (210, 204, 192)),   # plinth / foundation
    (16, (216, 176, 76)),   # finials
)


SAGRADA_PALETTE = (
    (11, (194, 170, 136)),  # warm Montjuïc stone
    (17, (220, 202, 169)),  # light tower crowns
)


BIG_BEN_PALETTE = (
    (11, (194, 179, 150)),  # warm limestone
    (17, (219, 207, 181)),  # lit limestone / carved trim
    (11, (151, 139, 119)),  # deep architectural shadow
    (17, (229, 224, 205)),  # clock glass
    (16, (39, 47, 45)),     # clock surround and numerals
    (16, (211, 170, 65)),   # gilt hands and finial
    (12, (54, 70, 67)),     # oxidized metal roof
    (15, (48, 57, 61)),     # dark window recesses
    (11, (177, 157, 128)),  # weathered stone bands
    (8, (129, 124, 112)),   # foundation
)


CHRIST_PALETTE = (
    (8, (166, 165, 158)),   # pedestal
    (17, (205, 207, 200)),  # soapstone figure
)


BROOKLYN_PALETTE = (
    (11, (176, 161, 137)),  # granite tower stone
    (17, (205, 191, 165)),  # sunlit tower stone
    (11, (129, 119, 104)),  # stone shadow
    (16, (110, 91, 73)),    # suspension cables
    (16, (72, 76, 78)),     # steelwork
    (6, (53, 55, 57)),      # roadway
    (8, (174, 151, 116)),   # timber promenade
)


LONDON_EYE_PALETTE = (
    (16, (220, 224, 224)),  # white wheel steel
    (16, (161, 169, 174)),  # shaded wheel steel
    (15, (91, 126, 145)),   # capsule glass
    (16, (59, 69, 76)),     # hubs and joints
    (8, (176, 176, 171)),   # concrete base
    (16, (173, 44, 45)),    # restrained red trim
)


MERLION_PALETTE = (
    (17, (225, 222, 207)),  # pale stone mosaic
    (17, (204, 201, 187)),  # shaded stone mosaic
    (11, (177, 176, 166)),  # sculptural recesses
    (15, (73, 105, 116)),   # mouth / eye shadow
    (8, (176, 184, 181)),   # plinth
    (9, (91, 151, 169)),    # fountain water accent
)


SPACE_NEEDLE_PALETTE = (
    (16, (224, 224, 218)),  # painted steel
    (16, (176, 181, 181)),  # shaded steel
    (15, (55, 75, 91)),     # observation glazing
    (17, (218, 213, 201)),  # saucer cladding
    (16, (204, 111, 50)),   # galaxy-gold/orange roof accent
    (8, (151, 151, 146)),   # concrete footing
)


GIZA_PALETTE = (
    (10, (205, 183, 139)),  # weathered limestone
    (10, (224, 205, 168)),  # sunlit limestone
    (11, (171, 151, 116)),  # shadowed courses
    (17, (231, 219, 192)),  # surviving casing stone
    (10, (190, 168, 128)),  # darker base courses
    (10, (213, 191, 149)),  # secondary pyramid stone
)


COLOSSEUM_PALETTE = (
    (17, (210, 196, 169)),  # travertine
    (17, (229, 218, 196)),  # sunlit travertine
    (11, (157, 143, 123)),  # arcade shadow
    (11, (184, 161, 132)),  # weathered exposed core
    (10, (190, 169, 130)),  # arena / floor
    (12, (100, 115, 81)),   # sparse vegetation
)


BRANDENBURG_PALETTE = (
    (17, (211, 200, 176)),  # sandstone
    (17, (229, 220, 198)),  # sunlit sandstone
    (11, (165, 154, 134)),  # colonnade shadow
    (11, (192, 177, 150)),  # attic relief
    (16, (82, 124, 105)),   # patinated quadriga
    (8, (139, 135, 124)),   # paving / foundation
)


ARC_PALETTE = (
    (17, (220, 211, 192)),  # pale limestone
    (17, (236, 229, 214)),  # sunlit faces
    (11, (166, 156, 140)),  # vault shadow
    (11, (195, 181, 158)),  # sculpted relief
    (8, (147, 143, 133)),   # foundation
)


TOWER_BRIDGE_PALETTE = (
    (11, (198, 190, 175)),  # granite and Portland stone
    (16, (62, 111, 151)),   # bridge blue steel
    (16, (224, 226, 222)),  # white steel highlights
    (15, (61, 76, 88)),     # glass / deep recess
    (6, (58, 61, 64)),      # roadway
    (11, (166, 151, 130)),  # stone shadow
    (16, (171, 59, 49)),    # red mechanical accents
    (16, (185, 148, 57)),   # gilt details
    (12, (70, 79, 88)),     # slate roofs
)


PARLIAMENT_PALETTE = (
    # Runtime tints compensate for the source scene's display transform.
    (11, (166, 161, 145)),  # weathered Anston limestone
    (17, (194, 181, 151)),  # carved stone trim and pinnacles
    (11, (144, 139, 122)),  # plinths, terrace and weathered stone
    (12, (95, 107, 103)),   # dark grey-green cast-iron roofs
    (12, (125, 129, 119)),  # Westminster Hall lead-grey roof
    (15, (49, 57, 67)),     # glazing and open tracery
    (16, (174, 148, 78)),   # restrained gilt details
    (17, (214, 212, 198)),  # clock faces
    (16, (45, 59, 84)),     # restored Prussian-blue clock ironwork
    (16, (154, 43, 49)),    # Union flag red
    (16, (35, 46, 91)),     # Union flag blue
    (17, (211, 211, 214)),  # Union flag white
    (16, (72, 74, 78)),     # poles and railings
)


EMPIRE_STATE_PALETTE = (
    (11, (197, 188, 169)),  # Indiana limestone
    (17, (222, 214, 197)),  # sunlit limestone and carved setbacks
    (11, (151, 145, 133)),  # recessed masonry shadow
    (15, (50, 62, 72)),     # dark window bands
    (16, (155, 162, 165)),  # stainless crown and mast
    (16, (213, 218, 219)),  # bright metal highlights
    (8, (126, 124, 117)),   # foundation and entrance surround
    (16, (126, 38, 34)),    # aviation beacon
)


EIFFEL_PALETTE = (
    (16, (112, 79, 57)),    # Eiffel brown ironwork
    (16, (82, 58, 44)),     # shaded lattice
    (16, (139, 99, 68)),    # sunlit ironwork
    (15, (45, 54, 58)),     # observation glazing and deep lattice
    (16, (165, 158, 146)),  # antenna and mechanical steel
    (8, (143, 137, 125)),   # feet and platform paving
    (16, (188, 45, 37)),    # summit beacon
)


TOKYO_TOWER_PALETTE = (
    (16, (213, 67, 42)),    # international orange steel
    (16, (235, 232, 220)),  # white steel bands
    (16, (151, 48, 34)),    # shaded orange steel
    (15, (54, 68, 78)),     # observation glazing
    (16, (168, 172, 169)),  # mechanical steel and antenna
    (8, (131, 130, 124)),   # footing and lower service deck
    (16, (222, 184, 62)),   # aviation lights
)


WASHINGTON_PALETTE = (
    (17, (218, 214, 202)),  # upper Maryland marble
    (17, (198, 194, 181)),  # lower marble color break
    (11, (163, 160, 151)),  # shaded stone
    (17, (233, 230, 219)),  # pyramidion highlight
    (15, (50, 57, 61)),     # entrance and observation apertures
    (16, (186, 190, 188)),  # aluminium apex
    (8, (143, 140, 132)),   # base terrace
)


AUTHORED_PALETTES = {
    "opera": (
        ("Tarana pink granite", 8, (179, 161, 139)),
        ("Granite paving", 8, (193, 175, 152)),
        ("Ceramic - warm white", 17, (233, 231, 220)),
        ("Concrete - exposed ribs", 8, (197, 191, 175)),
        ("Ceramic - cream", 17, (223, 221, 204)),
        ("Bronze glazing - opaque voxel material", 15, (61, 73, 72)),
        ("Bronze mullions", 16, (85, 77, 62)),
        ("Granite joints", 8, (155, 139, 120)),
        ("Steel rails", 16, (139, 137, 128)),
    ),
    "angkor-wat": (
        ("BASIN", 9, (48, 55, 52)), ("STONE", 11, (139, 134, 125)),
        ("STONE_BASE", 8, (115, 112, 106)), ("STONE_DK", 11, (92, 89, 85)),
        ("STONE_WARM", 11, (128, 111, 91)), ("VOID", 15, (13, 14, 17)),
    ),
    "buckingham-palace": (
        ("BAR", 16, (232, 232, 227)), ("FLAG_BLUE", 16, (13, 23, 89)),
        ("FLAG_GOLD", 16, (222, 168, 26)), ("FLAG_RED", 16, (184, 15, 23)),
        ("GOLD", 16, (219, 157, 41)), ("PAVE", 8, (139, 136, 133)),
        ("ROOF", 12, (73, 76, 80)), ("STONE", 17, (204, 198, 184)),
        ("STONE_TRIM", 11, (167, 158, 140)), ("WINDOW", 15, (8, 10, 13)),
    ),
    "burj-al-arab": (
        ("GLASS", 15, (40, 84, 138)), ("GREEN", 12, (27, 97, 43)),
        ("MAST", 16, (178, 181, 184)), ("WHITE", 17, (235, 233, 226)),
    ),
    "burj-khalifa-authored": (
        ("GLASS", 15, (73, 94, 120)), ("GLASS_DARK", 15, (31, 41, 54)),
        ("GLASS_MID", 15, (88, 108, 131)), ("ROOF", 16, (219, 222, 226)),
        ("SILVER", 16, (204, 209, 215)), ("SPIRE", 16, (178, 185, 194)),
    ),
    "brooklyn-bridge-authored": (
        ("CABLE", 16, (52, 48, 43)), ("GRANITE", 11, (162, 145, 122)),
        ("GRANITE_DK", 11, (119, 105, 87)), ("ROAD", 6, (79, 79, 83)),
        ("STEEL", 16, (27, 25, 23)), ("WOOD", 8, (168, 139, 94)),
    ),
    "chrysler-building": (
        ("black_granite", 11, (13, 13, 15)), ("crown_window", 15, (23, 28, 38)),
        ("dark_brick", 11, (51, 46, 43)), ("frieze_gray", 11, (140, 138, 133)),
        ("gray_spandrel", 11, (107, 107, 112)),
        ("nirosta_shade", 16, (133, 138, 145)),
        ("nirosta_steel", 16, (178, 184, 191)),
        ("white_brick", 11, (217, 214, 204)),
        ("white_marble", 17, (235, 232, 222)),
        ("window_glass", 15, (20, 26, 36)),
    ),
    "cn-tower": (
        ("ANT", 16, (204, 207, 209)), ("ANT_DARK", 16, (84, 89, 97)),
        ("CONCRETE", 8, (184, 178, 167)), ("GLASS", 15, (22, 27, 34)),
        ("WHITE", 17, (229, 229, 224)),
    ),
    "flatiron-building": (
        ("CORN", 11, (147, 136, 111)), ("LIME", 11, (196, 189, 168)),
        ("LIME_R", 11, (161, 150, 129)), ("PLINTH", 8, (122, 117, 107)),
        ("ROOF", 12, (82, 79, 84)), ("SHOP", 15, (33, 28, 23)),
        ("TERRA", 11, (187, 163, 125)), ("TOP", 17, (204, 195, 172)),
        ("WIN", 15, (26, 28, 33)),
    ),
    "golden-gate-bridge-authored": (
        ("CONCRETE", 8, (87, 82, 71)), ("CONC_DK", 8, (55, 51, 45)),
        ("LANE", 6, (206, 194, 141)), ("ORANGE", 16, (206, 18, 1)),
        ("ORANGE_DK", 16, (100, 9, 1)), ("ROAD", 6, (11, 11, 12)),
        ("WALK", 8, (47, 44, 38)),
    ),
    "golden-temple": (
        ("DARK_BAND", 11, (88, 68, 50)), ("FRAME", 16, (242, 194, 71)),
        ("GOLD", 16, (237, 180, 59)), ("GOLD_DEEP", 16, (150, 101, 29)),
        ("MARBLE", 17, (232, 224, 205)), ("MARBLE_SHADE", 17, (194, 184, 161)),
        ("VOID", 15, (11, 11, 14)),
    ),
    "himeji-castle": (
        # Ibushi roof tiles use cool charcoal tints rather than black.
        ("PLASTER", 17, (224, 222, 214)), ("RIDGE", 17, (205, 207, 204)),
        ("SAND", 10, (151, 143, 124)), ("STONE", 11, (112, 110, 105)),
        ("STONE_DK", 11, (82, 82, 80)), ("TILE", 12, (73, 82, 96)),
        ("TILE_LT", 12, (112, 122, 137)), ("TIMBER", 11, (55, 48, 43)),
        ("VOID", 15, (26, 30, 36)),
    ),
    "kremlin": (
        ("BRICK", 11, (95, 23, 18)), ("BRICK_DK", 11, (73, 17, 14)),
        ("BRONZE", 16, (48, 38, 24)), ("CATH_WHITE", 17, (237, 232, 218)),
        ("CLOCK", 15, (9, 9, 11)), ("FACET", 17, (215, 207, 189)),
        ("FLAG_B", 16, (26, 56, 140)), ("FLAG_R", 16, (184, 26, 31)),
        ("FLAG_W", 16, (235, 235, 235)), ("GLASS", 15, (28, 36, 43)),
        ("GOLD", 16, (222, 168, 41)), ("GREEN", 12, (19, 77, 38)),
        ("MODERN", 17, (227, 227, 222)), ("NAVY", 12, (19, 28, 66)),
        ("ROOF_GRN", 12, (34, 92, 59)), ("SILVER", 16, (178, 182, 189)),
        ("STAR", 16, (194, 11, 11)), ("VOID", 15, (11, 12, 14)),
        ("WHITE", 17, (226, 220, 204)), ("YELLOW", 11, (218, 173, 85)),
    ),
    "pisa": (
        ("MARBLE", 17, (222, 215, 200)), ("MARBLE_SHADE", 11, (186, 177, 159)),
        ("VOID", 15, (14, 17, 22)),
    ),
    "louvre-pyramid": (
        ("DARK", 15, (13, 15, 18)), ("FRAME", 16, (143, 134, 120)),
        ("GLASS", 15, (140, 166, 184)),
    ),
    "marina-bay-sands": (
        ("ATRIUM", 15, (33, 48, 66)), ("DECK", 17, (215, 210, 198)),
        ("GLASS_A", 15, (84, 115, 143)), ("GLASS_B", 15, (70, 98, 126)),
        ("POOL", 9, (33, 133, 168)), ("WHITE", 17, (235, 231, 219)),
    ),
    "merdeka-118": (
        ("ENTRY", 15, (15, 19, 23)), ("GLASS", 15, (193, 205, 218)),
        ("GLASS_B", 15, (178, 193, 208)), ("GLASS_C", 15, (164, 180, 198)),
        ("MAST", 16, (189, 191, 195)), ("PODIUM", 15, (60, 73, 85)),
        ("PODIUM_B", 15, (46, 57, 70)), ("ROOF", 12, (38, 40, 42)),
        ("SPIRE_CL", 16, (204, 210, 217)),
    ),
    "merlion-authored": (
        ("IVORY", 17, (224, 219, 204)),
    ),
    "notre-dame": (
        ("BRONZE", 16, (77, 116, 87)), ("CHIMERA", 11, (79, 75, 68)),
        ("GLASS", 15, (24, 29, 47)), ("GLASS_ROSE", 15, (18, 23, 59)),
        ("GOLD", 16, (218, 158, 43)), ("ROOF", 12, (110, 116, 126)),
        ("SPIRE", 16, (89, 94, 106)), ("STONE", 11, (194, 184, 163)),
        ("STONE_DARK", 11, (139, 129, 111)), ("STONE_TRIM", 17, (215, 208, 190)),
        ("VOID", 15, (13, 15, 20)),
    ),
    "one-world-trade-center": (
        ("BEACON", 16, (235, 236, 237)), ("GLASS", 15, (79, 107, 133)),
        ("GLASS_DARK", 15, (13, 18, 24)), ("GLASS_PARA", 15, (143, 168, 194)),
        ("MECH", 16, (122, 126, 133)), ("PODIUM", 16, (106, 116, 131)),
        ("ROOF", 12, (33, 34, 38)), ("SPIRE", 16, (153, 157, 163)),
    ),
    "nyhavn": (
        # Muted runtime tints retain the scene's per-house colour sequence.
        ("AWN_BLUE", 11, (69, 81, 112)), ("AWN_BURGUNDY", 11, (112, 55, 64)),
        ("AWN_CREAM", 11, (211, 202, 177)), ("AWN_GREEN", 11, (67, 99, 79)),
        ("BOLLARD", 16, (54, 59, 56)), ("BRICK", 11, (137, 91, 75)),
        ("BRICK_RED", 11, (154, 78, 62)), ("CHIMNEY", 11, (133, 101, 83)),
        ("COBALT", 11, (92, 111, 145)), ("COPPER", 16, (102, 148, 128)),
        ("CREAM", 11, (221, 214, 194)), ("DARK_BLUE", 11, (48, 59, 70)),
        ("DEEP_RED", 11, (137, 60, 58)), ("DOOR_BLACK", 11, (38, 39, 42)),
        ("DOOR_GREEN", 11, (59, 82, 66)), ("GLAZING", 15, (39, 47, 55)),
        ("KBH_WHITE", 17, (219, 218, 212)), ("KERB", 8, (165, 161, 153)),
        ("MUSTARD", 11, (174, 139, 72)), ("OCHRE", 11, (196, 157, 90)),
        ("ORANGE", 11, (196, 126, 76)), ("PALE_BLUE", 11, (166, 190, 201)),
        ("PALE_YELLOW", 11, (222, 204, 151)), ("PLINTH", 8, (104, 101, 96)),
        ("QUAY", 8, (145, 142, 136)), ("QUAY_WALL", 8, (119, 117, 112)),
        ("ROOF_GREY", 12, (91, 93, 96)), ("SALMON", 11, (203, 151, 129)),
        ("TILE_DARK", 12, (107, 65, 53)), ("TILE_RED", 12, (145, 78, 57)),
        ("TRIM", 17, (229, 228, 222)), ("WOOD_DARK", 11, (74, 61, 50)),
    ),
    "parthenon": (
        ("FRIEZE_BAND", 11, (153, 102, 72)), ("MARBLE", 11, (207, 180, 133)),
        ("MARBLE_LIGHT", 17, (221, 200, 159)), ("MARBLE_SHADE", 11, (144, 122, 88)),
        ("TYMPANUM", 11, (113, 95, 73)), ("VOID", 15, (13, 14, 18)),
    ),
    "petronas-towers": (
        ("ENTRY", 15, (15, 20, 23)), ("GLASS_GRN", 15, (28, 48, 43)),
        ("MAST", 16, (209, 212, 214)), ("ROOF", 12, (36, 37, 40)),
        ("STEEL", 16, (194, 196, 199)), ("STEEL_LO", 16, (173, 177, 181)),
    ),
    "st-basils": (
        ("BRICK", 11, (140, 40, 22)), ("BRICK_DK", 11, (102, 26, 14)),
        # Insert white before field colours so shared cells retain field colour.
        ("D_WHITE", 17, (237, 237, 229)), ("D_BLUE", 16, (24, 69, 161)),
        ("D_GREEN", 16, (18, 99, 41)), ("D_RED", 16, (173, 23, 17)),
        ("D_YELLOW", 16, (227, 176, 33)), ("GOLD", 16, (222, 163, 38)),
        ("ROOF_GREEN", 12, (27, 77, 43)), ("TENT_GREEN", 12, (20, 84, 40)),
        ("VOID", 15, (12, 13, 15)), ("WHITE", 17, (235, 226, 209)),
    ),
    "steinway-tower": (
        ("BRICK", 11, (120, 82, 64)), ("BRONZE", 16, (79, 52, 27)),
        ("COPPER", 16, (84, 133, 110)), ("GLASS", 15, (28, 33, 38)),
        ("GLASS_DARK", 15, (10, 12, 14)), ("LIMESTONE", 17, (204, 195, 176)),
        ("LIME_RUST", 11, (178, 168, 149)), ("TERRA", 17, (233, 226, 214)),
    ),
    "taipei-101": (
        ("CROWN", 16, (97, 103, 108)), ("GLASS", 15, (26, 73, 66)),
        ("GLASS_DARK", 15, (11, 37, 34)), ("SILVER", 16, (189, 196, 201)),
        ("STEEL", 16, (61, 64, 69)),
    ),
    "the-gherkin": (
        ("DIAGRID", 16, (23, 24, 27)), ("GLASS_DARK", 15, (7, 11, 15)),
        ("GLASS_LIGHT", 15, (120, 147, 168)), ("LENS", 15, (31, 47, 61)),
        ("TRIM", 16, (14, 15, 18)),
    ),
    "the-shard": (
        ("CORE", 16, (112, 117, 125)), ("DECK", 8, (163, 149, 128)),
        ("EDGE", 16, (237, 240, 242)), ("GLASS", 15, (133, 168, 199)),
        ("GLASS_LAP", 15, (102, 138, 173)), ("INNER", 15, (26, 38, 54)),
        ("STEEL", 16, (168, 173, 181)),
    ),
    "tower-bridge-authored": (
        ("BLUE", 16, (42, 78, 135)), ("BLUE_DARK", 16, (29, 52, 96)),
        ("FLAG_BLUE", 16, (14, 20, 84)), ("FLAG_RED", 16, (191, 18, 28)),
        ("FLAG_WHITE", 17, (235, 235, 242)), ("GLASS_WALK", 15, (26, 33, 43)),
        ("GOLD", 16, (217, 156, 43)), ("GRANITE", 11, (102, 103, 107)),
        ("IRON", 16, (56, 57, 62)), ("PAVE", 8, (143, 140, 134)),
        ("RED", 16, (178, 26, 31)), ("ROAD", 6, (69, 69, 71)),
        ("ROOF_LEAD", 12, (89, 94, 102)), ("STONE", 11, (193, 182, 162)),
        ("STONE_SHADE", 11, (139, 133, 119)),
        ("STONE_TRIM", 17, (223, 217, 200)),
        ("WHITE_STEEL", 16, (232, 235, 240)), ("WINDOW", 15, (12, 15, 22)),
    ),
    "white-house": (
        ("FLAG_BLUE", 16, (14, 22, 82)), ("FLAG_RED", 16, (184, 20, 26)),
        ("FLAG_WHITE", 16, (240, 240, 242)), ("IRON", 16, (59, 60, 64)),
        ("ROOF", 12, (102, 104, 107)), ("STEP", 8, (157, 153, 147)),
        ("TRIM", 17, (245, 242, 233)), ("WHITE", 17, (229, 224, 211)),
        ("WINDOW", 15, (11, 15, 14)),
    ),
}


PRESERVE_AUTHORED_ORIGIN = {
    "opera",
    "angkor-wat",
    "brooklyn-bridge-authored",
    "chrysler-building",
    "flatiron-building",
    "golden-gate-bridge-authored",
    "himeji-castle",
    "merdeka-118",
    "merlion-authored",
    "one-world-trade-center",
    "petronas-towers",
    "steinway-tower",
    "the-gherkin",
    "the-shard",
    "tower-bridge-authored",
}


AUTHORING_X_TO_RUNTIME_Z = {
    "brooklyn-bridge-authored",
    "golden-gate-bridge-authored",
    "merdeka-118",
    "merlion-authored",
    "petronas-towers",
    "tower-bridge-authored",
}


PALETTES = {
    "liberty": LIBERTY_PALETTE,
    "taj": TAJ_PALETTE,
    "sagrada": SAGRADA_PALETTE,
    "big-ben": BIG_BEN_PALETTE,
    "christ": CHRIST_PALETTE,
    "brooklyn": BROOKLYN_PALETTE,
    "london-eye": LONDON_EYE_PALETTE,
    "merlion": MERLION_PALETTE,
    "space-needle": SPACE_NEEDLE_PALETTE,
    "giza": GIZA_PALETTE,
    "colosseum": COLOSSEUM_PALETTE,
    "brandenburg": BRANDENBURG_PALETTE,
    "arc": ARC_PALETTE,
    "tower-bridge": TOWER_BRIDGE_PALETTE,
    "parliament": PARLIAMENT_PALETTE,
    "empire-state": EMPIRE_STATE_PALETTE,
    "eiffel": EIFFEL_PALETTE,
    "tokyo-tower": TOKYO_TOWER_PALETTE,
    "washington": WASHINGTON_PALETTE,
    **{
        kind: tuple((runtime_material, rgb) for _, runtime_material, rgb in entries)
        for kind, entries in AUTHORED_PALETTES.items()
    },
}


def _ldraw_sections(path: Path) -> dict[str, list[str]]:
    sections: dict[str, list[str]] = {}
    current = path.name.lower()
    sections[current] = []
    for raw in path.read_text(errors="replace").splitlines():
        if raw.upper().startswith("0 FILE "):
            current = raw[7:].strip().replace("\\", "/").lower()
            sections.setdefault(current, [])
        else:
            sections[current].append(raw)
    return sections


def ldraw_scene(path: Path, library_zip: Path) -> list[tuple[trimesh.Trimesh, str]]:
    """Resolve an LDraw MPD into triangle meshes grouped by colour."""
    sections = _ldraw_sections(path)
    root = next(name for name, lines in sections.items() if lines)
    archive = zipfile.ZipFile(library_zip)
    names = {name.lower(): name for name in archive.namelist()}
    parsed: dict[str, list[str]] = dict(sections)
    triangles: dict[int, list[np.ndarray]] = {}

    def lines_for(name: str) -> list[str]:
        key = name.replace("\\", "/").lower()
        if key in parsed:
            return parsed[key]
        candidates = (
            f"ldraw/parts/{key}",
            f"ldraw/p/{key}",
            f"ldraw/parts/s/{key}",
            f"ldraw/models/{key}",
        )
        found = next((names[c] for c in candidates if c in names), None)
        if found is None:
            raise FileNotFoundError(f"LDraw dependency not found: {name}")
        parsed[key] = archive.read(found).decode("utf-8", errors="replace").splitlines()
        return parsed[key]

    def visit(name: str, transform: np.ndarray, inherited: int, depth: int) -> None:
        if depth > 40:
            raise ValueError(f"LDraw recursion too deep at {name}")
        for raw in lines_for(name):
            fields = raw.strip().split()
            if not fields:
                continue
            record = fields[0]
            if record == "1" and len(fields) >= 15:
                color = int(fields[1])
                if color in (16, 24):
                    color = inherited
                child = np.eye(4)
                child[:3, 3] = np.asarray(fields[2:5], dtype=float)
                child[:3, :3] = np.asarray(fields[5:14], dtype=float).reshape(3, 3)
                visit(" ".join(fields[14:]), transform @ child, color, depth + 1)
            elif record in ("3", "4"):
                color = int(fields[1])
                if color in (16, 24):
                    color = inherited
                count = 3 if record == "3" else 4
                points = np.asarray(fields[2 : 2 + count * 3], dtype=float).reshape(count, 3)
                points = trimesh.transform_points(points, transform)
                triangles.setdefault(color, []).append(points[:3])
                if count == 4:
                    triangles[color].append(points[[0, 2, 3]])

    visit(root, np.eye(4), 16, 0)
    out: list[tuple[trimesh.Trimesh, str]] = []
    for color, faces in triangles.items():
        tri = np.asarray(faces)
        mesh = trimesh.Trimesh(
            vertices=tri.reshape(-1, 3),
            faces=np.arange(len(tri) * 3).reshape(-1, 3),
            process=False,
        )
        # LDraw is Y-down. Low Poly Earth authoring scenes are Y-up.
        mesh.vertices[:, 1] *= -1.0
        out.append((mesh, f"ldraw:{color}"))
    return out


def _crop(mesh: trimesh.Trimesh, keep: np.ndarray) -> trimesh.Trimesh | None:
    if not np.any(keep):
        return None
    return mesh.submesh([np.flatnonzero(keep)], append=True, repair=False)


def transformed_scene(
    path: Path, kind: str, ldraw_library: Path | None = None
) -> list[tuple[trimesh.Trimesh, str]]:
    if kind == "washington":
        # Keep the 152.4 m shaft and 16.894 m pyramidion as separate materials.
        def frustum(y0: float, y1: float, w0: float, w1: float) -> trimesh.Trimesh:
            vertices = np.asarray(
                [
                    [-w0 / 2, y0, -w0 / 2], [w0 / 2, y0, -w0 / 2],
                    [w0 / 2, y0, w0 / 2], [-w0 / 2, y0, w0 / 2],
                    [-w1 / 2, y1, -w1 / 2], [w1 / 2, y1, -w1 / 2],
                    [w1 / 2, y1, w1 / 2], [-w1 / 2, y1, w1 / 2],
                ],
                dtype=float,
            )
            faces = np.asarray(
                [
                    [0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7],
                    [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5],
                    [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7],
                ],
                dtype=np.int64,
            )
            return trimesh.Trimesh(vertices=vertices, faces=faces, process=False)

        base_w = 16.764
        top_w = 10.49
        break_y = 45.72
        break_w = base_w + (top_w - base_w) * break_y / 152.4
        lower = frustum(0.0, break_y, base_w, break_w)
        upper = frustum(break_y, 152.4, break_w, top_w)
        apex = np.asarray(
            [
                [-top_w / 2, 152.4, -top_w / 2],
                [top_w / 2, 152.4, -top_w / 2],
                [top_w / 2, 152.4, top_w / 2],
                [-top_w / 2, 152.4, top_w / 2],
                [0.0, 169.294, 0.0],
            ],
            dtype=float,
        )
        pyramidion = trimesh.Trimesh(
            vertices=apex,
            faces=np.asarray(
                [[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4], [0, 3, 2], [0, 2, 1]],
                dtype=np.int64,
            ),
            process=False,
        )
        return [
            (lower, "washington:lower-marble"),
            (upper, "washington:upper-marble"),
            (pyramidion, "washington:pyramidion"),
        ]

    if kind == "tower-bridge":
        if ldraw_library is None:
            raise ValueError(f"{kind} conversion requires --ldraw-library complete.zip")
        meshes = ldraw_scene(path, ldraw_library)
    else:
        scene = trimesh.load(path, force="scene", process=False)
        meshes: list[tuple[trimesh.Trimesh, str]] = []
        for node in scene.graph.nodes_geometry:
            transform, geometry_name = scene.graph[node]
            mesh = scene.geometry[geometry_name].copy()
            if not isinstance(mesh, trimesh.Trimesh):
                continue
            mesh.apply_transform(transform)
            material = getattr(getattr(mesh.visual, "material", None), "name", "") or ""
            node_name = str(node).lower()

            if kind == "parliament" and material == "CLOCK_BLUE":
                material = (
                    "CLOCK_BLUE_DIAL"
                    if str(node).lower().startswith("et_dialblue")
                    else "CLOCK_BLUE_DETAIL"
                )

            if kind == "christ":
                # Remove surrounding terrain from the photogrammetry mesh.
                centres = mesh.triangles_center
                mesh = _crop(
                    mesh,
                    (centres[:, 1] >= 0.0)
                    & (np.linalg.norm(centres[:, [0, 2]], axis=1) <= 0.8),
                )
            elif kind == "brooklyn":
                # Crop the riverbanks outside the bridge corridor.
                centres = mesh.triangles_center
                along_axis = np.asarray([1.0, -0.0735])
                along_axis /= np.linalg.norm(along_axis)
                cross_axis = np.asarray([-along_axis[1], along_axis[0]])
                origin = np.asarray([-1.1, 5.43])
                delta = centres[:, [0, 2]] - origin
                along = delta @ along_axis
                cross = delta @ cross_axis
                mesh = _crop(
                    mesh,
                    (np.abs(cross) <= 0.78)
                    & (along >= -17.5)
                    & (along <= 20.0)
                    & (centres[:, 1] >= 0.08),
                )
            elif kind == "london-eye":
                # Crop the South Bank while retaining the wheel and its base.
                centres = mesh.triangles_center
                mesh = _crop(
                    mesh,
                    (centres[:, 0] >= -3.7)
                    & (centres[:, 0] <= 2.4)
                    & (centres[:, 1] >= 0.12)
                    & (centres[:, 2] >= -2.1)
                    & (centres[:, 2] <= 0.85),
                )
            elif kind == "brandenburg":
                # Remove the presentation plinth below the gate.
                centres = mesh.triangles_center
                mesh = _crop(mesh, centres[:, 1] >= 60.5)
            elif kind == "parliament":
                # Remove labelled context geometry outside the Palace and terrace.
                context_materials = {"PAVE", "RIVER", "GRASS", "LEAF", "TRUNK"}
                if material.upper() in context_materials or node_name.startswith("npy_wall"):
                    mesh = None
            if mesh is not None and len(mesh.faces):
                meshes.append((mesh, material))

    if kind == "giza":
        # Normalize each pyramid crop independently and discard scan terrain.
        regions = (
            ([-1.05, 1.53], 1.43, 1.00, [0.0, 0.0], 115.0, 139.0, "khufu"),
            ([-0.98, -2.14], 1.35, 1.00, [-345.0, -390.0], 107.5, 136.0, "khafre"),
            ([-0.57, 4.92], 0.67, 1.02, [-780.0, -740.0], 52.0, 65.0, "menkaure"),
        )
        cropped: list[tuple[trimesh.Trimesh, str]] = []
        for mesh, _ in meshes:
            centres = mesh.triangles_center
            for centre_xz, source_half, source_base, target_xz, target_half, target_h, label in regions:
                delta = centres[:, [0, 2]] - np.asarray(centre_xz)
                piece = _crop(
                    mesh,
                    (np.abs(delta[:, 0]) <= source_half)
                    & (np.abs(delta[:, 1]) <= source_half)
                    & (centres[:, 1] >= source_base - 0.04),
                )
                if piece is None:
                    continue
                old = piece.vertices.copy()
                horizontal = target_half / source_half
                source_top = 2.205 if label == "khufu" else (2.16 if label == "khafre" else 1.48)
                vertical = target_h / (source_top - source_base)
                piece.vertices = np.column_stack(
                    (
                        (old[:, 0] - centre_xz[0]) * horizontal + target_xz[0],
                        (old[:, 1] - source_base) * vertical,
                        (old[:, 2] - centre_xz[1]) * horizontal + target_xz[1],
                    )
                )
                cropped.append((piece, f"giza:{label}"))
        meshes = cropped
        # Rebase each retained crop from its own bounds to y=0 and target height.
        target_heights = {"giza:khufu": 139.0, "giza:khafre": 136.0, "giza:menkaure": 65.0}
        for label, target_h in target_heights.items():
            group = [mesh for mesh, material in meshes if material == label]
            if not group:
                continue
            group_bounds = np.vstack([mesh.bounds for mesh in group])
            group_low = group_bounds[:, 1].min()
            group_high = group_bounds[:, 1].max()
            for mesh in group:
                mesh.vertices[:, 1] = (mesh.vertices[:, 1] - group_low) * target_h / (group_high - group_low)

    bounds = np.vstack([mesh.bounds for mesh, _ in meshes])
    low = bounds.min(axis=0)
    high = bounds.max(axis=0)
    centre = (low + high) * 0.5

    if kind == "liberty":
        # Convert Z-up to Y-up and scale the complete model to 92.99 m.
        scale = 92.99 / (high[2] - low[2])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * scale,
                    (old[:, 2] - low[2]) * scale,
                    -(old[:, 1] - centre[1]) * scale,
                )
            )
    elif kind in {"taj", "sagrada"}:
        # Convert Z-up sources and normalize horizontal and vertical extents
        # independently because their source proportions are not world scale.
        targets = {
            "taj": (105.0, 73.0),
            "sagrada": (100.0, 172.5),
        }
        target_horizontal, target_height = targets[kind]
        horizontal_extent = max(high[0] - low[0], high[1] - low[1])
        horizontal_scale = target_horizontal / horizontal_extent
        vertical_scale = target_height / (high[2] - low[2])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * horizontal_scale,
                    (old[:, 2] - low[2]) * vertical_scale,
                    -(old[:, 1] - centre[1]) * horizontal_scale,
                )
            )
    elif kind == "christ":
        # Uniformly scale the complete Y-up model to 38 m.
        scale = 38.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * scale,
                    (old[:, 1] - low[1]) * scale,
                    (old[:, 2] - centre[2]) * scale,
                )
            )
    elif kind == "big-ben":
        # Normalize the Y-up model independently on each axis.
        sx = 13.0 / (high[0] - low[0])
        sz = 13.0 / (high[2] - low[2])
        sy = 96.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * sx,
                    (old[:, 1] - low[1]) * sy,
                    (old[:, 2] - centre[2]) * sz,
                )
            )
    elif kind == "brooklyn":
        axis = np.asarray([1.0, -0.0735])
        axis /= np.linalg.norm(axis)
        cross_axis = np.asarray([-axis[1], axis[0]])
        origin = np.asarray([-1.1, 5.43])
        scale = 39.8  # Maps source tower separation to a 486 m main span.
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            delta = old[:, [0, 2]] - origin
            mesh.vertices = np.column_stack(
                (
                    (delta @ cross_axis) * scale,
                    (old[:, 1] - 0.08) * scale,
                    (delta @ axis) * scale,
                )
            )
    elif kind == "london-eye":
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] + 0.65) * (120.0 / 5.7),
                    (old[:, 1] - 0.12) * (135.0 / (5.78 - 0.12)),
                    (old[:, 2] + 0.50) * (25.0 / 2.95),
                )
            )
    elif kind == "merlion":
        # Preserve source proportions and scale the complete model to 8.6 m.
        scale = 8.6 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * scale,
                    (old[:, 1] - low[1]) * scale,
                    (old[:, 2] - centre[2]) * scale,
                )
            )
    elif kind == "space-needle":
        sxz = 44.0 / max(high[0] - low[0], high[2] - low[2])
        sy = 184.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * sxz,
                    (old[:, 1] - low[1]) * sy,
                    (old[:, 2] - centre[2]) * sxz,
                )
            )
    elif kind == "giza":
        pass  # Pyramid crops are already normalized above.
    elif kind == "colosseum":
        sx = 190.0 / (high[2] - low[2])
        sz = 158.0 / (high[0] - low[0])
        sy = 49.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 2] - centre[2]) * sx,
                    (old[:, 1] - low[1]) * sy,
                    -(old[:, 0] - centre[0]) * sz,
                )
            )
        footing = trimesh.creation.cylinder(radius=1.0, height=10.0, sections=128)
        old = footing.vertices.copy()
        # Rotate the Z-up cylinder to Y-up and scale it to the target ellipse.
        footing.vertices = np.column_stack((old[:, 0] * 94.0, old[:, 2], old[:, 1] * 78.0))
        footing.apply_translation([0.0, -5.0, 0.0])
        meshes.append((footing, "colosseum:foundation"))
    elif kind == "brandenburg":
        sx = 68.0 / (high[2] - low[2])
        sz = 24.0 / (high[0] - low[0])
        sy = 32.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 2] - centre[2]) * sx,
                    (old[:, 1] - low[1]) * sy,
                    -(old[:, 0] - centre[0]) * sz,
                )
            )
        footing = trimesh.creation.box(extents=[66.0, 3.0, 22.0])
        footing.apply_translation([0.0, -1.5, 0.0])
        meshes.append((footing, "brandenburg:foundation"))
    elif kind == "arc":
        # Normalize the Y-up model to a 45.1 × 22.2 × 50 m envelope.
        sx = 45.1 / (high[0] - low[0])
        sz = 22.2 / (high[2] - low[2])
        sy = 50.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * sx,
                    (old[:, 1] - low[1]) * sy,
                    (old[:, 2] - centre[2]) * sz,
                )
            )
        footing = trimesh.creation.box(extents=[44.5, 3.0, 21.6])
        footing.apply_translation([0.0, -1.5, 0.0])
        meshes.append((footing, "arc:foundation"))
    elif kind == "tower-bridge":
        sx = 34.0 / (high[0] - low[0])
        sy = 65.0 / (high[1] - low[1])
        sz = 244.0 / (high[2] - low[2])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * sx,
                    (old[:, 1] - low[1]) * sy,
                    (old[:, 2] - centre[2]) * sz,
                )
            )
    elif kind == "parliament":
        # Keep the Y-up vertical scale and ground plane. Flip Z and apply the
        # horizontal control-point correction without scaling height.
        ground = 0.0 if low[1] <= 0.0 <= high[1] else low[1]
        horizontal_scale = 1.099592343
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * horizontal_scale,
                    old[:, 1] - ground,
                    -(old[:, 2] - centre[2]) * horizontal_scale,
                )
            )
        material_order = (
            "STONE",
            "STONE_DARK",
            "ROOF",
            "ROOF_GREY",
            "STONE_TRIM",
            "WINDOW",
            "CLOCK_BLUE_DIAL",
            "CLOCK_WHITE",
            "CLOCK_BLUE_DETAIL",
            "GOLD",
            "IRON",
            "FLAG_BLUE",
            "FLAG_WHITE",
            "FLAG_RED",
        )
        meshes = [
            (
                trimesh.util.concatenate(
                    [mesh for mesh, source_material in meshes if source_material == material]
                ),
                material,
            )
            for material in material_order
            if any(source_material == material for _, source_material in meshes)
        ]
    elif kind == "empire-state":
        sx = 124.0 / (high[0] - low[0])
        sy = 443.2 / (high[1] - low[1])
        sz = 64.0 / (high[2] - low[2])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * sx,
                    (old[:, 1] - low[1]) * sy,
                    (old[:, 2] - centre[2]) * sz,
                )
            )
    elif kind == "eiffel":
        sxz = 125.0 / max(high[0] - low[0], high[2] - low[2])
        sy = 330.0 / (high[1] - low[1])
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    (old[:, 0] - centre[0]) * sxz,
                    (old[:, 1] - low[1]) * sy,
                    (old[:, 2] - centre[2]) * sxz,
                )
            )
    elif kind == "tokyo-tower":
        # Preserve the Y-up metre scale and use y=0 as architectural grade;
        # overlapping lattice solids extend slightly below it.
        ground = 0.0 if low[1] <= 0.0 <= high[1] else low[1]
        for mesh, _ in meshes:
            old = mesh.vertices.copy()
            mesh.vertices = np.column_stack(
                (
                    old[:, 0] - centre[0],
                    old[:, 1] - ground,
                    old[:, 2] - centre[2],
                )
            )
        # Merge same-material beams to reduce voxelizer calls across all LODs.
        meshes = [
            (
                trimesh.util.concatenate(
                    [mesh for mesh, source_material in meshes if source_material == material]
                ),
                material,
            )
            for material in sorted({source_material for _, source_material in meshes})
        ]
    elif kind in AUTHORED_PALETTES:
        # Map the exported Y-up frame to runtime +X east/+Z north. Preserve
        # metre scale and y=0 grade, including shallow below-grade footings.
        ground = 0.0 if low[1] <= 0.0 <= high[1] else low[1]
        if kind == "kremlin":
            # Apply the control-point fit for three mapped corner towers;
            # registry heading supplies the remaining rotation.
            for mesh, _ in meshes:
                old = mesh.vertices.copy()
                source_x = old[:, 0]
                source_z = -old[:, 2]
                mesh.vertices = np.column_stack(
                    (
                        0.916537786 * source_x - 0.005360645 * source_z - 10.30441889,
                        old[:, 1] - ground,
                        1.012193830 * source_z - 1.98328510,
                    )
                )
        else:
            origin_x = 0.0 if kind in PRESERVE_AUTHORED_ORIGIN else centre[0]
            origin_z = 0.0 if kind in PRESERVE_AUTHORED_ORIGIN else centre[2]
            for mesh, _ in meshes:
                old = mesh.vertices.copy()
                if kind in AUTHORING_X_TO_RUNTIME_Z:
                    mesh.vertices = np.column_stack(
                        (
                            old[:, 2] - origin_z,
                            old[:, 1] - ground,
                            old[:, 0] - origin_x,
                        )
                    )
                else:
                    mesh.vertices = np.column_stack(
                        (
                            old[:, 0] - origin_x,
                            old[:, 1] - ground,
                            -(old[:, 2] - origin_z),
                        )
                    )

        expected = {name for name, _, _ in AUTHORED_PALETTES[kind]}
        actual = {material.split("|", 1)[0] for _, material in meshes}
        if actual != expected:
            missing = sorted(expected - actual)
            unknown = sorted(actual - expected)
            raise ValueError(
                f"{kind} authored material mismatch: missing={missing}, unknown={unknown}"
            )

        # The authoring-axis reflection reverses triangle orientation.
        for mesh, _ in meshes:
            mesh.faces = mesh.faces[:, ::-1]
    elif kind == "washington":
        pass  # exact surveyed geometry is authored above
    else:
        raise ValueError(f"unsupported landmark kind: {kind}")
    return meshes


def palette_for_point(kind: str, point: np.ndarray, material: str) -> int:
    x, y, z = map(float, point)
    lowered = material.lower()
    if kind == "liberty":
        return 0 if y < 43.0 else (2 if y >= 89.0 else 1)
    if kind == "taj":
        return 1 if y < 5.0 else (2 if y >= 70.0 else 0)
    if kind == "sagrada":
        return 1 if y >= 150.0 else 0
    if kind == "christ":
        return 0 if y < 8.0 else 1
    if kind == "big-ben":
        if y < 4.0:
            return 9
        if y >= 94.0:
            return 5
        if y >= 80.0:
            return 6 if y < 92.5 else 5
        if 64.0 <= y < 80.0:
            # Clock dials are inset from the outer tower envelope.
            on_face = max(abs(x), abs(z)) >= 3.0
            along_face = z if abs(x) >= abs(z) else x
            radius = np.hypot(along_face, y - 72.3)
            if on_face and radius <= 4.8:
                if radius >= 3.8:
                    return 4
                minute_hand = abs(along_face) <= 0.35 and y >= 72.0
                hour_hand = abs((y - 72.3) - 0.55 * along_face) <= 0.32 and along_face <= 0.4
                return 5 if minute_hand or hour_hand or radius <= 0.55 else 3
            return 2 if y < 67.0 or y > 77.0 else 1
        if (y % 12.0) < 1.2:
            return 8
        return 1 if x + 0.35 * z > 0.0 else 0
    if kind == "brooklyn":
        near_tower = min(abs(z - 243.0), abs(z + 243.0)) < 48.0
        if y < 29.0 and abs(x) < 8.0:
            return 5
        if y < 31.0 and abs(x) < 13.5:
            return 6
        if near_tower and y >= 28.0:
            return 1 if x + z * 0.08 > 0.0 else 0
        if y >= 28.0:
            return 3
        return 4
    if kind == "london-eye":
        radial = np.hypot(x, y - 68.5)
        if y < 5.0:
            return 4
        if radial < 8.0:
            return 3
        angle = np.arctan2(y - 68.5, x)
        capsule_phase = ((angle + np.pi) * 32.0 / (2.0 * np.pi)) % 1.0
        if radial > 52.0 and abs(capsule_phase - 0.5) < 0.19:
            return 2
        if y > 126.0 and radial < 58.0:
            return 5
        return 0 if x + 0.25 * z >= 0.0 else 1
    if kind == "merlion":
        if y < 1.9:
            return 4
        # Classify facial recesses separately from the mosaic shading.
        if y > 4.7 and z < -1.0 and (abs(x) < 0.75 or y > 6.4):
            return 3
        shade = x + 0.45 * z + 0.12 * np.sin(y * 3.1)
        return 0 if shade > 0.45 else (1 if shade > -0.75 else 2)
    if kind == "space-needle":
        if y < 8.0:
            return 5
        if 146.0 <= y <= 166.0:
            radial = np.hypot(x, z)
            if 151.0 <= y <= 159.0 and radial > 11.0:
                return 2
            return 3 if y < 162.5 else 4
        return 0 if x + 0.3 * z > 0.0 else 1
    if kind == "giza":
        if y < 0.0 or "foundation" in lowered:
            return 4
        if "khafre" in lowered and y > 106.0:
            return 3
        if "menkaure" in lowered:
            return 5
        course = int(y / 3.6) % 3
        return (0, 1, 2)[course]
    if kind == "colosseum":
        if y < 0.0 or "foundation" in lowered:
            return 4
        if "sand" in lowered or "floo" in lowered:
            return 4
        if "brown" in lowered:
            return 3
        if "stgr" in lowered and y < 15.0:
            return 5
        if y % 13.0 < 2.2:
            return 3
        return 1 if x + 0.3 * z > 0.0 else (0 if y > 18.0 else 2)
    if kind == "brandenburg":
        if y < 0.0 or "foundation" in lowered:
            return 5
        if y > 25.0 and abs(x) < 13.0:
            return 4
        if 20.0 <= y < 26.0:
            return 3
        return 1 if x + 0.25 * z > 0.0 else (0 if y > 5.0 else 2)
    if kind == "arc":
        if y < 0.0 or "foundation" in lowered:
            return 4
        # Separate vault recesses, registers, and sun-facing limestone.
        if (abs(x) < 7.8 and y < 29.0) or (abs(z) < 4.0 and y < 18.5):
            return 2
        if (8.0 < y < 16.0) or (31.0 < y < 39.5):
            return 3
        if (y % 7.8) < 1.0:
            return 1
        return 1 if x + 0.25 * z > 4.0 else 0
    if kind == "tower-bridge":
        try:
            color = int(lowered.split(":", 1)[1])
        except (IndexError, ValueError):
            color = 71
        return {
            1: 1,    # blue steel
            73: 1,   # medium blue
            15: 2,   # white framing
            47: 3,   # glazing
            0: 4,    # asphalt / black details
            71: 0,   # light stone
            72: 5,   # stone shadow
            19: 0,   # tan stone
            28: 5,   # dark tan stone
            70: 5,   # reddish-brown shadow
            4: 6,    # red accents
            36: 6,
            297: 7,  # gilt details
            14: 7,
            320: 6,
            2: 8,
        }.get(color, 0)
    if kind == "parliament":
        return {
            "stone": 0,
            "stone_trim": 1,
            "stone_dark": 2,
            "roof": 3,
            "roof_grey": 4,
            "window": 5,
            "gold": 6,
            "clock_white": 7,
            "clock_blue_dial": 8,
            "clock_blue_detail": 8,
            "flag_red": 9,
            "flag_blue": 10,
            "flag_white": 11,
            "iron": 12,
        }.get(lowered, 0)
    if kind == "empire-state":
        if y < 4.0:
            return 6
        if y > 441.0:
            return 7
        if y > 381.0:
            return 5 if x + 0.3 * z > 0.0 else 4
        if y > 318.0:
            return 4 if (int(y / 8.0) % 2) else 1
        # Alternate window bands and pale setback cornices.
        floor_phase = (y / 3.66) % 1.0
        if 0.28 <= floor_phase <= 0.66 and y > 16.0:
            return 3
        if (y % 18.3) < 1.3:
            return 1
        return 0 if x + 0.28 * z > 0.0 else 2
    if kind == "eiffel":
        if y < 3.0:
            return 5
        if y > 326.0:
            return 6
        if y > 300.0:
            return 4
        if any(abs(y - level) < 3.0 for level in (57.0, 116.0, 276.0)):
            return 3
        return 2 if x + 0.35 * z > 7.0 else (0 if x + 0.35 * z > -12.0 else 1)
    if kind == "tokyo-tower":
        # Preserve per-part materials instead of applying height bands.
        if "glass" in lowered:
            return 3
        if "white" in lowered:
            return 1
        if "orange" in lowered:
            return 0 if x + 0.3 * z >= 0.0 else 2
        return 4
    if kind == "washington":
        if y > 168.55:
            return 5
        if "pyramidion" in lowered or y > 152.0:
            return 3
        width = 16.764 + (10.49 - 16.764) * np.clip(y, 0.0, 152.4) / 152.4
        face_x = abs(abs(x) - width * 0.5) < 0.75
        face_z = abs(abs(z) - width * 0.5) < 0.75
        window_x = abs(abs(z) - 1.7) < 0.65
        window_z = abs(abs(x) - 1.7) < 0.65
        if 149.0 <= y <= 151.5 and ((face_x and window_x) or (face_z and window_z)):
            return 4
        if y < 7.0 and z < -width * 0.5 + 0.75 and abs(x) < 1.6:
            return 4
        if "lower-marble" in lowered or y < 45.72:
            return 1 if x + 0.3 * z > 0.0 else 2
        return 0 if x + 0.3 * z > 0.0 else 2
    if kind == "white-house" and "|us-flag" in lowered:
        # Classify the unified flag spatially because 0.30 m source stripes
        # collide at the finest 0.60 m voxel pitch.
        if x <= 0.275 and y >= 24.65:
            return 0
        # Alternate at voxel-row scale so both stripe colours remain visible.
        stripe = int(np.clip(np.floor((25.4 - y) / 0.6), 0, 2))
        return 1 if stripe % 2 == 0 else 2
    if kind == "st-basils" and "|white-stripe-dome:" in lowered:
        # Classify one unified surface to keep both colours when pattern faces
        # collide in the same voxel.
        _, spec = lowered.split("|white-stripe-dome:", 1)
        dome_id, paired_color, cx, cz, y0, y1 = spec.split(":")
        cx, cz, y0, y1 = map(float, (cx, cz, y0, y1))
        theta_turn = np.arctan2(z - cz, x - cx) / (2.0 * np.pi)
        height_t = np.clip((y - y0) / max(y1 - y0, 1.0e-6), 0.0, 1.0)
        direction = -1.0 if dome_id in {"s_nikolavelikoretsky", "se_alexandersvirsky"} else 1.0
        phase = ((theta_turn + direction * 0.4375 * height_t) * 8.0) % 1.0
        source_name = "D_WHITE" if phase < 0.28 else paired_color.upper()
        return next(
            index
            for index, (candidate, _, _) in enumerate(AUTHORED_PALETTES[kind])
            if candidate == source_name
        )
    if kind in AUTHORED_PALETTES:
        lowered = lowered.split("|", 1)[0]
        indices = {
            source_material.lower(): index
            for index, (source_material, _, _) in enumerate(AUTHORED_PALETTES[kind])
        }
        try:
            return indices[lowered]
        except KeyError as error:
            raise ValueError(f"unknown {kind} authored material {material!r}") from error
    raise ValueError(f"no palette classifier for {kind}")
