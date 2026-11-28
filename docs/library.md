# Library destinations and layout

Everything saved to the Library goes to one of four **destinations**, chosen
in the same Save to Library drawer for every kind:

| Destination | Where | Who sees it |
|---|---|---|
| **This model** | Inside the open file (its plugin data `library`, written through History) | Anyone who opens the file; it travels with it |
| **This project** | `Library/` beside the project's `.builtproject` | Every model of the project |
| **My library** | A folder on this machine for this user (below) | You, in every model |
| **Shared libraries** | Folders added in Settings › Library, each with a name and a read-only flag | Everyone who adds the same folder |

Built's own bundled content (RAL colours, starter components, parametric
profile families, built-in styles) is a fifth *source* in the browser, never a
destination, and is read-only.

The code: `rust/built_render/src/library_store.rs` (the folders, plain Rust,
unit-tested) and `rust/built_render/src/library_host/destinations.rs` (the
preferences, the model's entries, the drawer, the browser rows and verbs).

## Kinds

| Kind | Saved from | Payload (`data`) | Use |
|---|---|---|---|
| `materials` | Library › a model material › Save to Library… | The material's dictionary; its image is carried in `<id>.files/` | Paint with it |
| `components` | Library › Components › a definition › Save to Library… | `{"file": "component.built"}`, the definition saved as a `.built` of its own in `<id>.files/` | Place a copy |
| `profiles` | Sketch workspace › Save to Library… (a profile) | The `built.profile` file | Use in Sweep; Open the drawing |
| `sections` | Sketch workspace › Save to Library… (a section) | The `built.profile` file | Open the drawing (the Cut tool takes presets); listed in Library › Sections |
| `presets` | A live member › Save as preset | The preset row (`uid` is the item's id) | Use in Sweep or Cut |
| `styles` | Display panel › Save style… | The style's dictionary | Use this style (also listed in the Display panel's Style choice) |
| `scenes`, `title_blocks` | Not yet wired to a Save button | — | The layout and destinations accept them |

A drawn sketch is saved as a profile or a section, by its use.

Junction details are presets too. Import pack… (Presets) reads a pack
folder, a `pack.json` or a `.builtpack`. It checks every file's hash and
stores the pack's rows at the destination you pick, every row or none.
Share pack… writes a pack of a detail, or of its whole pack, with the
presets and components it needs. See docs/junctions.md, Library and packs
(J6).

Two kinds keep "This model" in the model's own tables instead of the
`library` entries: a component's This model is its definition kept when
unused (Keep when unused), and a preset's is the model's preset table, so
the Sweep and Cut pickers keep reading them where they always did.

## My library's folder

| Platform | Default |
|---|---|
| macOS | `~/Library/Application Support/Built/Library` |
| Windows | `%APPDATA%\Built\Library` |
| Linux | `$XDG_DATA_HOME/built/library` (`~/.local/share/built/library`) |

With a settings profile (`--settings`, `BUILT_SETTINGS_DIR`, every test) it
is `<profile>/data/Built/Library`, so tests never touch the real folder.
Settings › Library changes it (for example to a synced folder) and adds,
edits or removes shared libraries. The preferences keys:

| Key | Value |
|---|---|
| `library/root` | My library's folder, empty for the default |
| `library/shared` | JSON list of `{"name", "path", "read_only"}` |
| `library/last_destination` | JSON `{kind: destination key}`: the drawer's default next time |

## The folder layout

Every folder destination has the same layout:

```
<root>/
  .migrated-v1                 the one-time migration ran (its log)
  materials/
    9f1c2d3e4b5a6978.json      one item
    9f1c2d3e4b5a6978.png       its thumbnail (optional)
    9f1c2d3e4b5a6978.files/    the files it carries (optional)
      oak.png
  components/
  profiles/
  sections/
  presets/
  styles/
  scenes/
  title_blocks/
```

One item, `<kind>/<id>.json`, UTF-8 JSON written atomically (temporary file
renamed over it):

```json
{
  "format": "built.library",
  "v": 1,
  "kind": "materials",
  "id": "9f1c2d3e4b5a6978",
  "rev": 2,
  "name": "Oak",
  "description": "Oiled, for the stairs",
  "tags": ["Wood", "Interior"],
  "saved": 1795000000,
  "data": {"name": "Oak", "color": [0.6, 0.4, 0.2], "texture": {"path": "oak.png"}}
}
```

| Field | Rule |
|---|---|
| `id` | 1–64 characters of `[0-9a-z_-]`; the file's name. The same item keeps its id in every destination. |
| `rev` | Starts at 1; every save of the same id raises it by one. |
| `data` | The kind's payload. Paths inside it are relative to `<id>.files/`. |

**Identity.** A re-save of the same thing keeps its id: a material by its
name, a style by its name, a component by its definition, a preset by its
`uid`, a profile or section by its name (as the older folder did) or by the
item it was opened from. Copy and Move keep the id and revision, so a copy is
recognised wherever it is.

**Revisions.** When the same id is in several destinations, the browser
offers **Update to the newer revision** on every writable copy older than the
newest one. A Copy onto a newer revision is refused ("The destination has a
newer revision"); a Move replaces it.

**Read-only.** A shared library marked read-only is listed but never offered
as a save target, and its items have no Move, Edit or Delete. A save to it
from a script fails with "<name> is read-only". A folder that cannot be
written is reported when saving.

## Migration

The first time My library is used, the items saved before this layout are
copied in, once per folder (`.migrated-v1` records it):

| From | To |
|---|---|
| `<data>/Built/profiles/*.builtprofile` | `profiles/` or `sections/`, by `use`; id from the file name |
| `<data>/Built/presets/*.builtpreset` | `presets/`; id from `uid`, revision kept |
| Preferences `library/mine` (My library's materials) | `materials/`; id from the name, category as a tag |
| Preferences `styles/user` (saved styles) | `styles/`; id from the name |

Nothing is removed: the old files and preferences stay, and an old item stays
listed until its copy is in My library. From then on the copy is the one the
browser edits and deletes. `sketch_workspace.save_profile` still writes the
older profiles folder for scripts.

## The browser

Every row and tile shows where its item lives (a chip on the tile, a word on
the row). The side list's sources are In this model, Recently used, This
project, My library, each shared library and Bundled; This project and a
shared library appear when they hold something of the open type. The
Components tab has the same sources as Source facets. A saved item's menu and
details offer Use, Edit name and tags…, Copy to…, Move to…, Update, Reveal in
Finder (folder destinations) and Delete… (writable ones).


## Trade add-ons

Build the portable archives with `python3 scripts/build_addons.py`. Each
source folder under `addons/` produces one format-2 archive in
`dist/addons/`. Import pack… in Profiles or Presets verifies the content
hashes and installs the rows together in the chosen destination. Pack
content is CC0-1.0. The website will host these packs later.

| Resource | Where it lives and why |
|---|---|
| Steel profiles · EN and · AISC | Add-ons containing the standard catalogue sizes, provenance and steel sweep presets. Each size carries its family and dimensions into the model. |
| Steel connections · EN and · AISC | Add-ons containing plates, fastener and weld details and steel-specific cuts. The EN pack includes the original M16 group. |
| Purlin connections · EN | Add-on containing the cleat and the bolt detail it uses. |
| MEP penetrations | Add-on containing Duct, Pipe and their wall and slab penetrations. |
| Timber joints | Add-on containing timber section presets and four framing joints. |
| Generic profile families | Bundled editable section geometry including I, C, L, T, Z, rectangle and round and rectangular tube. |
| Generic and Architecture presets | Bundled general fittings, plate, wall, opening, lining, frame, lintel, sill, corner and slab tools. |
| Metric and inch fastener tables | Bundled dimensions for the user-sized bolt primitive and for evaluating embedded fastener details after a pack is removed. |
| Colours, textures, styles, entourage and other starter components | Bundled appearance and scene resources. |
