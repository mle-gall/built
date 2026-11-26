# The Blender link

The Render drawer links a Built model with Blender both ways. Built sends
its geometry and keeps Blender up to date; Blender sends back materials,
lights, cameras and placement transforms, which wait in the drawer until
someone applies or ignores them. Mesh edits never come back. The model
for the split is a bridge: geometry one way, a few kinds of data back.

Code: `rust/built_core/src/blender/link/` (transport, review, document
side), `plugins/_blender_panel.py` (`BlenderLink`, the drawer),
`integrations/blender/built_link/` (the add-on). Tests:
`tests/test_blender_link.py`, `tests/test_blender_addon.py`, Rust unit
tests in the link module, and `tests/test_blender_link_e2e.py` with a real
Blender (`BUILT_BLENDER_TESTS=1`, slow).

## Pairing and transport

- Built listens on `127.0.0.1` only (port 47815, any free port when taken)
  once the drawer's Open in Blender or Pairing code… is used. One Blender
  links at a time.
- The pairing code is `built:<port>:<token>`, a fresh random token per
  Built session. Open in Blender starts Blender with it in `BUILT_LINK`
  and `launch.py`, which links at once; otherwise copy it and use Built ›
  Link to Built in Blender's 3D Viewport sidebar.
- The add-on's first message is `hello` (token, protocol version, the
  `.blend`'s document id, file name, the placements it holds with their
  fingerprints). A wrong token, another protocol or a `.blend` linked to
  another Built model is refused (`refused` with the reason) and the
  socket closed. "Link to this model instead" sends `relink`.
- Frames: an 8-byte prefix (JSON length, payload length; big-endian u32),
  a JSON object, then bytes. Payloads over 1 MiB travel as several frames
  sharing `msg` with `part` / `parts`.
- Blender closing or crashing shows "Waiting for Blender"; the add-on
  retries every two seconds and Built sends only what changed meanwhile.

## Built → Blender

- Each top-level placement is one root object in Blender's "Built"
  collection, exported as GLB in its own coordinates with `built_uid` in
  every node's extras (`gltf_save(..., link=[...])`); its matrix travels
  separately. The loose geometry is the chunk `loose`. Materials from the
  library carry `built_material`.
- Messages: `begin` / `end` around a pass, `chunk` (up to 48 placements
  per GLB), `transform`, `remove`, `material`, `lights`, `cameras`
  (Built's scenes) and `resolved`.
- What to send comes from History's change feed: the link reads each step
  with `History.add_listener` (as the renderer's incremental updates do).
  Named placements narrow the next pass; mesh changes, new or removed
  placements and incomplete steps widen it to a fingerprint pass
  (`LinkTracker`). A fingerprint covers names, paint, meshes and their
  revisions, nested placements and their matrices, never the placement's
  own matrix: a move sends `transform` alone.

## Blender → Built

- Values are named by key: `transform:<uid>`, `material:<name>`,
  `light:<id>`, `camera:<name>`. The add-on keeps Built's last value for
  each key and what Blender read back right after applying it; a change
  is a record that differs from the latter (float noise ignored). It sends
  `changes`: each item with `base` (Built's value it started from) and
  `value` (`base` with the changed fields replaced). Images travel as
  bytes in the payload and Built writes them beside its cache.
- Built compares `base` with its current value: equal is a plain change
  (Apply / Ignore); different is a conflict (Keep Built / Take Blender).
  Values Built already has are answered at once. Items are checked again
  after every Built edit and before applying.
- Applying builds one History step: transforms as Scale, Rotate and Move
  of the placement (a mirrored or locked placement is refused per item),
  materials as `RestampMaterialCommand`, lights in the Render panel's
  lights (point and spot; area and sun stay in Blender), cameras as a new
  scene or the named scene updated.
- Built answers each key with `resolved` (`applied`, `ignored`,
  `kept_built` with Built's value, or `failed` with the reason) so Blender
  neither proposes it again nor loses its own value silently.

## Install

Install Blender add-on… writes `built_link.zip` beside Built's render
cache and shows it in the Finder; `scripts/package_blender_addon.py`
writes the same zip. Blender 4.2 or later: Edit › Preferences › Add-ons ›
Install from Disk…

## Known limits

- A classic group (no matrix of its own) reports an identity transform:
  a move in Blender applies, but Built cannot tell a later Built move of
  the same group from no change, so it is not shown as a conflict.
- Only library materials come back (base colour, roughness, metallic,
  base colour image). Materials made in Blender stay in Blender.
