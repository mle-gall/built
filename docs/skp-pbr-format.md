# PBR materials in `.skp` files (.skp 2025+)

How a .skp 2025+ file stores a material's PBR data (metallic,
roughness, normal map and ambient occlusion), and how Built imports it.
Read by `formats/openskp_compat.py` (`read_pbr`, `attach_pbr`) and mapped to
`core.materials.Material.pbr` by `rust/built_core/src/skp/materials.rs`.

## How this was established (2026-11-19)

Black-box, per [the clean-room boundary](openskp-collaboration.md#clean-room-boundary):
only files and public documentation were read.

- **The shipped material library.** .skp 2026's 290 `.skm` files
  (`.skp.app/Contents/Resources/Materials`): all of them carry the PBR
  block, covering every combination of maps the library uses.
- **Real 2025/2026 `.skp` files** (read-only): 286 VFF files on the
  development Mac were scanned, and two 2026.2 files with PBR maps were
  imported for comparison (not committed).
- **The public .skp C SDK 2025.0** (`.skpAPI/model/material.h`) for
  the meaning of the enums, the value ranges and the image formats. Its
  library wrote `tests/data/skp_pbr_2025.skp` (`scripts/skp_pbr_fixture/`)
  and read it back for the ground truth (`skp_pbr_2025.truth.json`).
- **Which data counts.** A copy of the fixture had its XML edited
  (metallic 0.25 → 0.5, normal style 0 → 1, AO switched off). The SDK read
  back the edited values, so `material.xml` is the authority: `model.dat`
  keeps no second copy of the PBR data.

## Where it lives

A modern `.skp` (2021+) is a VFF header followed by a ZIP archive, not a
plain ZIP: Python's `zipfile` opens the file directly because it tolerates
data prepended to the archive. Each material has its own folder:

```
materials/<folder>/material.xml       ← colour, base texture, PBR block
materials/<folder>/<base image>       ← e.g. Plywood_01_1K_alb.png
materials/<folder>/<map images>       ← e.g. Plywood_01_1K_rgh.png, _nrm, _ao, _met
materials/<folder>/thumbnail.jpg      ← 2026 writer only; never a map or the base
```

The TLV stream (`model.dat`) still only maps material ids to names, as it
did before 2025. The `.skm` library format uses the same XML schema
(`document.xml`, with the images under `ref/`).

## The `<mat:material>` element

Namespace `http://sketchup.google.com/schemas/sketchup/1.0/material`.

| attribute | values | meaning |
|---|---|---|
| `workflow` | `0`, `1` | **`1` = PBR metallic/roughness** (`SUMaterialWorkflow_PBRMetallicRoughness`), `0` = classic. **The only reliable PBR test**: 2025+ writers give every material a `pbrMR` block, classic ones included. |
| `type` | `0` `1` `2` | colour / textured / colourized (unchanged) |
| `hasTexture` | `0` `1` | whether there is a base texture (unchanged) |
| `pbrPromoState` | `0`, `2` seen | not decoded; no effect on the values. Ignored. |

## The `<mat:pbrMR xScale yScale>` block

`xScale` and `yScale` give the tile in **inches**, the same values as the
base `<mat:texture>`'s. On an untextured material they keep the default (1
inch in SDK-written files). The child elements, in the order the writers
emit them (as text unless noted):

| element | type | range / default | Ruby / C API |
|---|---|---|---|
| `enable_metalness` | `0`/`1` | | `metalness_enabled?` |
| `enable_roughness` | `0`/`1` | | `roughness_enabled?` |
| `roughness_texture_invert` | `0`/`1` | **2026 writer only** (the SDK 2025.0 writer omits it); `1` = the roughness map is a glossiness map | none in the 2025 C API |
| `enable_normal` | `0`/`1` | the SDK refuses to enable it without a normal map | `normal_enabled?` |
| `enable_occlusion` | `0`/`1` | the same, with an AO map | `ao_enabled?` |
| `metallic_texture` | element, attributes `id` `path` `file_name` | present only when set | `metallic_texture` |
| `roughness_texture` | the same | | `roughness_texture` |
| `normal_texture` | the same | | `normal_texture` |
| `occlusion_texture` | the same | | `ao_texture` |
| `metallicFactor` | float32 as text (`0.60000002`) | 0..1, default 1 | `metallic_factor` |
| `roughnessFactor` | float32 | 0..1, default 1 | `roughness_factor` |
| `normalMapStyle` | int | **`0` = OpenGL (+Y), `1` = DirectX (−Y)** (`SUMaterialNormalMapStyle`); new materials default to `1` | `normal_style` |
| `normalScale` | float32 | ≥ 0, default 1 | `normal_scale` |
| `occlusionStrength` | float32 | 0..1, default 1 | `ao_strength` |
| `baseColorFactor` | uint32 | always `4294967295` (`0xFFFFFFFF`, opaque white) in every file seen | none; ignored |

