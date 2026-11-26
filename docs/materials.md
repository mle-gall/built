# Materials

A material (`built_core.materials.Material`) is a name, a base and what
sits on top of it. Faces and placements wear a copy of its fields (their
paint, with `mat` naming the material); the registry (`scene.materials`)
holds the named ones. Commands: `docs/core-protocol.md` (`create_material`,
`edit_material`, `set_material_base`, `set_material_pbr`...).

| field | meaning |
|---|---|
| `color` | base colour, three numbers 0–1 (over an image: its tint) |
| `texture` | base image: `{path, sw, sh, rot?}`, the tile in metres |
| `opacity` | 0–1; absent is opaque |
| `finish` | a named finish (`material_finish`), used when there is no PBR surface |
| `pbr` | the PBR recipe below |

## The PBR recipe (`pbr`)

| key | meaning | default when absent |
|---|---|---|
| `metallic`, `roughness` | 0–1 | 0, 1 |
| `normal_scale` | ≥ 0 | 1 |
| `ao_strength` | 0–1 | 1 |
| `metallic_map`, `roughness_map`, `normal_map` (`flip_y`), `ao_map`, `orm_map` | `{path}` images, tiled as the base image | none |
| `emission` | linear RGB, three numbers 0–1 | black |
| `emission_strength` | ≥ 0, multiplies `emission` | 0 |
| `emission_map` | `{path}`, tiled as the base image; multiplied by `emission` | none |

A recipe with only `emission*` keys is not a PBR surface: the finish (or
the default look) still decides roughness and metallic, in the editor's
"PBR surface" switch, in glTF export and in the Blender link. Setting
emission on a finish material that has no surface copies the finish's
roughness and metallic into the recipe, so its look stays.

Saving: the recipe is stored as it is in `.igz` and `.built` documents,
and every `*_map` image is packed into the archive (`file_textures.rs`).
Older files have no emission keys and load unchanged. `.skp` files carry
no emission (`docs/skp-pbr-format.md`).

glTF: `emission` ↔ `emissiveFactor`, `emission_map` ↔ `emissiveTexture`,
`emission_strength` ↔ `KHR_materials_emissive_strength` (written only when
it is not 1; read as 1 when absent). Nothing is written for black or 0.

Editor: the Library's material detail shows Emission (a colour well),
Emission strength and an "Emission map…" slot in its Look section, for
every material.

The viewport adds emission colour times strength to the lit surface in linear RGB. An emission map modulates that contribution using the base image mapping.
