# Trade add-ons

Trade content (standard steel sections, steel and timber connection
details, duct and pipe penetrations) is not part of the app bundle. Each
folder here is the source of one downloadable pack. The website will
host the built packs later. Until then, build them and install them with
Library › Profiles or Presets › Import pack…

All pack content is **CC0-1.0**.

## Layout

```
addons/<pack-id>/
  pack.json                 manifest, pack format 2 (docs/junctions.md §10, Portable detail packs)
  rows/<uid>.builtpreset    one preset row per file, named by its uid
```

- `<pack-id>` is the folder name and the file name of the built
  `<pack-id>.builtpack`. Use lowercase words and the region or standard
  last: `steel-profiles-en`, `steel-connections-aisc`.
- `pack.json` holds `format: "built.presets"`, `v: 2`, `rows` (the row
  files in display order) and `pack`: a fixed random 32-hex `id`, `name`,
  semantic `version`, `author`, `licence: "CC0-1.0"`, `description`,
  `units`, `requires` (`built` and the features the rows use) and `files`
  (each row file's `sha256:`).
- A row carries its own `uid` (16 hex, stable for its whole life), `rev`
  (raise it when the content changes, so installed copies are offered the
  update), `pack` (the pack's display name), `provenance` (`created_with`,
  and for catalogue data its `source` and `standard`), `depends` and its
  own `hash`.
- A profile size row sets `profile_size: true` and stores the whole
  section in `values.profile`: `library` key (`en/ipe240`), generic
  `family` (`i`, `channel`, `angle`, `rhs`, `chs`, `rectangle`...) with its `params` in mm, and
  `catalogue` (name and series). The family is a bundled parametric
  profile, so a placed member keeps its geometry without the pack.
- A detail that uses another row (a plate, a bolt) ships that row in the
  same pack. Several packs may carry the same row: same uid, same content.

## Commands

```
python3 scripts/build_addons.py --rehash      # after adding or editing rows
python3 scripts/build_addons.py               # all packs into dist/addons/
python3 scripts/build_addons.py --dist DIR steel-profiles-en
```

`--rehash` needs the built_core module, so run it with the repo's venv. It
lists new `rows/*.builtpreset` files in the manifest and updates the row
and file hashes. It rewrites only the files whose content changed. A build
refuses a folder whose files do not match their hashes. Archives are
deterministic: the same sources give the same bytes.

`packaging/` never copies this folder into the app (packaging/native.spec
and packaging/native_check.py refuse it). Tests install packs from here
through `tests/addon_cases.py` into a temporary library; see
tests/test_addons_build.py and tests/test_junction_library.py.
