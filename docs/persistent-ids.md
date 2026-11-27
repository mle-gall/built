# Persistent ids

Status: **approved 2026-11-27** with the §9 recommendations as written;
**implemented** (Phase 2). §10 lists where the implementation differs from
the text below and what is left for later.

Today, only placements (`Group.uid`), tags, tag folders, sheets,
deliveries and BCF topics keep an identity across sessions. Vertices,
edges and faces have session keys, and so do definitions (`m<n>`, the
arena's `Definition.key`). Materials and scenes go by name. Section
planes use a small integer counter. Annotations go by list index.
[core-protocol.md](core-protocol.md) §2 says it plainly: "a consumer that
outlives the session keys by placement uid and positions". The
annotation anchors do exactly that (`annotation_anchor::archive` stores
the placement path, the vertex positions and their weights).

This design gives every element that something outside the session might
name a persistent id. It does so before the first release, so `.built`
format 2 can stay the format for a long time. The consumers that need it:

- live collaboration: an operation log over the change feed
- a web client
- cloud sync
- BCF topics and agent tasks that name a face or a part
- the Blender link
- IFC GlobalIds that stay the same across exports and sessions

## 1. Decisions at a glance

| | Decision |
|---|---|
| Two id families | **Record uids** for document rows (placements, definitions, materials, scenes, …): 64 random bits, 16 lowercase hex, the form placements already use. **Element ids** for vertices, edges and faces: 128 logical bits, *origin* (64) ‖ *counter* (64), stored packed in 64 bits. |
| Element id scope | Unique per definition and per kind, never reused. A full reference is (definition uid, element id). An occurrence reference is (placement uid path, element id). |
| Minting | No coordination. Each editing session draws a random 64-bit origin and counts from 0. Origin 0 is reserved for ids derived on migration. |
| Storage | One id column per kind in the store `State`, so snapshots, undo and redo carry ids for free. The column is **implicit** (`first + slot`) until an edit breaks the sequence. The file holds an implicit `first`, or a delta-coded `<u8` array. |
| Continuity | The id follows the kernel's key: same key, same id. When an op replaces an element (remove and create in one step), one successor inherits the id by a fixed rule (§4). |
| Format | `built_format: 2`. It adds a `doc` header (document id, origin table) and, for each geometry block, a definition uid and id arrays. Format 1 and `.igz` files get deterministic ids on load. A stability policy goes into built-format.md (§5.6). |
| Cost at ride scale | 0 bytes and 0 ms for untouched meshes. About 0.17 B per element compressed and 8 B per element in memory for meshes whose sequence an edit broke. Measurements in §3. |

## 2. What gets an id

### 2.1 Geometry elements: vertices, edges, faces

**All three kinds get element ids, vertices included.**

- *Why vertices.* An operation log has to name them: `move_vertices`,
  `add_edge` welding onto an existing vertex, and dimensions or labels
  that follow a corner. Today's position-plus-weights workaround breaks
  as soon as the geometry moves. Weld semantics need them too: the
  survivor keeps its id. With the implicit column (§3) a vertex id costs
  nothing until edits break the sequence, so leaving vertices out would
  save almost nothing and leave the op log incomplete.
- Ring corners are not elements. A corner is (face id, vertex id).
- **Curves** keep their integer column (`edge_curve`, an `i128` in the
  store). New curves take their number from the element-id space (packed
  origin index and counter, §3), so concurrent clients never collide.
  Existing integers are read as origin 0, so their values do not change.

### 2.2 Document rows (record uids)

| Row | Today | Decision |
|---|---|---|
| Placement (group, component instance, nested part, link wrapper) | `uid`: 16 hex, random. Linked children are derived (UUID5 of the link placement uid and the source group uid) | **Unchanged.** |
| Definition (prototype, kept component, loose geometry) | Proto index in the file, session `Definition.key` | **New `uid`**, saved in the geometry block meta. The document's loose geometry uses the reserved uid `0000000000000000`. |
| Material | Its name. A rename is a remove plus a create in the feed | **New `uid`**, kept on rename and on edits, new on duplicate. Faces keep referring to materials by name in format 2; see open question Q2. |
| Tag, tag folder | `key` (the name at creation) and `uid` | **Unchanged.** |
| Section plane | `uid`: an integer counter (1, 2, …) | **Becomes a 16-hex string.** A legacy integer *n* reads as `format!("{n:016x}")`, and references such as a saved view's `section.active` are mapped the same way. New planes get random uids. |
| Scene (saved view) | Its name. Sheet frames use `view_key: "scene:<name>"` | **New `uid`.** Frames use `view_key: "scene#<uid>"`, and a legacy `scene:<name>` is resolved by name on load. |
| Sheet | `uid`: 32 hex | **Unchanged.** Every opaque uid already in the format stays as written. |
| Sheet items (frames, texts, shapes, dimensions, labels, levels, title block) | Frames have `uid`; other items have a `uid` field that is often empty | **Each item gets a `uid` at creation.** Empty legacy uids get a deterministic one on load (§5.3). |
| Annotations (guides, dimensions, text labels, geo paths, image planes) | List index (`erase_stroke` names them by index) | **New `uid`.** Commands accept a uid or an index. |
| Task / BCF topic, comment, viewpoint | `guid` (UUID, as BCF requires) | **Unchanged.** Components gain `ifc_guid` and an element-level `authoring_tool_id` (§8.2). |
| Delivery | 32-hex key | **Unchanged.** |
| Record (`Group.ifc`) | Belongs to its placement. `guid` is the stored IFC GlobalId | The placement uid identifies the record. A stored `guid` remains an explicit GlobalId override (§8.1). |
| Document | None (the Blender link keeps its own `link_id` in document data) | **New `doc.id`**: 128 bits, 32 hex, like `.builtproject` ids. |

### 2.3 What does not get an id

These keep their current identity or have none:

- weld cells
- face-attribute table rows (an interned value, not an identity)
- triangles and baked or resident buffers
- session mesh keys (`m<n>`) and session element keys (both stay as the
  fast in-process handles)
- History revisions (the op log numbers its own entries)
- the selection and the camera
- texture blobs (already content-addressed: `textures/<hash>-<name>`)
- `geom/` member names (local to the file)
- hatch patterns, styles and linetypes (named library entries)
- undo records

## 3. Id scheme and cost

### 3.1 Logical id

```
element id  = origin : counter        logical 128 bits
  origin    = 64 random bits, drawn once per editing session that mints;
              0 = derived on migration (§5.3)
  counter   = per origin, from 0, never rewound
text form   = "<kind>:<origin hex>:<counter hex>"   lowercase, no leading zeros
              e.g. "f:9c1e4a7b2d3f6e10:4a2", a migrated face "f:0:4a2"
reference   = {"def": "<16 hex>", "id": "f:…"}               (a definition's element)
            | {"path": ["<uid>", …], "id": "f:…"}            (an occurrence's element)
```

Ids are created without coordination. Two clients editing the same
document offline get different origins, so their ids never collide. With
64 random bits the collision odds are about *n*²/2⁶⁵ for *n* sessions:
10⁻⁸ at 10⁶ sessions. The counter needs no lock across processes, only
within one, where it is an atomic.

### 3.2 Packed storage

The document keeps an **origin table**: index 0 is the migration origin,
followed by every origin whose ids the document holds. Columns store
`index << 40 | counter`. That allows 2²⁴ origins per document and 2⁴⁰
ids per session.

The table belongs to the document: each open document, and each project
tab, has its own. Indices are local to that table, and that is the only
reason they are not exposed. The protocol, the op log, BCF and the file's
row references always use the logical text form. A session that mints
nothing adds no origin, and an origin enters the saved table only when
the save contains at least one of its ids.

### 3.3 In memory

Each `State` gets an `ids: [IdColumn; 3]`, one per kind:

```
enum IdColumn {
    Implicit { first: u64 },   // id(slot) = first + slot. No memory.
    Dense(CowVec<u64>),        // 4,096-element Arc chunks, like the other columns
}
```

- The column lives inside `State`. A snapshot clone shares its chunks,
  undo and redo restore it exactly, and the diff compares it chunk by
  chunk.
- The **minter** (session origin index and counter) lives outside
  snapshots, next to `Hwm`, so a restore can never rewind it.
  `Slots::alloc` is the one place new elements are born. It writes the
  minted id there. `revive` (relink, redo) keeps the tombstone's id,
  because tombstones keep their column data (`Slots::kill`).
- `Implicit` turns `Dense` the first time an allocation breaks
  `first + slot`. A fresh import mints its ids in slot order, so it stays
  implicit until it is edited.
- **id → slot**: `Implicit` is arithmetic. `Dense` builds a hash index
  the first time something resolves an id in that mesh (16 B per element,
  that mesh only, dropped when the mesh changes).
- **Deferred meshes** (never turned into a store) keep the block's
  implicit `first`, or the delta array undecoded. The pack and the
  renderer never read ids.

### 3.4 In the file

Each kind is either implicit (`"ids": {"vertex": "<first, hex>"}` in the
block meta) or an array `vertex_id` / `edge_id` / `face_id` with dtype
`<u8`. The array holds **wrapping differences** from the previous id; the
first value is relative to 0. The writer picks the implicit form when the
sequence is exact.

### 3.5 Measured (this Mac, NumPy, zlib level 1, the writer's level)

| 4 M elements | Raw | Compressed |
|---|---|---|
| Implicit (untouched or migrated mesh) | 0 | **0** (≈40 B of JSON) |
| Delta column, sequential | 32 MB | 171 kB |
| Delta column, 5 % deleted and 10 % new from another origin | 33.6 MB | **754 kB** (0.19 B/element) |
| Plain `<u8` column, same edit | 33.6 MB | 6.3 MB |
| Random 64-bit per element (rejected) | 32 MB | 32 MB |
| Random UUID per element (rejected) | 64 MB | 64 MB |

Delta encode or decode of 4 M ids: 11 ms. Deflate of the edited column:
0.13 s, on the writer's existing parallel per-block path.

### 3.6 Ride-scale estimate

The largest real model has ≈4 M own faces (14 M placed, 26.6 M in the
linked composite). Assuming V ≈ 5 M, E ≈ 9 M and F ≈ 4 M, that is ≈18 M
elements. Ids belong to definitions, not to placed copies.

| | Implicit (first save after migration, untouched meshes) | Every mesh edited at some point (worst case) |
|---|---|---|
| File growth | ≈0 (JSON only) | ≈3.4 MB on a 141 MB file (+2.4 %) |
| Load | 0 | ≈50 ms delta decode, parallel per block |
| Save | 0 | ≈0.5 s deflate on one core, parallel per block |
| Resident memory | 0 | 144 MB if every store is dense (≈7 % of the 1.9 GB "all meshes touched") |

A typical session edits tens of meshes, so the resident cost is a few MB.

Phase 2 gates, checked on the `test_library_huge_perf.py` generator scaled
to ≈18 M elements:

- open: under 2 % slower
- save: under 5 % slower
- RSS: under 2 % higher with the implicit form
- one edit on the largest mesh: no measurable change

If the dense worst case matters in practice, a run-length `IdColumn`
variant (runs of +1) is the fallback. The file format already permits it,
because the delta array is exactly that data.

## 4. Semantics under edits

**Rule 0 (continuity): same session key, same id.** Every op that keeps
an element's key keeps its id, with no per-op code. That covers:

- move, rotate, scale and Move-to (vertices move; edges and faces keep
  their keys)
- paint, soften, hide, tag, reverse and orient faces
- ring rewrites in place, and hole add or remove on a kept face
- the first half of an edge split (the kernel keeps it, `store/diff.rs`)

**Rule 1 (succession): one successor inherits.** An op can remove an
element and create its replacements in the same step. For example, the
plane rebuild renews the keys of the faces on a plane, and push/pull
rebuilds its cap. The op already names a mother or paint donor for each
new face (`drawing_face::Division::{Holes, Carve}`, `push_extrude`
donors, `push_translate::Translated.replaced`, `plane_repair` donors).
Exactly one successor of each old element inherits its id:

- **face**: the largest-area successor (ties go to the first created)
- **edge**: the successor that contains the old edge's first end
- **vertex**: the welded survivor

The other successors get fresh ids. The step reports `derived:
[(new, parent)]` so the op log and the renderer can see the lineage.

**Rule 2 (merge): the survivor keeps its id.** The survivor is the one
the kernel keeps today (first in mesh order). Absorbed ids end, and the
step reports `merged: [(absorbed, survivor)]`.

**Rule 3: everything else is fresh.**

| Edit | Ids |
|---|---|
| Line across a face (split) | The larger half keeps the face id; the other half is fresh, `derived` from it. The new edge is fresh. Edges the line splits keep their id on the first half. |
| Rectangle or circle inside a face | The outer face (now with a hole) keeps its id. The inner face is fresh, `derived`. |
| Erase a line between coplanar faces (merge) | Rule 2. |
| Push/pull (`keep_base = false`) | The moved cap keeps the face id (Rule 0 or 1). Side walls and their edges are fresh, `derived` from the cap's boundary edges. |
| Push/pull (`keep_base = true`) | The face left in place keeps the id. The new cap is fresh, `derived`. |
| Push through (the face disappears) | The id ends. |
| Follow Me | The start cap inherits the profile's id. Walls are fresh. |
| Offset | The outer ring face keeps its id. The inset is fresh, `derived`. |
| Intersect faces | Pierced faces split by Rule 1. |
| Divide or join edges | Divide: the first piece keeps the id. Join: the first segment's id survives (Rule 2). |
| Weld or Purge | Survivors keep their ids. Purged debris ends. |
| Erase | The ids end and are **never reused**. |
| Undo / redo | Ids come back **exactly**: they are part of `State`, the minter never rewinds, and redo restores the state the first run made (the store already does this for keys). A command that re-executes on redo instead of restoring is a bug, and it shows up in tests as new ids. |
| Copy / paste of loose geometry (Move with copy, paste into the same definition) | Fresh ids. The feed's `copies` gives (source, copy) pairs. No source link is stored in the file. |
| Copy of a group placement | New placement uid and new definition uid. The copy **keeps the element ids** (unique per definition), so a face of the original and the same face of the copy share an id: free lineage. |
| Component instance copy | The definition is shared, so nothing changes in the elements. |
| Make Unique | New definition uid. Element ids are **kept**. A reference by occurrence (placement path, element id) still resolves after Make Unique, so BCF pins and anchors survive it. |
| Make Group / Make Component | The elements move into a new definition and **keep their ids** (it is fresh, so nothing collides). |
| Explode | The elements move into the parent definition and keep their ids unless the parent already holds one (two copies exploded into the same parent). Then that element gets a fresh id. `copies` reports every pair, as it does today. |
| Split into Pieces | Each piece is a new definition. Its faces keep their ids. |
| Solid operations (union, subtract, trim, intersect, split, outer shell) | The result definitions are new and their elements fresh, with `derived` taken from the op's paint donors where it has them. These ops rebuild from a merged world copy, so keeping ids would be a guess. |
| Import (.skp, .dae, .obj, .stl, .dxf, .rfa, glTF) | Fresh ids, minted per definition in file order, so they stay implicit. |
| Insert a `.built` or `.igz` as a component, or paste across documents | New definition uids. Element ids are **kept**, with their origins remapped into the host's origin table. Origins are logical, so nothing is lost. |
| Linked file | Definition uid = the first 16 hex of UUID5(URL namespace, `built:link:v1:def:<link placement uid>:<source definition uid>`), the same scheme the placement uids already use. Element ids are the source's, verbatim. A reference made in the source as (source placement path, element id) maps into the host as (link wrapper uid, the mapped child uids, element id) with no table. A packed link keeps the same derived uids. |

## 5. File format: `built_format: 2`

### 5.1 Document header

```json
{
  "built_format": 2,
  "requires": [],
  "doc": {
    "id": "5f0c…32 hex",
    "origins": ["0", "9c1e4a7b2d3f6e10", "41d7…"]
  },
  "app_version": "…",
  "scene": { … }
}
```

- `doc.id` is drawn when a document is created. A format-1 or `.igz` file
  gets one on load, and it is written on the next save. Files are never
  rewritten on open, so until that save the id is provisional (see Q9).
  "Save As" keeps the id: the new file is the same document. "Save a
  Copy" and a template instance draw a new one.
- `doc.origins` is the origin table (§3.2), with index 0 always `"0"`.

### 5.2 Geometry blocks

Each `geom` block (the loose geometry, every entry of `prototypes`,
every kept component) gains:

```json
"geom": {
  "member": "geom/3.bin",
  "uid": "a41f09c2e1b7d360",
  "ids": {"vertex": "0", "edge": "0"},
  "arrays": { …, "face_id": ["<u8", [12345], 81920] }
}
```

For each kind there is exactly one of `ids[kind]` (implicit: packed first
id, hex) or the `<kind>_id` array (delta-coded, §3.4). The array length
equals the kind's count, and the reader checks this as it checks the
other arrays. The reader's dtype list gains `<u8`. Ids must be unique per
kind in a block. A duplicate is a damaged file: the reader re-mints the
duplicates under a fresh origin and reports it, the way it repairs other
damage.

Record rows gain `uid` (materials, scenes, section planes, sheet items,
annotations), or change it (section planes, from int to string).

### 5.3 Migration (format 1, `.igz` 1–3)

Ids are deterministic, so the same file gets the same ids every time it
is opened before a resave:

- **Elements**: origin 0, counter = the element's index in its block's
  file arrays (per kind). Binary blocks give this exactly. JSON meshes
  (old `.igz`) use the loader's vertex order, which is deterministic for a
  given file and app version. A future change to that order would change
  the derived vertex ids of a never-resaved JSON `.igz`. That is
  acceptable: they become fixed at the first save.
- **Definitions**: the first 16 hex of UUID5(URL namespace,
  `built:legacy:v1:def:<proto index>`) for prototypes, `…:kept:<index>`
  for kept components, `0000000000000000` for the loose geometry. These
  are unique per document, and that is all a definition uid needs.
- **Rows without uids** (materials, scenes, annotations, sheet items):
  UUID5 of `built:legacy:v1:<table>:<index>`, with the sheet uid for sheet
  items. The section integer converts as in §2.2.
- The migrated meta is exactly the implicit form, so migration costs
  nothing in time or size. A raw-copied `geom/` member stays
  byte-identical, and only its JSON meta gains `uid` and `ids`.

### 5.4 Writing other formats

- "Save a Copy as IngeTrazo (.igz)" writes format 3, which has no ids.
  Reopening that `.igz` derives legacy ids, which differ from the
  `.built`'s ids. The copy dialog already warns about precision; it will
  say this too.
- A `.built` 2 file opened by a Built 1 reader is rejected (unknown
  version), as today.

### 5.5 Forward compatibility

Later features should add data without bumping the format:

1. **Extension maps.** The document, every placement (it has `ext`
   already), definition, material, tag, scene, sheet and sheet item may
   carry `"ext": { "<namespace>": … }`. A reader keeps `ext` it does not
   understand, verbatim, through edits and saves. New optional data goes
   there first (namespaced `built.<feature>`) and is promoted to a named
   field only in the next major format.
2. **Unknown keys outside `ext` are ignored**, not preserved. A writer
   that needs a field to survive older readers puts it in `ext`.
3. **Unknown geometry arrays** survive only while their block is
   unchanged (raw-copied). When the block is rewritten they are dropped,
   because a reader cannot keep a per-element column it does not
   understand consistent through edits. Per-element data that must
   survive goes through `requires`.
4. **`requires`** lists feature names a reader must implement to open
   the file correctly, like PNG's critical chunks. A reader that lacks
   one refuses to open the file and names the missing feature. An empty
   list means any format-2 reader is fine. The major number moves only
   when an existing field changes meaning.

### 5.6 Stability policy (added to built-format.md)

- Format 2 is the long-term format. Every Built release reads every
  earlier Built format and every `.igz` format.
- Ids are never reused and never change meaning.
- A field, array or table is never repurposed: new meaning means a new
  name.
- Additive changes follow §5.5. Anything else bumps the major version and
  keeps a reader for the old one.
- A corpus of files from each release (`tests/fixtures/built_format/`)
  must open, keep every id and round-trip byte-stably where nothing was
  edited.

## 6. Protocol and change feed

### 6.1 Steps carry persistent ids

The session keys stay. They are the fast in-process handles, and every
existing consumer keeps working. The feed adds the following.

```
Step
  document     doc.id
  …            (unchanged)
  derived      [{definition, kind, id, parent}]       Rule 1
  merged       [{definition, kind, id, into}]         Rule 2

MeshChange
  definition   the definition uid (persistent; `mesh` stays the session key)
  ids          {vertex|edge|face: {"<session key>": "<element id>"}}
               for every key in created / removed / modified / moved
  materials    persistent material uids next to names in Step.materials
```

- `ids` costs as much as the change itself (a lookup per listed key).
  It is built when the step's JSON is produced, so in-process
  subscribers that ignore it pay nothing.
- Queries (`faces`, `edges`, `vertices`, `triangles`, `selection`,
  `select`) accept `ids` wherever they accept `keys`, and their answers
  carry `id` next to `key`.
- Commands accept an element as a key, an id string (`"f:…"`) or a
  reference object.
- A new query, `resolve`, turns `{definition|path, ids}` into session
  keys and lists the missing ones. That is how an agent, a BCF pin or
  the web client gets back from a saved id to a live element.

### 6.2 Operation log (design only; not built in Phase 2)

An op log entry is the **net patch of one step, by persistent id**, not
the command and its arguments:

```
OpEntry
  doc, origin, seq          who made it, and its order within that origin
  base                      the log position the author last applied
  definitions: [{
    def,
    created:  vertices {id, position}, edges {id, ends: [vid, vid], flags, curve, tag},
              faces {id, rings: [[vid…]…], attrs (materials by uid), flags}
    removed:  [id]
    moved:    [{id, position}]
    modified: field values by id
  }]
  rows: placements / materials / tags / scenes … by uid, field values (the journal has them)
  derived, merged           lineage, informational
```

- *Why a patch and not a replayed command.* Replaying a command means
  every client, including a web client, must run a bit-identical kernel
  over identical floats. A patch is exact by construction: positions are
  already f64, and topology is named by id. The feed already computes the
  net diff (`changes_since`). The patch is that diff with full element
  data, read from the after-state.
- **Applying a remote entry**: open one transaction. For each definition,
  resolve ids to slots (§3.3), create elements with the *given* ids (the
  minter is bypassed: those ids belong to the remote origin), kill the
  removed ones, write the fields, update the weld registry and incidence,
  and check the store's invariants (`Store::check`). It becomes one
  History step, marked remote, which the user can undo as a step.
- **Conflicts**: a central sequencer (the sync server) orders entries.
  An entry whose `base` is behind and that touches an element changed
  since then is rejected, and its author rebases by re-running its
  command on the new state. Geometry does not merge well field by field,
  so no CRDT for geometry. Rows (names, tags, record fields) can merge
  last-writer-wins per field.

## 7. Exporters

### 7.1 IFC GlobalId

Today `object_guid` writes a random `uuid4` into the record at export and
bumps `scene.version`. The GlobalId therefore stays stable only if the
user saves after exporting. Export twice in different sessions without
saving in between and the GlobalIds differ.

New rule, which never writes to the document:

- A record's stored `guid` wins: imported, typed by the user, or saved
  from an earlier export.
- Otherwise: `stable_guid(namespace, "occurrence", *uid_path)`, with the
  placement uid path from the root (one segment for a root placement).
  Rename, move, geometry edits, record edits and Make Unique keep it. A
  copy gets a new uid, so it gets a new GlobalId, which is what
  object-record.md requires.
- Face-tagged legacy objects keep `ifc.guid` in their face attrs, as
  today.
- `namespace` is `ifc_identity.namespace` (the project's, or the
  document's if it was ever set) and otherwise `doc.id`. Choosing it no
  longer writes to the document either.
- Types, systems, relationships and the spatial skeleton keep their
  semantic GUIDs. A test exports, reopens without saving, exports again,
  and checks that the GlobalIds match.

### 7.2 BCF

Tasks write `ifc_guid: null` today (`built_render/src/tasks.rs`). New:

- `ifc_guid` is the occurrence's GlobalId (§7.1).
- `authoring_tool_id` is `built:<uid>/<uid>…`, optionally followed by
  `#f:…` to pin an element.
- Reading back accepts both forms, and also the bare uid written today.

### 7.3 glTF (Blender link and exports)

- Node extras: `built_uid` (today) plus `built_path`.
- Mesh extras: `built_def`, the definition uid, which lets Blender share
  meshes between instances by identity.
- Per-face ids are **not** exported by default. The link sends no mesh
  edits back. The option (a `_BUILT_FACE` vertex attribute indexing an
  extras table) waits until something needs it (Q11).

## 8. Phase 2 plan (after "go")

1. **Store**: `IdColumn` and the minter, `alloc` and `revive`, a diff
   over id columns, id → slot resolution. Rule 1 and Rule 2 hooks at the
   donor and mother sites listed in §4, and in the plane rebuild.
2. **Document**: `Definition.uid`, `doc.id`, the origin table, row uids
   (materials, scenes, sections, annotations, sheet items), and the
   definition rules for Make Unique, copy, explode, make group and
   links.
3. **File**: writer and reader for format 2, the `<u8` dtype, legacy
   derivation, the `.igz` copy note, the stability policy in
   built-format.md.
4. **Protocol**: the feed fields, ids in queries and commands, `resolve`.
   core-protocol.md §2 updated ("Never persisted" goes away).
5. **Exporters**: IFC GlobalId derivation without writes, BCF
   components, glTF extras.
6. **Tests**:
   - save/load round trip of every id kind
   - migration determinism (open the same file twice, same ids)
   - undo/redo exactness over random op sequences (the existing fuzz
     goldens, compared with ids)
   - the §4 table, one test per row
   - linked-file mapping
   - IFC GlobalId stable across export → reopen → export
   - protocol ids and `resolve`
   - ride-scale gates (§3.6)

## 9. Open questions, each with a recommendation

| # | Question | Recommendation |
|---|---|---|
| Q1 | Do vertices get ids? | **Yes** (§2.1). They cost nothing until edited, and the op log needs them. |
| Q2 | Should faces refer to materials by uid now? | **Not in format 2.** Add the material `uid` now and keep references by name (a rename restamps, as today). A later switch adds a `mat_uid` attribute under `requires`. Moving references now is a refactor across the Library and paint code that no current consumer needs. |
| Q3 | Origin per session or per installation? | **Per session.** A Finder copy of a file edited on two machines stays conflict-free, at the cost of one table entry per saving session that minted (16 hex characters). |
| Q4 | Which half keeps the id on a face split? | **The larger area.** "The wall" stays the wall when a window is drawn on it. The alternative, "whatever the kernel keeps", depends on how the kernel is implemented. |
| Q5 | Should lineage be saved in the file? | **No.** It goes in the feed and the op log only (`derived`, `merged`, `copies`). Saving it would grow without bound. |
| Q6 | Does a group copy keep element ids? | **Yes.** Uniqueness is per definition. It gives lineage for free and costs nothing. |
| Q7 | Explode: keep if free, or always re-mint? | **Keep if free.** An exploded group's faces stay the same faces for agents and pins. Collisions only arise for repeated copies and are reported in `copies`. |
| Q8 | Should scene references (`view_key`) move to uids in Phase 2? | **Yes.** Rename already rewrites `view_key`, so the change is small, and name references are what the collaboration work would trip on. |
| Q9 | Legacy document id: provisional random, or derived? | **Provisional random**, saved on the next save. Element and definition ids do not depend on it, and its users (links, collaboration, IFC fallback namespace) need a saved file anyway. |
| Q10 | Should "Save a Copy as .igz" carry ids in `ext`? | **No.** `.igz` is the compatibility exit. Warn instead. |
| Q11 | Per-face ids in glTF for the Blender link? | **Not now.** Node and mesh extras only, until Blender sends mesh-level data back. |
| Q12 | Element ids for solid operation results? | **Fresh, with donor lineage.** Revisit if users pin faces of solids. |
| Q13 | Annotations by uid, `erase_stroke` and other indices? | **Accept both**, uid preferred. Index arguments stay for existing scripts. |
| Q14 | Phase 2 size | One lane. The store and file work carries the risk; protocol and exporters are small. Rule 1 hooks touch the drawing and push/pull files that BIGMESH does not own (BIGMESH works in the resident pack, the snap index and the panels). |

## 10. Implementation notes (Phase 2)

Code: `rust/built_core/src/element_ids.rs` (origins, minting, text form,
definition and material uids), `store/ids.rs` (the id column),
`store/succession.rs` (Rules 1 and 2), `file_ids.rs` (format 2 header,
block ids, row uids), `doc/history/dispatch.rs` (succession after each
step), `doc/feed/capture.rs` (feed ids). Tests:
`tests/test_persistent_ids.py`, `tests/test_persistent_ids_perf.py`
(perf-marked), Rust unit tests in those modules.

Where the implementation differs from the text above:

- **Succession runs after each History step**, on the open mesh (and the
  undo/redo target), comparing the state before with the state after, not
  inside each kernel op. It is a pure function of the two states, so a
  redo that restores the step's state gets the same ids. Steps that edit
  other meshes than the open one (Explode's source, a definition shared
  back) rely on Rule 0 and on their own id handling (Explode's keep-if-free,
  Make Group and Split into Pieces through the store's `extract`).
- **Merge**: the *larger* original keeps its id (the text said "first in
  mesh order"); split and merge then follow one rule, largest overlap
  first.
- **Pushing a lone face**: the kernel keeps the original face as the
  bottom of the new box; succession hands its id to the moved top, as §4
  says, and the bottom gets the top's fresh id (`derived`).
- **Derived uids** (migration, linked definitions) are the first 16 hex of
  SHA-256 of their name, not UUID5. Linked definitions are named after the
  source document id and the source definition uid
  (`built:link:v1:def:<doc id>:<uid>`): a linked mesh is shared by every
  placement of that source, so it cannot carry a placement-scoped uid.
  Occurrence references still go through the link wrapper's uid path.
- **Section planes**: integer uids of older files are kept as they are
  (unique and stable in their file; saved views and sheets refer to them);
  new planes get 16-hex uids.
- **Scenes in sheets**: frames keep `view_key: "scene:<name>"` (every
  sheet consumer reads it) and gain `view_uid`; on load a frame whose
  scene was renamed follows the uid.
- **Row uids** are kept unique per table at save: a row duplicated with
  its original's uid takes a new one there.
- **glTF**: `built_def` sits in the node extras next to `built_uid` (the
  link reads node extras).
- **BCF**: the export writes `IfcGuid` on each component (the GlobalId the
  IFC export writes, `core.ifc_identity.component_guid`); stored tasks keep
  their `authoring_tool_id` as the placement uid, and the `built:<path>#id`
  form is accepted when reading.
- **IFC**: a record's stored `guid` still wins; otherwise the GlobalId is
  `stable_guid(namespace, "occurrence", *uid path)`. The namespace is the
  stored one or the document id, and the export no longer writes it.
  Records created by the inspector still get a stored GUID at creation, as
  before.

Left for later (not needed by any consumer yet):

- the operation log itself (§6.2): the feed carries everything it needs;
- material uids in `Step.materials` (names only, as before);
- a new document id on "Save a Copy" and template instances (both keep the
  id today);
- per-face ids in glTF (§7.3, Q11);
- `ext` maps on rows other than placements and the document.

## 11. Reserved for exact geometry

Exact geometry is a separate design lane ([exact-geometry.md](exact-geometry.md),
its §2 asks for this reservation): a face may carry an exact surface
(plane, cylinder, cone, sphere, torus, NURBS) and an edge an exact curve
(line, arc, ellipse, NURBS), kept through edits where possible. Format 2
reserves its place so it needs no format 3:

| Reserved | Where | Meaning when implemented |
|---|---|---|
| `built.exact_geometry.v1` | the `requires` list | Set by every file that carries exact data, so an older editor refuses it before it can drop bindings. Ordinary format-2 files omit it. |
| `ext["built.exact_geometry"]` | the document's top-level `ext` map | Version, units, the immutable type-tagged geometry pools and their binary members. |
| `face_surface_id` / `face_surface_ref` | geometry block arrays, paired | Sparse, sorted by packed face id (`<u8`): the faces bound to a surface, and each one's pool index (`<u4`). |
| `edge_exact_curve_id` / `edge_exact_curve_ref` | geometry block arrays, paired | The same for edges and exact curves. |
| `face_surface_*`, `edge_exact_curve_*`, `face_loop_*`, `coedge_*` | geometry block array name families | The sparse sense, edge interval, face loop range and coedge columns. |

Bindings are keyed by packed element id within the block's definition,
never by slot or position. A block without them is faceted: no bytes, no
memory, no migration. The existing `edge_curve` column (segment groups) is
unchanged.

This build refuses a file that lists `built.exact_geometry.v1` in
`requires`, carries the `ext` namespace, or has an array of a reserved
family ("needs features this build of Built lacks"). Refusing rather than
dropping means no exact data is ever lost by a build without the feature.

In memory the store has `State.exact` (`store/exact.rs`): optional
per-slot `face_surface` and `edge_exact_curve` pool indices, `None` for an
all-faceted mesh. They are part of the snapshot, so undo, redo and
restores carry them; a new slot starts faceted; and the succession pass
(§4, Rule 1) gives the successor that inherits an id the binding of the
element it came from (unit test
`the_inheriting_half_also_inherits_an_exact_surface`). Pools, evaluators,
the file arrays' reader and writer, and binding repair after a duplicate
id are the exact-geometry lane's.