The factors and flags are stored whether or not their channel is enabled.
A disabled channel's value is a leftover default, not a setting.

### Map references

```xml
<mat:roughness_texture id="2" path="./Forrest_Floor_01_1K_rgh.png"
                       file_name="Forrest_Floor_01_1K_rgh.png"/>
```

- `path` is relative to the material's folder: `./name.png` in a `.skp`, or
  `name_<id>.png` under `ref/` in a `.skm`. It is the entry to read.
- `file_name` is the image's name when it was loaded. The base texture's
  `textureFilename` attribute, by contrast, often holds the author's full
  original path (`P:/.skp projects/...`).
- `id` numbers the material's images: 1 is the base image (`<mat:images>`),
  then the maps.
- The images are stored as their original bytes (PNG or JPEG). The C API
  asks for **8-bit single-channel** metallic, roughness and AO maps and a
  24-bit RGB normal map. All 99 such maps in the shipped library are
  Grayscale8. .skp never packs channels: each map is its own image,
  and each material owns its own copies (the API forbids sharing).

## Mapping to `Material.pbr`

The Rust material reader populates the fields defined in `core/materials.py`.

| file | `pbr` key |
|---|---|
| `workflow != 1` | no `pbr` at all |
| `enable_metalness=1` | `metallic` (clamped 0..1); `metallic_map` if a map is set |
| `enable_roughness=1` | `roughness`; `roughness_map` if set. With `roughness_texture_invert=1` the map is written inverted (`<name>_inverted.png`) |
| `enable_normal=1` with a map | `normal_map` (`flip_y: true` when `normalMapStyle=1`), `normal_scale` |
| `enable_occlusion=1` with a map | `ao_map`, `ao_strength` |
| a PBR material with every channel off | no `pbr` (an empty recipe would render with the glTF defaults, i.e. fully metallic) |
| a map whose image is missing from the ZIP | that map and its scalar are dropped |

`orm_map` never comes out of a `.skp`. Map images are written into the same
per-file texture cache folder as the base image (`_texture_dir`). A
different image that collides with an existing name gets the material id as
a prefix.

## Versions covered

| writer | tested by |
|---|---|
| .skp C SDK 2025.0 (`{25.0.633}`) | `tests/data/skp_pbr_2025.skp`, compared with the SDK's own read-back |
| .skp 2026 (`{26.x}`) | the 2026 differences (`roughness_texture_invert`, `thumbnail.jpg`) applied to a copy of the fixture in the tests; two real 2026.2 files checked by hand |
| 2021–2024 | `workflow="0"` throughout, or no block at all: imports unchanged (`skp_arcs_2021.skp`, `skp_instances_2021.skp`) |
| pre-2021 (MFC, not ZIP) | cannot carry PBR; untouched |

## Open questions

- **A factor together with a map.** The importer passes both, and a glTF
  renderer multiplies them. Whether .skp multiplies, or ignores the
  factor once a map is set, is not established: the shipped library always
  pairs maps with factor 1, so it never shows the difference.
- **A map on an untextured material** tiles by `pbrMR`'s `xScale`/`yScale`
  in .skp. The contract has no tile for maps, and the renderer uses a
  1 m planar tile instead.
- **`roughness_texture_invert` = 1** never appears in a real file seen so
  far. Reading it as "glossiness, invert it" follows from the name and from
  how renderers treat glossiness maps; it is not confirmed against
  the modeller's own rendering.
- `pbrPromoState` and `baseColorFactor` are not decoded (they are constant
  or have no visible effect in every file seen).

## Upstream

`docs/openskp-upstream/pbr-materials.patch` carries the same reader for
OpenSKP itself: `_core._extract_pbr`, a `model.PbrMaterial` dataclass,
`Material.pbr`, the fixture and four tests. It was made against upstream
`main` at `c393f3f` (2026-11-19) with `git diff --binary`; apply it with
`git apply`. There, 1039 passed before the patch and 1043 after, with 34
skipped both times. It has **not** been offered upstream. Once an OpenSKP
with `Material.pbr` is installed, `openskp_compat._keep_pbr` steps aside
and the importer reads upstream's field: `tests/test_skp_pbr_import.py`
passes against the patched upstream too.
