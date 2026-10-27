# Vehicle asset sources

Kestrel uses **Bob**, and Phase Manta uses **Executioner**, by **Quaternius**,
from the [Ultimate Spaceships Pack](https://quaternius.com/packs/ultimatespaceships.html).
The creator also distributes the source archive on
[OpenGameArt](https://opengameart.org/content/lowpoly-spaceships-pack).
Both meshes and their painted albedo textures are **CC0 1.0 Universal**.
The creator's original license text is preserved in
`source/quaternius/License.txt`; the download URLs and hashes are recorded in
`source/quaternius/provenance.json`.

The preserved OBJ, MTL, and PNG files under `source/quaternius/` are unmodified.
`source/build_craft.py` uniformly scales and centers the selected craft, assigns
paint, metal, and glass materials, evaluates small edge bevels and weighted
normals, repairs invalid corner-normal hemispheres, and adds emissive exhaust
cores. Runtime albedo PNGs retain the original painted UV layout and contents.
The original artist's panel design, markings, and surface wear remain intact.

The Sparrow mesh, its Blender builder, and the export tooling are original
Vearth work. Their licensing follows the repository license.
