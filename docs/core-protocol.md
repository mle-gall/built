# The core protocol: commands, queries and a change feed by stable keys

Code: `core/protocol/` (keys, feed, queries, commands), `core/agent_protocol.py`
(bridge tools), `views/wgpu_updates.py` (first consumer); store side
`StoreState.changes_since` / `StoreSnapshot.changes_since` (trunk's), backed
by a chunk-skipping diff (`rust/built_core/src/store/diff.rs`). Tests: `tests/test_core_protocol.py`.

Two tracks meet here. The kernel is moving into the Rust `MeshStore`, where
every op returns a change record by element key. The UI is being rebuilt in
Rust (egui drawn by the wgpu renderer), and its plan's first step is "every
edit and read behind commands, queries and a change feed, all using stable
keys; the MCP bridge uses the same layer". This document is that layer, seen
from the core. It is a public API. Names, arguments and records are plain
data. No Python object and no `id()` ever crosses it.

```
   commands (name + key/value args) ──►  History  ──► ChangeFeed ──► consumers
   queries  (read by key)           ◄──  scene / meshes (store or Python)
```

Consumers today and planned:

| Consumer | Reads | Status |
|---|---|---|
| Metal view's incremental update (`views/wgpu_updates.py`) | which meshes a step changed, and their element records | **built**: geometry steps come from the feed (§6.1) |
| Agent / MCP bridge (`plugins/ai_bridge.py`) | the feed, queries by key | **built**: 7 tools (§5) |
| Outliner / Structure panels (`views/outliner.py`, Structure dock) | placements, tags, materials by key; `complete` | feed subscriber tracks valid scans and dirty placement and mesh keys |
| egui UI (`ui/egui-spike`) | everything above, in process | planned: subscribe to `history.feed`; later the same records from a Rust-side history |

## 1. Commands

A command is a name plus key/value arguments. Elements are named by key
(§2), positions and distances by plain numbers. Each command is **one History
step** (one undo entry). It is answered with the feed record of that step:
`{"ok", "step", "error", "rejected"}`. `step` is the `Step` of §4, or `None`
when nothing happened (refused, rejected, clamped to nothing). A key that
names nothing raises `KeyError` **before anything runs**.

Built (`core.protocol.commands.run(history, name, **args)`):

| Command | Arguments | Runs |
|---|---|---|
| `add_edge` | `a, b: [x,y,z]` (or `a` and `offset`: the end is the stored position at `a` plus `offset` exactly, a typed length), optional `soft`, `tag` key and `eye` (camera position: which way a new vertical face looks) | Line, including splits and face detection; the Line tool's commit |
| `add_edges` | `segments: [[a,b], ...]` | Batch drawing as one step |
| `add_face` | `outer: [[x,y,z]]` (or `anchor` and `offsets`: the stored position at `anchor` plus each offset exactly), optional `holes: [[[x,y,z]]]`, `curve: true` (the ring is one closed curve: tagged so, its plane's faces rebuilt as `core.arrangement.curve_extras` says) | The face with its edges, the plane it lands on rebuilt (#538): the Rectangle's, Rotated Rectangle's, Circle's and Polygon's commit, one step |
| `add_guide_circle` | `center: [x,y,z]`, `normal: [x,y,z]`, `radius` | An analytic guide circle (the Circle tool with Ctrl); nothing for a zero radius, one step |
| `add_arc` | `points: [[x,y,z]]` (an open polyline), optional `center: [x,y,z]` (two radii close the wedge: the Pie), `trim: [[[x,y,z],[x,y,z]]]` (edges to erase after: a fillet's corner stubs) | The arc tagged as one open curve, its plane's faces rebuilt as `core.arrangement.arc_command` says: the Arc tools' and Freehand's commit, one step |
| `move_placements` | `placements: [uid]`, `delta: [x,y,z]` | `MoveGroupCommand` each, one step |
| `move_selection` | `delta: [x,y,z]`; `placements: [uid]`, `vertices`, `edges`, `faces: [key]` (none named: the scene's selection, with its guides, labels, section planes and images); optional `copy`, `count`, `mode: "x" \| "/"` | The Move tool's commit (`core/move_ops.py`): groups as units, loose positions with coincident vertices along and textures carried; a copy stamps the topology verbatim and selects the copies. One step. Dimensions follow their anchors only in the tool |
| `copy_placements` | `placements: [uid]`, `delta`, `count = 1`, `mode: "x" \| "/"` | `count` copies at multiples of `delta` (`x`) or dividing it (`/`), one step; the answer adds `placements`, the copies' uids |
| `divide_edges` | `edges: [edge key], n: integer` | Divide each line or curve into n pieces |
| `join_edges` | `edges: [edge key]` | Join straight segments |
| `follow_me` | `face` key, `path` of local [x,y,z] points, optional `closed` boolean | Sweep the painted profile, consuming its key and creating outward walls and caps. |
| `intersect_faces` | `mode` (context, model, selection, placements), optional `faces` keys or `placements` sibling uids | Partition faces in the open mesh or in each selected placement. Omit the keys to use the scene selection. The feed identifies changed meshes. |
| `move_vertices` | `vertices: [vertex key]`, `delta: [x, y, z]` | `MoveVerticesCommand` (the Move tool's: coincident vertices move along, warped faces fold) |
| `place_group` | `template` placement uid or occurrence path, `basis` (three axes), `offset`, optional `point` and serialized `attachment` | Stamp a copy of the template. The interactive tool can also stamp a detached import. |
| `erase_stroke` | Lists of `edges` (decimal keys), `placements` (uids or paths) and record indices in `guides`, `dimensions`, `geo_paths`, `text_labels`. Optional `hide` | Erase one stroke as one step. Hide applies to edges and placements. |
| `add_geo_path` | `points` (at least two), optional `closed` | Add a geographic path. |
| `move_geo_path_node` | `path` and `node` (zero-based indices), `point` | Reposition a path node. |
| `add_dimension` | `a`, `b`, `offset`, optional `axis` (x, y or z) and two serialized `attachments` | Add an aligned or axis-parallel dimension. |
| `add_text_label` | `anchor`, `offset`, `text`, optional serialized `attachment` | Add a leader and text. |
| `add_image_plane` | `path`, `origin`, edge vectors `u` and `v`, optional `aspect` and `name` | Add a reference image. |
| `place_section_plane` | `point`, `normal`, optional `name` and `symbol` | Place a section plane. Omit the symbol to use the next available letter. |
| `set_group_scale` | `placement` uid or occurrence path, `factors` (three absolute local-axis scales) | Apply Scale Manager values. |
| `set_north` | `degrees` | Set model north and wrap the angle to one turn. |
| `solid_operation` | `operation` (outer_shell, union, subtract, trim, intersect, split), `placements` (sibling uids in pick order) | Runs the Solid Tools command. The step names consumed and result placements. |
| `rebuild_cap` | `face: seed face key` | Rebuild its plane, including debris pruning. Undo and redo restore the original and replacement keys. |
| `reverse_faces` | `faces: [face key]` | Reverse outer and hole rings in input order. Keys and paint survive. |
| `orient_faces` | `face: seed face key` | Flip inconsistent neighbours across two-face edges. All keys survive. |
| `push_pull` | `face: face key`, `distance: number`, `keep_base: bool = false` | the Push/Pull tool locked on the face, the distance typed, committed (clamped as the tool clamps) |
| `erase` | `edges: [edge key]`, `faces: [face key]` | `EraseSelectionCommand` (the Eraser: a split line between coplanar faces dissolves) |
| `explode_group` | `placement: uid` | `ExplodeGroupCommand`: the placement's geometry merges into the open mesh through its matrix (corners weld onto what is there, coplanar overlaps rebuilt into one face), its nested placements come out whole. The step's `copies` say where each source vertex, edge and face went |
| `split_into_pieces` | `placement: uid` | `SplitIntoPiecesCommand` over `core.pieces.split_into_pieces`: one child group per physical piece. `ok` false and no step when it already is its pieces |
| `explode_view` | `placement: uid`, `factor: number`, `mode: "outward" \| "x" \| "y" \| "z" = "outward"` | `ExplodeViewCommand` (`core/explode.py`): the parts pulled apart, `factor` 0 reassembles |
| `purge_unused` | `layers: bool = true`, `materials: bool = true` | `PurgeUnusedCommand`: tags and materials nothing uses go (a tag a saved view hides stays). `ok` false and no step when nothing would go |
| `purge_debris` | — | `Mesh.purge_debris` on the open mesh as one snapshot step: vertices no edge and no face uses |
| `undo`, `redo` | — | `History.undo` / `redo` |
| `hide` / `unhide` | `placements: [uid]` (nested included) | `HideCommand` |
| `lock` | `placements: [uid]`, `locked: bool = true` | Native step `LockCommand` (`NativeCommand.lock`); locking drops them from the selection |
| `rename_placement` | `placement: uid`, `name: string` | Native step `RenameGroupCommand` (`NativeCommand.rename`) |
| `make_unique` | `placement: uid`, `rename: bool = false` | `MakeUniqueCommand`: the copy gets its own definition (and, with `rename`, a name of its own) |
| `rename_placements` | `placements: [uid]`, `names: [string]` (one per distinct placement) | Native step `RenameGroupCommand` (`NativeCommand.rename_each`), one step; placements already named so are left out |
| `paint_placements` | `placements: [uid]`, `paint` (a paint dict, or null to unpaint), optional `material` (in process: a `Material` the paint names) | Native step `SetGroupMaterialCommand` (`NativeCommand.paint`): the placements are painted whole, faces painted themselves keep theirs. A material not in the registry joins it in the same step |
| `set_tag` | `placements: [uid]`, `tag: tag key` | `AssignLayerCommand` |
| `edit_tag` | `tags: [tag key]`, `field`, `value` | Apply a node’s Drawing style. Fields are `color` (#RRGGBB or empty), `weight` (mm or auto), `linetype`, `hatch` (pattern key or empty) and `print` (0 or 1). The document validates the batch before one undoable step. |
| `edit_structure_node` | `node: key`, `field`, `value` | Rename (`name`), move to an editable node or the root (`parent`), or set the working node (`active`). Rename and move derive the descendants’ filing from the updated registry in the same history step. |
| `set_tag_state` | `tags: [tag key]`, `folders: [tag folder key]`, `visible: bool?`, `locked: bool?` (at least one; folders have no lock) | `SetTagStateCommand` (`core/protocol/commands.py`): the flags, one step. A linked file's tag (`"A / Rails"`, nested `"A / C / Flights"`) also records the host's override in `plugin_data["links"][path]["overrides"]`, so the source's later switch of that tag no longer reaches it (`core/links.py`); undo restores flags and overrides. The Project tree's eye and lock use it, on the host's nodes and on a linked file's (its row: the file's tag folder, `"A"`) |
| `resegment_curve` | `edge: edge key`, `segments: integer` | `ResegmentCurveCommand` (`core/curve_segments.py`): the regular circle, polygon or arc rebuilt in place with that many pieces; a curve that is not flat is `rejected` |
| `repair_solid` | `placement: uid` | the Solid Inspector's `RepairSolidsCommand` on the placement's mesh (shared by its copies); the answer adds `repaired` counts |
| `rename_material` | `name`, `new_name` | `RestampMaterialCommand` under the new name: every face (front and back) and container wearing it follows. A name another material has raises `ValueError` |
| `edit_material` | `name`, optional `color: [r,g,b]` 0–1, `opacity` 0–1 (1 = opaque), `tile: [w, h]` metres | plain: `RestampMaterialCommand`; textured colour/tile: `EditTexturedMaterialCommand` (each face keeps its own texture placement). `tile` on a plain material raises `ValueError` |
| `duplicate_material` | `name`, optional `new_name` | `AddMaterialCommand` (`core/material_commands.py`): a copy worn by nothing; default name `"<name> copy"` (numbered when taken); the answer adds `name` |
| `delete_material` | `name` | `DeleteMaterialCommand`: the registry entry goes and what wore it shows the default material; undo restores faces and containers exactly |
| `delete_placements` | `placements: [uid]` (nested included) | one `DeleteGroupCommand` each, as one `CompoundCommand` step |
| `rename_component` | `placement: uid` of any copy or of the kept template, `name` | One native `RenameGroupCommand` step over every copy and the kept template |
| `create_material` | one base: `color: [r,g,b]` or `texture: {path, sw, sh}`; optional `name`, `opacity`, `pbr`, `finish`, `reuse = true` | `AddMaterialCommand`. With `reuse` and no `pbr`, a material with the same plain recipe is answered (`reused: true`, no step). Unnamed: `Colour #RRGGBB` or the image's name. The answer adds `name` |
| `set_material_base` | `name`, `base: "color" \| "image"`, optional `color` or `texture` | `RestampMaterialCommand`: the base switches, PBR values, opacity and finish are kept; what wears it follows. An image base with no image raises `ValueError` |
| `set_material_pbr` | `name`, any of `metallic`, `roughness`, `ao_strength` (0–1), `normal_scale` (≥ 0), `metallic_map`, `roughness_map`, `normal_map`, `ao_map` (path or `{path, flip_y?}`); `null` removes | `RestampMaterialCommand`; the same channels for a colour or an image base. No channel left: no `pbr` (contract: `docs/skp-pbr-format.md`) |
| `name_unnamed_colours` | — | `NameColoursCommand`: "Name these". Each distinct colour painted without a material (older files: front, back, containers) becomes a material, reusing one with the same recipe. The answer adds `named`. Files are never rewritten on open |

The material and component commands (`rename_material` to
`name_unnamed_colours`) were added for the Library (`views/wgpu_library.py`);
a material is named by its registry name (§2). Every paint is a material:
a colour or an image is its *base*, and PBR values, opacity and finish sit on
top of either. The Paint tool never stamps an anonymous colour any more: a raw
colour reuses a material with the same recipe or creates one in the same
undo step (`core.material_commands.material_for_paint`). The placement commands above (`hide` to `repair_solid`, added for the egui
UI) take placements at any depth of the open context, as the project tree
names them; the older ones take top-level placements.

All act on the open mesh (`scene.mesh`, the loose model or the group being
edited); a `placement` is a top-level placement of the open context
(`scene.groups`), unknown uids raise `KeyError` before anything runs. The bridge does not expose them yet. Agents edit through
`run_python`, and the feed shows what that did.

Planned, each over the existing History command it names (the argument
lists are the contract to build to):

| Command | Arguments | History command |
|---|---|---|
| `paint_faces` | `faces: [face key]`, `material: name \| null`, `side: "front" \| "back"` | `SetFaceColorCommand` family |
| `soften_edges` | `edges: [edge key]`, `soft: bool` | `SoftenByAngleCommand` |
| `open_placement` / `close_placement` | `placement: uid` | `begin_group_edit` / `end_group_edit` (not History steps today) |

## 2. Stable keys

| What | Key | Lifetime |
|---|---|---|
| Placement (group, component instance, nested part) | `Group.uid` (a string) | saved with the document; a copy gets its own; linked parts derive theirs from the source |
| Tag | `Layer.key` (names are labels) | saved |
| Material | its name in `scene.materials` | saved; a rename is a remove plus a create until materials get keys as tags did |
| Mesh | `"m<n>"`, given the first time the protocol names a mesh (`core.protocol.keys.mesh_key`) | this session. The loose model and each geometry definition have a key. An active native occurrence view uses its definition’s key. |
| Element | `(mesh key, element key)`. The element key is the int every `Vertex` / `Edge` / `Face` already carries | this session; unique across meshes in the process; kept across undo / redo, remove / relink and the store's restores; a deep copy is new elements with new keys |

Element keys of a store-backed mesh encode `namespace << 40 | kind << 38 |
generation << 26 | slot` (`rust/built_core/src/store/`). Their kind is in
the key. Python-mesh keys are a plain counter. **In JSON an element key is a
decimal string**: store keys pass 2^53, and a JavaScript client (MCP hosts
are) would round them. Python callers use ints. `from_json_key` takes both.

Mesh references accepted by queries (`resolve_mesh`): omitted or `"active"`
(the open mesh), `"loose"`, a mesh key, a placement UID, or an occurrence UID path
(the definition drawn by that placement).

Never persisted: mesh keys and element keys (persisting element keys is
`.igz` format 4, out of scope). A consumer that outlives the session keys
by placement uid and positions.

## 3. Queries

Built (`core.protocol.queries`). Every answer is JSON-safe. Nothing reads by
building Python objects for a store-backed mesh's untouched elements, takes
an undo snapshot or materialises a store.

| Query | Arguments | Answer |
|---|---|---|
| `face_set_volume` | `keys, mesh?` | `{mesh, missing, volume}` for the selected face rings |
| `solid_report` | `mesh?` | `{mesh, solid, volume}`, local cubic metres or null |
| `triangles` | `keys, mesh?` | Face keys, oriented triangle coordinates and missing keys |
| `mesh_info` | `mesh?` | `{mesh, backing: "store" \| "python", vertices, edges, faces, active, revision}` |
| `faces` | `keys`, `mesh?` | per face: `rings` (positions, outer first, then holes), `corners` (vertex keys, same shape), `normal`; `missing` keys |
| `edges` | `keys`, `mesh?` | per edge: `ends` (vertex keys), `positions`, `soft`, `hidden`, `faces` (keys, with multiplicity); `missing` |
| `vertices` | `keys`, `mesh?` | per vertex: `position`; `missing` |
| `selection` | — | `{mesh, placements: [uid], vertex, edge, face: [key]}` (elements of the open mesh) |
| `select` | `faces?, edges?, vertices?: [key]`, `mode: replace \| add \| remove \| clear` | `selection` plus `missing`. Not a History step: selection is view state |
| `feed` | `since_revision` | §4 as JSON |

Planned: `faces_around(vertices)` (the store's, for the renderer's partial
rebake), `faces_on_plane(point, normal)` and `face_at(ray)` (an agent naming
a face it can only describe), `placements(uids)` (rows: name, tag, material,
matrix, child uids), `tags()`, `materials()`.

## 4. The change feed

`History` makes a `ChangeFeed` in its constructor (`history.feed`). The feed
listens to every execute, undo and redo through `History.add_listener`.
After each step that ran it publishes one `Step`. A step that failed and was
rolled back, or that a command rejected, publishes nothing (History's
`revision` does not move either).

### 4.1 A step

```
Step
  revision     History.revision after the step (the token query_changes uses)
  phase        "execute" | "undo" | "redo"
  command      the command's class name
  meshes       [MeshChange]   one per mesh the step changed
  placements   {created, removed, modified: [uid]}
  tags         {created, removed, modified: [tag key]}
  materials    {created, removed, modified: [material name]}
  complete     False when the step may have changed placements, or face
               attrs in other meshes, that are not named here: re-read
               what you follow
  copies       [{source, target, keys}]: elements the step copied from
               one mesh into another (Explode, Split into Pieces), the two
               mesh keys and per kind (source key, key in target) pairs;
               empty on undo (the copies are gone: `removed` names them)

MeshChange
  mesh         mesh key            (.mesh in process: a weak reference)
  backing      "store" | "python"
  replaced     re-read the whole mesh (see below)
  created      {vertex, edge, face: [key]}
  removed      {vertex, edge, face: [key]}
  modified     {vertex, edge, face: [key]}
  moved        [vertex key]
  revision     the store's revision after the step (None for Python)
```

**Store-backed mesh: the store's change record.** Before the step the feed
takes the mesh's `capture_state()` (the store snapshot, O(1), plus the
faces' own attrs dicts). After the step it asks
`capture_state().changes_since(before)`, the net change between the two
states (§22 of the store design). That is the same `StoreChange` record the ops
return, and it covers whatever the step did: ops, field writes, a restore,
several of those in a row.
- `created` / `removed`: keys live after and not before, or before and not
  after (a relinked tombstone counts as created, as the ops report it).
- `moved`: vertices whose position changed.
- `modified.vertex`: vertices whose edge set changed.
- `modified.edge`: ends, soft / hidden, curve, tag, or the face list
  (order and multiplicity).
- `modified.face`: rings, attribute index, interior flag, or the face's own
  attrs dict (attrs live beside the store, design decision 6;
  `StoreState.changes_since` compares them). Past `ATTRS_COPY_MAX` (50,000)
  faces with dicts of their own, the feed stops copying them before each
  step; then only the faces a paint command names are reported.

What follows from a move (the edges and faces around it) is not listed. A
renderer derives it with the store's `faces_around(moved)`.

**Python mesh: coarse.** `replaced` is true when its geometry serial moved
(re-read it). A paint that names its faces is reported as `modified.face`
without `replaced`. Any other attrs-only change sets `replaced`. A mesh that
went from the store to Python objects, or back, during the step
(materialisation, an undo onto a store snapshot) is `replaced` as well.

**Net, and undo is the inverse.** A step's record is a diff, not a log. An
element made and removed inside one step is in neither list. Undo is a diff
too, so it publishes the inverse of the step it takes back: `created` and
`removed` swap, `moved` and `modified` stay the same keys. Redo publishes the
step again. Tested exactly against an oracle read through the mesh, for
move, push/pull and erase with undo and redo, and for random op sequences
(`tests/test_core_protocol.py`).

**Which meshes.** The open mesh at execute; at undo / redo the mesh the
command ran on (`_history_mesh`, even after the user left that group); a
`SnapshotMutation`'s own target; a soften's meshes; a compound's parts';
the shared definition a Reshare step writes (leaving an edited component
instance or assembly). A mesh with nothing changed is left out.

**Namespace changes.** A store mesh that adopted another store's snapshot
during the step (the definition after `share_back`) has new keys: its
record names every old key removed and every new key created, not
`replaced`.

**Document changes.** The journal identifies changes to placements, tags
and materials. Commands can supplement it with affected placement UIDs,
including the placements sharing an edited definition.
Geometry-only commands (`keeps_rows`, and the snapshot / move / draw / erase
commands) change no placement. Explode names the group removed and its
lifted children created (undo: the reverse); Split into Pieces the group
modified and its pieces created; the exploded view the parts it moved;
Purge Unused no placement (its tags and materials are compared by value);
leaving an edited component instance (`ReshareInstanceCommand`) every
placement that shows its definition, nested and hidden ones included, on
execute, undo and redo.
Any other command sets `complete = False`.

Every step runs inside a Rust document transaction. The journal supplies the
document portion of each feed record. Root placements created / removed are the net edits of the scene root's list
(same identity rule, same `modified` for a uid both removed and created);
`modified` adds every placement whose fields or child list the step wrote
(so it can name more than the command knowledge above, never fewer); tags
created / removed / modified come from the scene's tag list and the tags'
fields (sheet styles included since slice 2); materials created / removed
from the registry, which is a document list since slice 2 (`Registry`: a
name given another `Material` is modified, as the identity comparison
said), modified from the fields written of a registered material. Records
(`Group.ifc`), extension data (`Group.ext`), 3D text and exploded views
are placement values too: a step that writes one names that placement as
`modified` (a record is no longer outside the feed's view). The scene's
own tables (tag folders, the active tag, `plugin_data`) are document
values as well but, as before, not part of the record (the journal has
them: `Transaction.changes()["scene"]`). For the commands that
only write placement fields (Lock, Hide and Tag of placements, Rename,
Move of an instance) the command knowledge above is not read again: the
journal already names every placement they wrote. Nothing else in the
record changes: same keys, same `complete`, same `copies`. Undo by
transaction publishes the inverse of the same journal. Cost: a few
microseconds instead of the 2 × 1.2 ms of §4.3; Lock of 20,000 placements
builds its record in ~1 ms.

**Copies.** Explode on store-backed meshes reports `copies` = one entry,
`source` the group's mesh (read, never changed: it is not in `meshes`),
`target` the open mesh, and keys for every kind: each source face with the
face made from it, each source vertex and edge with the one now at its
mapped place (a corner that welded onto a vertex the mesh had names that
vertex; faces a coplanar merge rebuilt are left out, `created` has their
successors). Redo reports the same keys (the store puts back the state the
first run made). Split into Pieces reports one entry per piece the store
cut, `face` pairs only, when the group had no nested placements (with
them, the source is a merged world copy no one can name). Python-mesh
runs report no copies.

On the native document, an in-process step also exposes `scene_fields`
for changed root tables and `records` for field edits to existing
annotation, saved-view and sheet records. The latter maps `annotations`,
`views` and `sheets` to session-local document keys for the families
affected. The JSON representation remains the shape in §4.1.

### 4.2 Delivery

- `history.feed.subscribe(fn)` → `fn(step)` after every step, synchronously
  on the UI thread, before every `History` listener registered after the
  feed (all others: History subscribes it first). A subscriber that raises
  is logged and never breaks the command.
- `history.feed.step(revision)`, `.last`, `.since(revision)` →
  `{revision, complete, steps}`; `complete` is false when steps older than
  `revision` fell out of the ring (256 steps, like History's journal).
- A command run inside another command's `do` is folded into the outer step.
- JSON: `Step.to_json()` (keys as strings).

### 4.3 Cost (this Mac, 2026-11-22, synthetic meshes)

| | |
|---|---|
| `changes_since` after one vertex move, 300 × 300 grid (90,000 faces) | 11–15 µs (chunks both states share are not read); trunk's element-by-element walk it replaces: 10.2 ms |
| Move + undo through the feed vs without it, same grid, store-backed | 48.1 vs 48.3 ms (45.4 vs 44.8 in an earlier run) |
| Same, Python mesh | 1,225 vs 1,192 ms (noise: the feed reads two counters) |
| Document comparison per step, 20,000 root placements, 300 tags, 300 materials | 2 × 1.2 ms |

## 5. The agent / MCP bridge

The bridge's tools for the protocol (`core/agent_protocol.py`, listed by
`list_tools`, all read-only):

| Tool | Arguments | Query |
|---|---|---|
| `query_feed` | `since_revision` | `feed` |
| `query_mesh` | `mesh?` | `mesh_info` |
| `query_faces` / `query_edges` / `query_vertices` | `keys`, `mesh?` | `faces` / `edges` / `vertices` |
| `query_selected_elements` | — | `selection` |
| `select_elements` | `faces?, edges?, vertices?`, `mode` | `select` |

So an agent can name a face by key: select it in the app (or read it from
`query_selected_elements`, or from the `created.face` of the step that made
it), read its rings with `query_faces`, act on it, and read back what the
step did with `query_feed`. `query_changes` (the command journal) stays as
it was; its revisions are the feed's.

## 6. Consumers

### 6.1 Metal view: incremental update (built)

`InstanceUpdates.observe(history)` already followed History's listeners and
decided from the command class which mesh a geometry command had edited:
the open mesh, or a snapshot mutation's target. It now takes those meshes
from the feed's step instead. It also keeps each pending mesh's
`MeshChange`s until the next `diff()`, and hands the records of the meshes
it rebakes to the caller as `changed_records` (`id(mesh) → [MeshChange]`).
A partial rebake (only the faces around `moved` plus `created` /
`modified`, `removed` tombstoned) would start from those records; that
choice belongs to the renderer track. Rebakes are still whole prototypes.
Without a feed (a stand-in History) it falls back to the command class.

One behaviour changed: **edit inside a group, leave the group, undo.** The
command class named the open mesh, which by then was the loose model, and
the resident pack kept drawing the undone edit (it no longer matched a fresh
pack). The feed names the mesh the undo ran on, which gets rebaked
(`test_renderer_undo_after_leaving_the_group_rebakes_it`). Every
`tests/test_wgpu_updates.py` test passes unchanged.

### 6.2 Browser, Materials, Structure and BIM

A synchronous feed subscriber maintains the scan marks used by these panels.
The Browser can refresh renamed rows by placement UID. Materials evicts the
cached swatches for repainted store meshes by mesh key. Incomplete steps and
revision gaps invalidate the scans. Content versions and the native write
serial also detect changes between published steps. Group entry and exit
carry forward only the scans that can reuse their rows or mesh data.

### 6.3 egui UI (planned)

In process, the egui layer subscribes to `history.feed`, issues §1 commands
and §3 queries, and never holds a Python element. Rust History publishes the same
`Step` and `MeshChange` shapes using the store records from `edit::Change`.

## 7. Limits of this first version

- Python meshes report `replaced` (coarse) until their ops are ported.
- On a Python mesh, paint is named by its faces only for the `SetFace*`
  commands (on a store mesh any attrs edit is, up to `ATTRS_COPY_MAX`). A
  material replaced across every mesh (`complete = False`) is not.
- A step's meshes are the ones known before it ran. A soften's meshes at
  execute are found only after, and are not reported (the renderer handles
  soften itself).
- Mesh keys are per session; the feed is not saved.
- Commands are not exposed through the bridge yet (agents edit with
  `run_python`); the planned command list is §1.
- `copies` of a split name faces only; vertices and edges of the piece are
  new keys the feed lists as the piece mesh is first named.

### Document deliveries

A delivery stores an export recipe in the document. The additive JSON object
`plugin_data.deliveries` maps stable keys to records. IFC deliveries can refer
to a shared BIM registry `export_profiles` key. Their destination and last
successful run belong to the individual document.

| Command | Arguments | Result |
|---|---|---|
| `create_delivery` | `delivery` record | New key and History step |
| `edit_delivery` | `key`, `changes` object | Merged record as an undoable edit |
| `delete_delivery` | `key` | Undoable removal |
| `run_delivery` | `key` | Job ID in the active Metal window, or the completion step in a synchronous host |

Example input for `create_delivery`:

```json
{"delivery":{"name":"Client viewer","format":"glb","scope":"visible","destination":"client.glb","options":{"textures":true}}}
```

Formats are `ifc`, `dxf`, `glb`, `obj`, `dae`, `svg`, `stl`, and `pdf`.
Model scopes are `whole`, `visible`, `selection`, and `node`. A node scope
also takes the Project tree key in `node`. A run resolves the current
membership. DXF and SVG also accept `scenes`. PDF uses `sheets`.
The optional `options.names` array selects named scenes or sheets in document
order. Multiple scenes produce numbered sibling files such as `plan-01.dxf`.
Sheets share a multipage PDF.

IFC accepts `options.profile`. GLB embeds textures by default and accepts
`textures: false`. STL accepts `unit`. SVG accepts `scale`, `paper` as a pair in mm or `"screen"`, a
`frame` triple of horizontal pan, vertical pan and zoom, and `faces` with boolean `materials`, `textures`, `as_view`, and
`shadows`. Omit `faces` for linework alone. A visible SVG delivery can set `picture` and
`axes` to embed the viewport capture.

Relative destinations resolve against the document directory. Unsaved models
require an absolute destination. The synchronous Python entry point accepts
host context through `project_directory`, `camera`, `screen_mm`, the viewport's
`svg_picture(axes, frame)`, and the composer's `pdf_writer(path, sheets)`. Successful runs stamp `last_run` in UTC.
Undoing that metadata change leaves exported files on disk.

### IFC inspector commands

| Command | Arguments | Result |
| --- | --- | --- |
| `patch_record_field` | `placements` (UIDs), `path`, `value`, optional `schema` (default IFC4) and `property_kind` | Validates the effective record of every target, then applies one undoable step. A scalar path such as `["description"]` edits an occurrence field. `["psets", set, property]` edits one typed value, preserving siblings and GUIDs. A null property value restores its filing or type default. Creating a property requires its kind. `["classifications", system, code]` edits the occurrence's reference name. Linked targets and filing-owned identity fields reject the patch. |
| `set_filing_rule` | `node`, `recipe`, `expected` (previous local recipe) | Checks the expected rule and registry, then writes the local overrides and resynchronises members, including descendants, in one history step. |

The narrow query `core.protocol.records.occurrence` returns field paths, typed values, ownership and editability. The native inspector groups these by path and previews the explicit UID list for a mixed selection. `core.protocol.records.filing_rule` without a history previews the affected UID count and fields.

Nested placement addresses for hide, unhide, lock, rename and tag assignment
can be supplied as arrays of UIDs from the scene root to the target. A string
keeps the existing UID lookup. Record commands' `placements` may also
contain UID paths. A mesh reference may be a UID path to its definition.
In a native affine edit context, `active` resolves the store in tool
coordinates and the definition mesh key resolves it in definition coordinates.
Change records keep the persisted placement UID and existing fields.

A link subset’s `select.uids` accepts source UIDs and root-to-child UID paths.
The subset retains the selected occurrence’s fields and world transform.
