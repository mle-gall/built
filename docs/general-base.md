# The general base: Sweep, Live cut, Sketch workspace, Repeat, Presets

Technical design for owner review, against `origin/main` at `1389c34fa`
(2026-11-27). Design only: no product code lands with this document.

The UX is settled: pass 5, option A for all five topics
(`ux-pass5/compare/index.html`), with the owner's two remarks after the
pick:

1. **Anchored cutters and adaptive copies.** A cutter can be positioned
   relative to an element that is not what it trims. A bay of purlins
   between two non-parallel beams: each purlin's end detail sits at a fixed
   distance from the beam it meets. So Repeat copies share one *recipe*
   (profile, cut definitions, offsets) but each resolves its own geometry.
   Editing the recipe updates all of them; Release bakes; copies that
   resolve identically share geometry as instances.
2. **Sketch workspace layout:** tools on the left (in the rail's place),
   the settings pane on the right (in the dock's place). The pass-5 board
   had them the other way round.

Binding principles (pass-5 brief): general primitives with trades as
presets; no node graph, relations made and edited the .skp way in the
viewport and shown on the objects, the tree and the record; facets are
geometry; every element is filed with an IFC class and a record; Release
bakes.

## 1. Summary

One new concept carries all five features: a **recipe**, a document row
that says how a placement's geometry is made. The geometry itself stays
ordinary Built geometry (facets, edges, persistent ids, exact bindings) in
an ordinary definition, saved in the file as today. The recipe is kept
beside it so the geometry can be made again when an input changes.

| Feature | Recipe kind | Produces |
|---|---|---|
| Sweep | `sweep` | the geometry of one element (a member, a wall, a gutter) |
| Live cut | `cut` | nothing of its own; it modifies its target's geometry |
| Repeat | `repeat` | placements (copies) of a source, instanced or adaptive |
| Sketch | `sketch` | a 2D profile or section, used by sweeps and cuts |
| Preset | `preset` | nothing; default values and locks for the other kinds |

Recipes refer to each other and to model elements through **references**
(§2.3). Those references form a dependency graph that the user never sees
as a graph (§3). The graph is evaluated incrementally inside the History
step that changed an input, so a step is still one undo entry and undo
never re-evaluates.

Naming: in the code "recipe" already means a Project Structure node's
filing recipe (`registry.structure.nodes[key].recipe`, `structure.rs`).
This design keeps the owner's word "recipe" in prose, but its code, file
namespace and protocol verbs use **live** (`built_core::live`, `LiveTable`,
`ext["built.live"]`, `live_set`), so the two never share a name.

What the user sees as "the relation is visible on the object, in the tree
and in the record" is this data read back: a cut row under its target, the
Repeat row with its rule, a sweep's record with its preset.

## 2. Data model

### 2.1 Where recipes live

| Data | In memory | In the file (format 2) |
|---|---|---|
| Recipe rows (sweep, cut, repeat, sketch) | `LiveTable`, a Rust table owned by the `Doc` arena and journalled through its `Op`s (`doc/mod.rs`: `Set`, `Ins`, `Rem`, `Patch`), so undo is a diff like every document field | `ext["built.live"]` on the document: `{"v": 1, "rows": [...]}` (kept today in `scene.doc_ext`, `file_document.rs`) |
| Which placement a recipe drives, and whether its definition is generated | `Recipe.owner` (placement uid); reverse index rebuilt on load | `ext["built.live"]` on the placement: `{"row": "<uid>", "key": "<content key>", "stamp": "<hex>"}`. Format 2 carries `ext` only on the document and on placements (persistent-ids §10), so the placement, not the geometry block, says that its definition is generated |
| Generated geometry | ordinary definitions (store `State`) | ordinary geometry blocks, deduplicated by mesh identity as today (`file_prepare.rs`), so copies sharing a generated definition save it once |
| Presets used by the document ("This project") | `PresetTable` (rows) | `ext["built.presets"]` on the document |
| Library presets, profiles, sections | files in Library collections | `.builtpreset` / `.builtprofile` JSON files (§2.7) |
| Evaluation cache | `LiveCache` (not saved) | none |

Rules:

- **The file always carries the evaluated geometry.** A reader that knows
  nothing about recipes opens the file and sees exactly what the author
  saw. Opening never re-evaluates; it validates stamps (§3.6).
- **The record stays on the placement** (`group.ifc`, object-record.md),
  never in the recipe. A sweep's IFC class, name, tag and psets are the
  element's, as today. The recipe holds only how the geometry is made, and
  a reference to the preset it came from.
- Every row has a 16-hex record uid (persistent-ids §2.2), kept for life.
  Copying a recipe-driven placement copies its row with a new uid.
- Row uids, placement uids and element references use the logical text
  forms of persistent-ids §3.1, never session keys.

### 2.2 Recipe rows

All rows share `{uid, kind, owner?, preset?, state, v}`. `state` is the
last evaluation result (`ok`, or a failure kind, §3.5); it is saved so the
tree can show a failure without evaluating on open.

**Sweep**

| Field | Value |
|---|---|
| `profile` | `{"library": "<collection>/<profile key>", "params": {…}}` (a catalogue size or parametric family), `{"sketch": "<row uid>"}`, or `{"face": <reference>}` captured once into a sketch row (a picked face is copied, not linked) |
| `path` | `{"points": [p0, p1]}` in the parent's frame; `{"chain": [<edge reference>…], "closed": bool}`; or `{"ends": [<end>, <end>]}` where each end is a point or an **anchored end** `{"to": <reference>, "offset": mm, "extend": bool}` (the purlin case, §2.4) |
| `attachment` | one of nine bounding-box points, or a named point saved with the profile (`"shear_centre"`) |
| `rotation`, `mirror` | degrees about the path; mirror across the profile's local Y |
| `corner` | for chains: `mitre` or `butt` (the one corner option of pass-5 research) |
| `segments` | facet count for round parts of the profile, as for a Circle (facets are geometry) |
| `cuts` | ordered list of cut row uids that belong to this element (end fittings first, then body cuts) |

**Cut**

| Field | Value |
|---|---|
| `target` | the placement uid it modifies (the owner) |
| `cutter` | `{"plane": {anchor}}`, `{"bisector": [<member uid>, <member uid>]}`, `{"body": <placement uid>, "clearance": mm}`, `{"section": "<sketch row uid>", "extent": "through" \| "depth" \| "both" \| "to_face", "depth": mm, "to": <reference>}`, or `{"repeat": "<repeat row uid>"}` (a repeated cutter, one row) |
| `operation` | `subtract`, `intersect`, `union`, `trim` |
| `role` | `end` (a fitting: sets where a sweep ends, changes its length) or `body` (removes material, length unchanged), Tekla's split |
| `end` | for `end` cuts on a sweep: `start` or `end` |
| `anchor` | `{"ref": <reference>, "frame": "face" \| "axis_end" \| "point", "offset": [dx, dy, dz] or mm, "rotation": deg}`: where the cutter sits. The anchor reference need not be the target or the thing it trims (owner remark 1) |
| `on` | boolean (the eye in the Cuts list) |
| `preset` | the cut preset it came from (Mitre, Cope, Notch, End plate, Opening, Penetration, saved ones) |

**Repeat**

| Field | Value |
|---|---|
| `source` | placement uid (a sweep, a group, a component, a cutter, or another Repeat's owner for nesting) |
| `run` | `{"points": [a, b]}`, `{"edge": <reference>}` (bound to its host: re-divides when the host changes), `{"chain": [...]}`, `{"polar": {"centre", "axis", "angle"}}`, or `{"between": "<repeat row uid>"}` (Rhino's Between) |
| `rule` | `{"count": n, "step": mm}`, `{"divide": n}`, `{"fit_max": mm}`, `{"spacing": mm}`; `measure`: `pitch` or `gap` |
| `ends` | `{"start": mm, "end": mm, "at_start": bool, "at_end": bool}` |
| `orientation` | `fixed`, `follow` (polar and path runs), `alternate` (mirror every other copy) |
| `skips` | list of positions along the run, each `{"from": "start" \| "end", "at": mm}`, so a skip stays at the door when the count changes |
| `adapt` | `auto` (default), `identical`, `adaptive` (§2.5) |
| `copies` | derived, not saved as rows: copy *n* is placement uid `h64("built:repeat:v1:<row uid>:<station key>")` (§2.6) |

**Sketch**

| Field | Value |
|---|---|
| `use` | `profile` or `section` (the Library usage tag: the sweep picker and the cutter picker show different lists) |
| `plane` | an anchor (`{"ref", "frame", "offset"}`): a face, a plane, a member end ⟂ its axis; profiles in the Library have no plane |
| `curves` | 2D entities in sketch units (mm): `line {a, b}`, `arc {c, r, a0, a1}`, `circle {c, r}`, each with a persistent sketch element id and an optional `segments` |
| `points` | named points: `origin`, attachment points (nine are implicit), extras such as a shear centre |
| `dims` | driving dimensions (§5.3): `{kind: distance \| radius \| angle, a, b, value}` |
| `extrude` | for sections: as the cut's `extent`, kept on the sketch so the Library section carries a default |

Regions (outer loops and holes) are derived from the curves, not stored.

**Preset**

| Field | Value |
|---|---|
| `name`, `pack` | "Steel rafter", "Steel EN 10365" |
| `primitive` | `sweep`, `cut` or `repeat` |
| `values` | the primitive's fields (profile, attachment, rotation; or cutter kind, operation, numbers; or rule) |
| `record` | IFC class, predefined type, object type, record field defaults, material |
| `naming` | pattern such as `R-{n}` |
| `node` | the Project Structure node it files into (made if missing) |
| `locks` | per field: `fixed` (the preset decides) or `free` (per element) |
| `rev` | integer, bumped on every edit; elements store the revision they follow |

### 2.3 References

A reference names something a recipe depends on. One closed set of kinds,
resolved by one module:

| Kind | Form | Resolves to |
|---|---|---|
| point | `[x, y, z]` in the owner's parent frame | a point |
| element | `{"path": [<placement uid>…], "id": "f:…" \| "e:…" \| "v:…"}` (persistent-ids §3.1, occurrence form) | a face's plane (or exact support), an edge's line/curve, a vertex |
| axis | `{"sweep": "<row uid>", "at": "start" \| "end" \| t \| {"from": "end", "mm": d}}` | a frame on a sweep's path: origin on the path, X along it, Z from the profile's up |
| datum | `{"sweep": "<row uid>", "point": "<profile point name>", "at": …}` | a profile's named point at a station |
| placement | `{"placement": "<uid>"}` | the placement's frame |
| slot | `{"slot": "start_host" \| "end_host" \| "<name>"}` | bound per copy by a Repeat (§2.5); a standalone element binds its slots once |

An element reference to a generated face survives re-evaluation because
generated faces keep their ids by role (§2.6). A reference that no longer
resolves is a failure of the dependent recipe, never a silent change.

### 2.4 Anchored ends and anchored cutters

Owner remark 1 in data: the purlin's sweep has

```
path:   {"ends": [{"to": {"slot": "start_host"}, "offset": 0},
                  {"to": {"slot": "end_host"},   "offset": 0}]}
slots:  {"start_host": <element ref: R-1 web face>,
         "end_host":   <element ref: R-2 web face>}
cuts:   [notch N-1 (body, section),  end plate E-1 (end, plane)]
```

and the notch's cut row has `anchor: {"ref": {"slot": "start_host"},
"frame": "face", "offset": 20}`: the section sits 20 mm from R-1's web
whatever the purlin's length or angle. The end plate's anchor is the same
slot with its own offset. Neither cutter is anchored to the purlin it
trims, and neither trims R-1.

Resolution of a sweep with anchored ends:

1. Place the axis line (the run places it for a copy, two picked points
   for a standalone one).
2. Intersect the line with each host (plane of a face, or a member's axis
   frame) and apply `offset`; `extend` lets the end grow past its drawn
   point. No intersection (parallel) is the failure `end_unresolved`.
3. Resolve every cut anchor in the same frame, so the cutters follow.

### 2.5 Adaptive copies

A Repeat places its source at each station. What a copy *is* depends on
the source:

| Source | `adapt = auto` resolves to | Each copy | Shared |
|---|---|---|---|
| Plain group or component, no recipe | `identical` | an instance of the source's definition | everything |
| Recipe whose references are all inside it (a free-standing member, its own end cuts) | `identical` | an instance | everything |
| Recipe with slots or external anchors (the purlin between R-1 and R-2) | `adaptive` | the source's recipe with its slots bound at this station, resolved on its own | the recipe (profile, cut definitions, offsets) and, where resolution agrees, the geometry |

`adaptive` copies share the recipe row: the copy has no row of its own,
only its bindings, which are computed, not saved. Editing the recipe
(profile swap, notch depth, end-plate offset) re-evaluates every copy.

**Sharing geometry.** Each copy's resolved inputs are expressed in the
copy's own frame (path start at the origin, X along the path) and hashed,
quantised to 1 µm and 1 µrad: profile revision, length, end-cut planes,
cutter poses. Copies with equal keys point to one generated definition, so
they are instances: one cut-list line with a quantity, one IFC
representation map. A bay of parallel beams gives one definition; a bay
between non-parallel beams gives one per distinct length. The inspector
states it: "24 copies · 5 distinct parts".

`identical` forces instances (copies whose slots are unbound keep the
source's geometry); `adaptive` forces per-copy resolution on a source with
no external references (useful when a later cut will differ). The inspector
shows the switch only where it changes something.

Cuts travel with the source: cut rows listed in the source's `cuts` are
part of the recipe, so every copy is cut. A Repeat whose source is a
cutter (`cutter: {"repeat": …}`) cuts its target once per copy: one row.

### 2.6 Generated geometry and persistent ids

A recipe's geometry lives in an ordinary definition, which its
placement's `ext["built.live"]` marks as generated. Elements in it get their
ids by **role**, not by minting:

```
role    = a tuple naming what made the element:
          sweep side face = (side, profile edge id, path span)
          sweep cap       = (cap, start | end)
          cut face        = (cut, cut row uid, cutter face role)
          edge, vertex    = the sorted roles of the faces it bounds
origin  = the reserved generated origin: one entry in doc.origins,
          h64("built:generated:v1")
counter = h40(role) (the packed counter is 40 bits, persistent-ids §3.2);
          a collision inside one definition probes to the next free value
          in role order, so the result is still deterministic
```

The same role gives the same id after every re-evaluation, so a dimension,
a BCF pin or another cut anchored to "R-1's top flange face" keeps working
when R-1 is lengthened or re-profiled to another I-section. A role that
vanishes (the cut is removed) ends its id; if the same role comes back,
the id comes back with the same meaning. That is the one exception to
"never reused" (open question Q2): an id names a role, and the role is the
same thing.

Profile edges have stable ids: parametric families number their edges by
name (top flange top, web left, …) so swapping IPE 240 for IPE 300 keeps
every role; a sketch's curves keep their sketch element ids. Swapping
families (IPE to HEA keeps roles; IPE to a tube does not) ends ids whose
role no longer exists, and dependants report it (§3.5).

The ordinary succession pass (`store/succession.rs`) does not run on a
generated definition, and neither does the exact rebind pass
(`store/exact_bound.rs`): the generator sets ids and exact bindings
directly (§4.1). Both still run on user geometry in the same step.

Copies in a Repeat take derived placement uids from the repeat row uid and
the station key (the copy's position along the run, quantised, plus its
nest path), so IFC GlobalIds (persistent-ids §10: `stable_guid(…,
*uid path)`) stay the same across exports while the count does not change
around them. Under Fit ≤, changing the run length shifts stations; a copy
keeps its uid while it keeps its index from the start, as Revit keeps its
array members.

### 2.7 Library files

| Library tab | Holds | File |
|---|---|---|
| Profiles | catalogue families (parametric, sizes as parameter rows, standard and provenance) and drawn profiles | `.builtprofile` (JSON: sketch row with `use: profile`, or a family table) |
| Sections | drawn cutter sections | `.builtprofile` with `use: section` |
| Presets | presets grouped in packs | `.builtpreset` (one preset) or a pack folder with `pack.json` |

A document embeds every preset and profile it uses (`ext["built.presets"]`,
and sketch rows for drawn profiles) so the file opens anywhere. The
Library shows each as built-in, modified or local only.

### 2.8 Undo

A recipe edit and its re-evaluation are one History step:

- Row edits are journalled `Doc` ops (`Doc::begin`/`commit`, `Patch` for a
  field inside a row): diffs, coalesced within the step, as every document
  field is undone today.
- Generated definitions are store states and are covered like any geometry
  edit: the evaluation runs as a `SnapshotMutation` (`doc/snapshot_steps.rs`)
  per written definition, before/after states sharing unchanged
  4096-element chunks (`store/cow.rs`), with savepoints in `doc/geometry.rs`.
  A Repeat adding or removing copies uses the journalled placement ops.
- Undo and redo restore both; they never call the evaluator. A redo that
  re-evaluated would be the bug persistent-ids §4 already names (new ids).
- The `LiveCache` is keyed by content, so it stays valid across undo.

## 3. Evaluation

### 3.1 The dependency graph

Edges come only from references in rows; nobody draws them.

| Dependant | Depends on |
|---|---|
| sweep | its profile (sketch row revision, library profile key and parameters), its path references, its slot bindings, its cut rows |
| cut | its target's uncut result, its cutter (placement transform and definition revision, sketch row, bisector members), its anchor reference |
| repeat | its source (placement and recipe), its run references, the Repeat it runs between, its parent Repeat (nesting) |
| sketch | its plane anchor (only for its pose in the model; the 2D content has no inputs) |
| element reference | the referenced placement path's transforms and the definition revision of the element's mesh |

The graph is an index: `dependants: HashMap<Key, SmallVec<RowUid>>` where
a key is a row uid, a placement uid or a definition uid. It is rebuilt on
load (O(rows)) and patched when a row changes. It is never saved.

### 3.2 When it runs

Recipe evaluation is a pass at the end of each History step, beside the
succession and exact-rebind passes (`doc/history/dispatch.rs` `dispatch`,
after `lifecycle::execute` and before `succeed`, skipped for undo and redo
as succession is). Today a handful of relations re-evaluate ad hoc:
hosted openings (`hosted_openings::regenerate`, called from
`doc/transform_steps.rs` and `doc/group_steps.rs`), dimension anchors
(`refresh_dimensions`, `doc/history.rs`), structure resync and assembly
sync (`doc/assembly.rs`). The live pass replaces the first and sits beside
the others:

1. **Collect.** The step's change records give what moved or changed:
   placement transforms, definition revisions, recipe rows. The feed
   already computes these (`doc/feed/capture.rs`, core-protocol §4.1).
2. **Dirty.** Walk `dependants` from those keys. A placement move that
   only carries its cuts' cutters along (a cutter grouped with its
   target) dirties nothing; the whole assembly moved.
3. **Order.** Topological order over the dirty set (Kahn), ties by row
   creation order so the result is deterministic.
4. **Evaluate** each row (§3.3) against the step's state.
5. **Write** changed generated definitions into the same step. Placements
   added or removed by a Repeat are created or deleted in the same step.

A step that makes nothing dirty costs one hash lookup per changed key.

**Cycles are refused when they would be made**, by the tool, before the
step: a cut whose cutter depends on its target, a Repeat running between
its own copies. The tool's hover shows the refusal ("R-1 is cut by C-1
already: a mitre between them is one cut"). On load, a cycle (from a
merge or a hand-edited file) is broken by marking the newest row in it
`cycle`; its geometry stays as saved.

### 3.3 Per-kind evaluation

| Kind | Steps | Cost class |
|---|---|---|
| sweep | resolve path and slots; place the profile (attachment, rotation, mirror); build facets by extrusion along each span (straight spans: one prism; chains: mitre or butt joints via `store/sweep.rs` station projection); apply end cuts analytically (clip the prism by the end planes); apply body cuts (§ cut) | analytic, O(profile edges × spans) |
| cut, plane or bisector | clip the target's current solid by a half-space; planar result faces | analytic for prisms, O(faces) |
| cut, body or section | build the cutter solid (section: extrude the sketch region by the extent); boolean with the target through `solids/` | mesh boolean, O(faces of both) |
| repeat | stations from the run and rule; for each copy: identical → transform only; adaptive → bind slots, evaluate the recipe, hash, share | O(copies) transforms, plus unique recipe evaluations |
| sketch | regions from curves (planar arrangement); validity (closed, non-self-crossing) | O(curves log curves) |

**End zones.** A sweep's geometry splits into a start zone, a middle and
an end zone at the cuts' extents along the path. A cut confined to one
zone (a notch 20 mm from the end, an end plate) is evaluated on that zone
only, and the zone's result is cached by its local key. Copies of
different lengths then share their cut ends and differ only in the middle
prism, which is analytic. This is what keeps an adaptive bay of notched
purlins inside the budget.

### 3.4 Caching and budgets

- `LiveCache`: content key → generated mesh (and zone meshes), LRU,
  bounded at 64 MB, not saved. Keys are the same quantised hashes as §2.5.
- Copies are evaluated in parallel (rayon): evaluation is a pure function
  of its inputs. Writing into the store is serial.
- Drags (end grip, run grip, cutter grip) evaluate into the preview layer
  every frame and commit once on release.

| Budget (release build, M5 Max, measured on the ride model plus a garage frame) | Target |
|---|---|
| Re-evaluate one edited member (sweep + 4 cuts) and commit, with the ride open | < 16 ms, evaluation itself < 2 ms |
| 200-copy adaptive Repeat (notched purlins between non-parallel beams), recipe edit to committed state | < 100 ms |
| Same Repeat, drag preview per frame | < 16 ms (evaluation only; commit on release) |
| A step that dirties nothing, ride open | < 0.1 ms over today |
| Open a file with 10,000 recipe rows | stamps checked, nothing evaluated, < 50 ms over today |

The 16 ms member budget is dominated by the commit (store write, resident
pack, feed), not the sweep. Performance findings (2026-11-26/27) put a
200k-face commit and a far-model full pack per edit on the critical path;
generated definitions are small, so the commit must stay proportional to
the definitions the step wrote. Lane 0 measures this first.

### 3.5 Failure states

A failure never deletes or changes geometry the user made, and never
leaves a hole where an element was. The rule: **a failed recipe keeps its
last good result** and says why, in place.

| Failure | Shown | Geometry |
|---|---|---|
| `cut_misses`: the cutter no longer touches the target | cut row amber in the tree and record, "misses R-1" | target without that cut |
| `end_unresolved`: an anchored end has no intersection with its host | end grip amber, sweep row amber | last good length |
| `ref_lost`: a referenced element, placement or sketch is gone | row amber, "lost: R-2 web face", action "Pick again…" | last good result |
| `profile_open`: the sketch has an open loop or crosses itself | in the Sketch workspace: red ends, walkable list; users keep the last good profile | users unchanged |
| `boolean_failed`: the mesh boolean refused (non-manifold input) | cut row amber, "could not cut: R-1 is not solid" with Solid Inspector link | target without that cut |
| `cycle` | row amber, "depends on itself" | as saved |
| `stale`: geometry was changed by a reader without recipes (§3.6) | row grey, "detached: edited outside Built recipes", actions Re-evaluate / Release | as saved |

A copy of an adaptive Repeat that fails is marked on its own; the others
evaluate. Failures are toasts only when the user's own step caused them
("Mitre with R-2 no longer touches: kept the square end").

### 3.6 Stamps and readers without recipes

`placement.ext["built.live"].stamp` is a 64-bit hash of the generated
definition's content (ids and positions) written at evaluation. A format-2
reader keeps `ext` verbatim (built-format.md, Stability) but may edit the
generated geometry. On open, a mismatching stamp marks the row `stale` and
the geometry is left as found. The user chooses Re-evaluate (the recipe
wins) or Release (the geometry wins).

That makes recipe data non-critical: no `requires` entry, and a file stays
readable by every format-2 reader (open question Q1). Exact bindings on
generated geometry still set `built.exact_geometry.v1`, as any exact data
does.

## 4. Geometry and export

### 4.1 Facets with exact bindings

Generated geometry follows skp-style modelling: facets are the geometry,
segment counts are visible, Soften/Smooth as usual.

- A round profile part is a polygon with the profile's `segments`; the
  generated edges between its facets are soft and smooth, like a Circle
  pushed up.
- The generator writes exact bindings directly into `store/exact.rs`
  (`Curved`), no fitting: straight spans give planar faces (no binding
  needed) and cylinders or linear extrusions for round profile parts;
  circular paths give revolutions or tori; plane end cuts give planar
  faces whose curved edges are ellipses on the cylinders; a cylindrical
  cutter (a bore, a round cope) gives a cylinder support on the cut face.
  Body cuts by arbitrary faceted cutters stay faceted.
- Edges where a cut meets the sweep are hard.

### 4.2 IFC

The IFC writer (`formats/ifc/export.py`, `geometry.py`, pure Python)
writes faceted B-reps or polygonal face sets today, mapped items under the
`instancing` profile option, and openings with void and fill relations
from the record. It writes no extruded, swept or boolean solid yet: the
rows below marked *new* are lane 1 and lane 3 work. Per element (the
record's class, GlobalId and psets as today):

| Built | IFC4 (design transfer / default profile) | Reference View profile |
|---|---|---|
| Straight sweep, catalogue profile | *new* `IfcExtrudedAreaSolid` on `IfcIShapeProfileDef` / `IfcUShapeProfileDef` / `IfcRectangleHollowProfileDef` … with the catalogue's dimensions; `Axis` representation as an `IfcPolyline` | `IfcTriangulatedFaceSet` |
| Straight sweep, drawn profile | *new* `IfcExtrudedAreaSolid` on `IfcArbitraryClosedProfileDef` or `…WithVoids` (arcs as `IfcIndexedPolyCurve` arc segments) | tessellated |
| Chain or curved path | *new*; circular profile: `IfcSweptDiskSolid`; others: `IfcFixedReferenceSweptAreaSolid` on the directrix | tessellated |
| End cut (plane, bisector) | *new* `IfcBooleanClippingResult` (DIFFERENCE) with `IfcHalfSpaceSolid` | baked into the tessellation |
| Body cut, section cut (steel processing: cope, notch) | *new* `IfcBooleanResult` (DIFFERENCE) with the cutter as `IfcExtrudedAreaSolid` | baked |
| Opening preset (doors, windows, penetrations) | `IfcOpeningElement` + `IfcRelVoidsElement`, door/window `IfcRelFillsElement` (existing: `export.py` from `record.openings`; the opening's `faces` are now written from the cut row at export, so they follow a moved wall) | same, with tessellated bodies |
| Copies sharing a generated definition | `IfcRepresentationMap` + `IfcMappedItem` per occurrence (existing: `geometry.py` `mapped_body`, the `instancing` profile option; mirrored copies bake, as today) | same |
| Repeat | each copy its own element (own GlobalId); the Repeat is an `IfcElementAssembly` only when its record says so (pass-5 storyboard: portal frames) | same |

The record-driven rules hold: unknown class → proxy, record class wins,
openings keep their stored GUIDs. Exact CSG is written only where the
recipe says so and the target is a single solid; anything else falls back
to the tessellated body, with the reason in the export report.

### 4.3 STEP

Generated definitions export like any definition through the AP242 writer:
facets plus their exact bindings, so a straight member is planes and
cylinders and a mitred tube is two cylinders and two trimmed planes
(exact-geometry §1). Instances share one product definition.

### 4.4 Release

Release bakes, keeping everything the element is:

| Released | Becomes | Kept |
|---|---|---|
| Sweep | the placement keeps its uid, record and GUID; its generated definition becomes an ordinary one | element ids, exact bindings, material, tag |
| Cut ("Bake into target") | the cut applied, the row removed; a section cutter (sketch) disappears, a body cutter stays as its own object | as above |
| Repeat | copies become ordinary placements: identical and shared-geometry copies instances of one definition, others unique | uids (derived uids are frozen as ordinary ones), records, GUIDs |
| Sketch | nothing in the model changes; its users keep their profile as a frozen copy | — |

Release is one step and undoable. Released geometry never re-evaluates.

## 5. Interaction contracts

Each tool is a Rust tool machine on the existing framework (`trait Tool`,
`tools/mod.rs`): `ToolEvent` pointers arrive snapped by the inference engine
(`inference/`, `snap_engine.rs`), typed values arrive as `Value`/`Array`
events from the anchored field (`value_field.rs`), previews are
`tools/preview.rs` plus host-side previews, and every commit is one
`Command` run through `ToolHost::run` into `command_protocol.run` (one
History step). New commands are protocol verbs (`live_set`,
`sweep_place`, `cut_add`, `repeat_set`, `sketch_commit`, `release`), so
scripts, the agent bridge and tests use the same path. Keys are not routed
generically: each tool's adapter forwards the keys it owns (today only
Place Group forwards Tab, for walls), so the Sweep and Cut adapters forward
Tab, R, F, L and D while armed and the value field's `claims` keeps typing
safe. Each new tool regenerates `docs/tool-coverage.md`. The keys are the
pass-5 ones.

### 5.1 Sweep

| Contract | Mapping |
|---|---|
| Arm | rail tool `Sweep` (S); preset from the active node's recipe (Project Structure), else the last used |
| Chip | kit atom `cursor_chip`: *preset · profile · attachment · rotation*; wall presets speak wall words (material, side, height) |
| Points | Line's point picking and inference; chains like Line; double-click or Esc ends; anchored field: length, Tab to the angle |
| Keys while armed | Tab cycles attachment (nine plus named points; in the field Tab keeps its field meaning, the chip's Tab acts when the field is empty), R rotate 90° (typed angle in the field), F mirror, L picker, D draw a profile (Sketch workspace), Alt-click a face: profile from face |
| Selection on arm | edges selected: Enter sweeps each (members from lines); a connected chain under the cursor is offered as one path |
| Anchored ends | snapping an end to a face of another element records an anchored end (`to` that face), shown by an anchor glyph at the end; Alt while snapping keeps a plain point |
| Commit | one step: the placement, its record from the preset, the sweep row, filed under the active node |
| After | end grips (stretch with inference; on an anchored end the grip moves the offset), path grips on chain vertices, context menu: swap profile, edit profile (Sketch workspace), reverse, cut, repeat, save as preset, Release; record in the inspector with Preset, Profile, Section pad, Path, Material, IFC class, Mass |

### 5.2 Live cut

| Contract | Mapping |
|---|---|
| Arm | rail tool `Cut` (next to Sweep) |
| Pick | the target (Shift adds; a preselection skips the step) |
| Hover | candidates under the cursor: a face offers a plane, a joint offers the bisector, an object offers a body cut with clearance; ↓ cycles candidates; D draws a section on the hovered face or member end |
| Chip | *preset · operation · cutter* ("Mitre · Subtract · bisector of C-1 · R-1"); Tab cycles the operation; L opens presets and cutter kinds; the field holds the preset's number (gap, clearance, depth) |
| Anchor | the cutter is anchored to the element it was picked on (the face, the joint, the member end); Alt-hover another element while placing anchors to it instead (owner remark 1: the end plate anchored to R-1's web, cutting the purlin); the offset is the field value |
| Commit | one step: the cut row, the target re-evaluated |
| Display | the target's cutters drawn amber with grips only while the target is selected; View › Show All Cutters for review |
| Tree and record | a row under the target (amber scissors, kind text, no eye or IFC class); the record's Cuts list with an eye per cut; right-click: Edit, Select cutter, Disable, Bake into target, Remove |
| Push/Pull routing | on a cut face: edits that cut (plane offset, section depth); on an end cap of a sweep: edits the length; elsewhere on a recipe target: offers Edit profile or Release; on a plain group target: edits its uncut source and every cut re-applies (§5.6) |

### 5.3 Sketch workspace

A document context like group editing, with its own tool set:

| Contract | Mapping |
|---|---|
| Enter | Cut › D, Sweep › D, double-click a section cutter or a sweep's profile, Library › Profiles › New or Import; the camera state is saved |
| Layout (owner remark 2) | **left**: the reduced toolbar in the rail's place (Select · Line, Arc, Circle, Rectangle · Offset, Fillet, Trim, Extend · Dimension · Origin, Attachment points · Scale by edge, Merge arcs, Drop small fillets, Close gaps); **right**: the settings pane in the dock's place (the sketch's record: use, plane, extrude, depth, operation, users, units, view, Save to Library; the import problem list below it); canvas between; the 3D inset in the canvas's bottom-left corner |
| Top bar | breadcrumb (Garage › Purlins › P-1 › Notch N-1, back arrow leaves), "Sketch · 1:1 · mm", Cancel (ghost) and Done (primary, ↩) at the right |
| Context | neighbours ghosted and snappable (projected outlines of what the plane cuts), Isolated / Ghosted / Sliced, the live inset; holding the inset peeks the 3D view |
| Undo inside | a local history: ⌘Z steps through sketch edits; Done collapses them into one document step (the sketch row and every user re-evaluated); Cancel restores the row and offers Undo of the cancel |
| Esc | drops the active tool; a second Esc asks to leave (Done / Discard / Stay) |
| Constraints-lite | typed values in the anchored field, inference, driving dimensions: editing a dimension's value moves the free end of the entity it measures (the later-drawn one), one propagation step, no solver, no constraint colours |
| Import cleaning | DXF and SVG into a new sketch: layers listed, problems as a walkable list driven by one tolerance (merge segmented arcs, drop fillets under it, close gaps, unit doubt), Fix all as one local step; Scale by edge: click an edge, type its real length |
| Shared 2D kit | a new `draft2d` module (exact 2D lines, arcs, circles; trim, extend, fillet, offset, driving dimensions; snapping) used by the Sketch workspace and adopted by Sheets, whose drawing tools today are line, rect, ellipse, polygon and dimensions with no trim, extend, fillet or offset (`sheet_workspace/tools.rs`); the workspace shell follows Sheets' enter/leave (`sheet_workspace/workspace.rs` `Workspace::enter`/`leave`, the rail via `Ui::set_tools`) |

### 5.4 Repeat

| Contract | Mapping |
|---|---|
| Make | Move or Rotate with Ctrl, then the field: `4x`, `/4`, `1000`, `≤1200`; Tab cycles Count, Spacing, Divide, Fit ≤ and re-reads the number; Shift+wheel changes the count; the first array value turns the copy into a Repeat, a plain Ctrl-copy stays a copy |
| Anything | the selection is the source: a sweep, a group, a component, a cutter (then a repeated cutter), a Repeat (nesting) |
| Grips | the run grip on the last copy (inference; dropped on an edge it binds the run to that edge), one badge per nest level (×4, ×3) |
| Record | rule, ends, orientation, Alternate, skips, Adapt (shown when it changes something), "24 copies · 5 distinct parts", a breadcrumb through the nest, Edit source, Release |
| Skip | right-click a copy › Skip; stored by position (§2.2) |
| Edit source | double-click a copy: edits the source in place, the others follow live (identical: same definition; adaptive: the recipe) |
| Count change | Revit's question as a field option: keep the spacing or keep the ends (the rule decides by default) |

### 5.5 Presets

| Contract | Mapping |
|---|---|
| Choose | the chip starts with the preset name; L opens one searchable picker (enabled packs, This project, My presets, Recent); rows show profile, material swatch, IFC class |
| Manage | Library › Presets tab in the shared large sheet: packs, This project, Yours; detail with per-field locks, status, one primary "Use in Sweep" (or Cut, Repeat) |
| Capture | right-click an element › Save as preset… (popover): profile, attachment, material, IFC class, its cuts as cut presets, optionally record values; into This project, My presets or a shared folder |
| Defaults | the active node's recipe names a default preset |
| Locks | editing a fixed field on an element detaches it from the preset (a note in its record, "detached from Steel rafter: profile"); free fields are per element |
| Preset edit | an edit to a project preset re-evaluates every element that follows it (one step) |

### 5.6 Plain groups as cut targets

A cut on a plain group (not a recipe) must not destroy the user's
geometry. The group's definition becomes the **source** (kept, not shown)
and its displayed geometry a generated **result**. Double-click enters
the source with the cutters drawn amber; leaving re-cuts. Release, or
removing the last cut, makes the source the displayed definition again.
For a component, the source is shared by every instance; instances that
are cut identically share the result.

## 6. Migration and reuse

| Existing | Becomes |
|---|---|
| Hosted openings (`hosted_openings.rs`: the host must be a closed 6-face box; rectangular cuts in the wall's unit frame, `ext["opening_frame"]`; the fill's `ext["wall_host"]`; entries with world-space `faces` in `wall.ifc["openings"]`; re-cut when the fill is inserted, deleted, moved or flipped, not when the wall moves; refused with "Wall faces have changed" once the wall was edited) | the **Opening** cut preset: a door or window carries a body cut on its host (B's role inside A); the opening prism is the cutter, anchored to the door placement. Walls drawn before stay readable: on open, a hosted wall becomes a Wall-preset sweep with Opening cuts in memory, written in the new form at the next save; the reveal and overlap rules move into the Opening preset's evaluation. Gains: openings follow a moved or lengthened wall, walls need not stay boxes, and the IFC opening body is written from the cut at export instead of stored world faces |
| Wall prism regeneration in `hosted_openings.rs` | replaced by the sweep generator plus Opening cuts; its face matching that keeps keys is replaced by role ids |
| Move/Rotate arrays (`3x`, `/3`: `move_tool.rs` `array()` undoes the copy and re-runs it as `MoveCopiesCommand`; Rotate builds `InsertGroupCommand` + `RotateGroupCommand` per angle in `polar_history.rs`; all baked at once) | the same array window makes a Repeat row instead of copies (pass 4), and the field gains spacing (`1000`) and fit (`≤1200`) in `value_field.rs` `Accepts.arrays`; a plain Ctrl-copy stays a copy; old files have plain copies and nothing migrates |
| Follow Me and its sweep store (`store/sweep.rs`, `sweep_operation/`) | the Sweep generator reuses station projection, mitres and revolution detection; Follow Me stays a one-shot .skp tool |
| Solid tools (`solids/`: mesh booleans through Python `manifold3d`, every operand must pass `is_solid`, results merged back into polygon faces by `output.rs`) | the boolean engine of body and section cuts, called in process from Rust (lane 3 measures and moves the call off the Python path if it misses the budget); "Keep live" on a Solid tool later makes a cut row with that operation |
| Fillet and Chamfer (session presets) | stay one-shot; a later "edge treatment" cut kind could keep them live on generated edges by role id |
| ChangeAxes (`set_group_axes`, `ChangeAxesCommand`: a new frame, nothing moves) | on a recipe placement, changes the attachment and rotation of the sweep (the recipe owns the definition's frame); on a plain group, unchanged |
| Library › Profiles (`ProfilesProvider` is a stub today, `library_host.rs`), pass-4 profile catalogue | the steel packs' data |
| Project Structure node recipes | the default preset per node |
| Exact geometry bindings | written by the generator directly |
| Recording kit and frames | each lane ships its beats (§7) |

## 7. Implementation lanes

Each lane merges alone into `main` with Rust unit tests, Python protocol
tests, window tests and frames (lived-in state: full recents, a missing
linked file, a 1280×800 window). Lanes 1 and 2 run in parallel after lane
0; then 3, 4 and 5 in parallel; never more than four at once.

| Lane | Delivers | Depends on | Effort | Gates |
|---|---|---|---|---|
| 0 Live core | `LiveTable` (journalled rows, undo), references and their resolver, dependency index, the evaluation pass in `dispatch.rs`, role ids and the generated origin, stamps and `stale`, format-2 `ext` read/write, protocol commands `live_set`, `live_release`; a test-only recipe kind | — | M | undo/redo restores ids exactly; open with 10k rows < 50 ms; a no-op step < 0.1 ms; ride commit of one small generated definition < 16 ms |
| 1 Sweep | sweep rows, generator (straight, chain, mitre/butt, end zones), exact bindings, Library › Profiles with steel and timber families, the Sweep tool (chip, Tab/R/F/L, anchored ends, members from lines), grips, inspector record, IFC extruded solids and axis | 0 | L | member edit < 16 ms; IFC validated by IfcOpenShell; STEP reopen volume error < 1e−7 |
| 2 Sketch workspace | workspace shell (tools left, settings right), sketch rows, `draft2d` shared with Sheets, driving dimensions, attachment points, Save to Library, DXF/SVG/DWG import cleaning (the DXF reader keeps arcs and bulges as arcs instead of tessellating them; DWG through the existing LibreDWG bridge), profile-from-face | 0 | L | sketch Done re-evaluates users in one step; import fixtures (segmented arcs, gaps, inch file) |
| 3 Live cut | cut rows, plane/bisector/body/section cutters, anchors, end vs body roles, Cut tool, cutter display and grips, tree rows and Cuts list, Push/Pull routing, plain-group source/result, IFC clipping and openings; hosted openings migrated | 1, 2 (section cutters) | L | existing opening tests pass on the new path; a mitre and a cope survive move, resize and undo; cut misses keep geometry |
| 4 Repeat | repeat rows, rules, run grip and host binding, skips, nesting, Between, Alternate, repeated cutters, adaptive copies with geometry sharing, IFC mapped items | 1, 3 | L | 200-copy adaptive < 100 ms; copies' GlobalIds stable across exports; Release keeps uids |
| 5 Presets | preset rows and packs (Steel EN 10365, Steel AISC, Timber C16/C24, Architecture incl. Wall, MEP), picker, Library › Presets, Save as preset, locks and detach, node default | 1 | M | the storyboard beat built from presets only; a project preset edit updates its elements in one step |

The demo beat (pass-5 storyboard: portal frame, mitres, notch, purlins,
walls with a door) is the acceptance test for lanes 1–5 together.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Commit cost on a lived-in model dominates the 16 ms member budget | lane 0 measures on the ride first; generated definitions are small; the pack update must be per definition |
| Mesh booleans fail on near-coincident cutter faces (a cope flush with a flange) | clearance defaults > 0 in presets; end cuts analytic, not boolean; `boolean_failed` keeps the uncut result |
| Booleans go through Python `manifold3d` and re-tessellate both operands: too slow for 200 notched copies | end zones (§3.3) make one boolean per distinct cut end, not per copy; lane 3 benchmarks the call and moves it to an in-process Rust binding of the same library if needed |
| Role ids collide across a profile family change | role codes include the profile edge's stable id; families publish their edge names; a family change ends ids explicitly |
| Users edit generated geometry directly and lose the edit at re-evaluation | edits inside a generated definition are routed (§5.2) or refused with "Release to edit freely"; never silently overwritten |
| Adaptive Repeats explode the part count | the inspector says it ("5 distinct parts"); identical is the default where nothing differs |
| Anchors to faces of unclassified, re-drawn geometry are fragile | anchors prefer axis and datum references on recipe elements; element references are kept for plain geometry with `ref_lost` as the visible failure |

## 9. Open questions for the owner

| # | Question | Recommendation |
|---|---|---|
| Q1 | Should recipe data be a critical `requires` feature, so a reader without recipes refuses the file? | **No.** Save the evaluated geometry always, keep recipes in `ext`, detect outside edits with the stamp and mark the recipe `stale`. Every format-2 reader still opens every file. |
| Q2 | Generated faces keep their ids by role, so an id returns if its role returns (a removed cut re-added). Accept this exception to "ids are never reused"? | **Yes**, scoped to generated definitions: the id names a role, and a dimension or BCF pin on "the mitre face of R-1" should come back with the mitre. |
| Q3 | Adaptive or identical by default for a Repeat? | **Automatic**: adaptive only when the source has slots or external anchors (the purlins between beams), identical otherwise; the inspector shows the distinct part count and offers the switch where it matters. |
| Q4 | Default IFC body for cut members: exact CSG (`IfcBooleanClippingResult`, profile defs) or tessellated? | **Exact by default** in the IFC4 profile, tessellated in Reference View profiles, with the fallback reason in the export report. |
| Q5 | Walls drawn before: convert hosted-opening walls to Wall-preset sweeps on open? | **Yes, in memory on open**, written in the new form at the next save (files are never rewritten on open). Walls the converter cannot read as a rectangle sweep stay as today with their openings. |
| Q6 | DWG import into the Sketch workspace? | **Yes, through the existing LibreDWG bridge** (`dwg_bridge.rs`: DWG to DXF, then the DXF path), with the same cleaning list; no new reader. |

## Owner decisions (2026-11-27)

All six open questions accepted as recommended: recipes stay optional extension data with a content stamp (stale → Re-evaluate or Release); generated geometry keeps ids by role, including when a role returns; Repeat copies are adaptive automatically when the source is anchored, identical otherwise, with an inspector switch; cut members export as exact IFC4 solids (meshes only for Reference View, fallback reasons in the export report); existing hosted walls convert to Wall-preset sweeps on open and save in the new form; DWG enters the Sketch workspace through the LibreDWG bridge.

## Notes from lane 0 (feat/live-core)

- Evaluation runs in a hook immediately before the outer History transaction commits (`rust/built_core/src/doc/execute.rs`); the post-execute dispatch is too late to include generated writes in the same undo step. Generators must write there.
- A missing placement stamp on a surviving recipe row marks that row stale. A document whose whole `built.live` table was stripped by another reader has only baked geometry: there is nothing to mark, and it opens as plain geometry.

## Sweep and Sketch interchange (lanes 1 and 2)

The Library reads the version 1 `.builtprofile` envelope with `format: "built.profile"`, a name and a use. The bundled starter catalogue puts its sizes and millimetre dimensions in `table`. A drawn entry puts its sketch in `sketch`. Both enter the Sweep tool through its profile picker. The workspace can obtain the local profile folder from `ProfilesProvider.saved_directory()`. With an isolated settings directory this is `data/Built/profiles` below it. A captured drawing becomes a document sketch on the same undo step as its first member. Further members can reference that row by UID. Editing its curves through `live_set` updates those members together.

Sketch curves use the lane 2 IDs and shape fields. Coordinates and radii are millimetres. Arc angles are radians. Named points use `{name, at: [x, y]}` in the sketch frame. A sweep stores its attachment by name. Sweep rows retain the input path in the parent placement frame. Their placement marker stores the evaluated axis in definition coordinates for grips and IFC export.

The evaluated placement marker exposes `zones` per path span. Each span contains three signed metre intervals along its own axis, in start, middle, end order. Square ends have zero depth. A mitre or butt can extend past the picked station. The middle has the unchanged section. The Cut lane can expand the end intervals by its fitting extents and cache the resulting local cut geometry. Butt joints keep the manufacturing ends as distinct closed shells. Their evaluated planes are also recorded for exact IFC clipping.

During integration with the Sketch workspace, preserve its flattened sketch fields and `sketch_commit` command alongside the Sweep fields and `create_sweeps`. Keep the sketch UID in the Sweep dependency index. An open profile leaves its users' baked geometry in place. The sketch editor supplies the curve and point records described above.

## Preset interchange

Sweep stores the preset's display name in `preset`, its stable 16-hex ID in
`preset_uid` and its followed revision in `preset_rev`. Existing Sweep rows
with only a name keep their geometry and record. A document embeds its preset
rows in `ext["built.presets"] = {"v": 1, "rows": [...]}`. A pack file uses the
same envelope plus `"format": "built.presets"`. When importing or exporting a
pack folder, the Library reads or writes that envelope in `pack.json`. A single saved Library preset
is one JSON row in a `.builtpreset` file. Drawn profiles in a portable preset
carry their sketch under `values.profile.drawing`.

The command protocol accepts `preset_update(history, rows)` for a single
atomic pack edit, and `preset_delete(history, uid)` for an unused project
preset. The active node remembers `recipe.default_preset`. Cut and Repeat can
use the same preset row with `primitive` set to `cut` or `repeat` and their
own fields in `values`.

## Notes from lane 2 (feat/sketch-workspace)

Interfaces lanes 1 and 3 build on:

- **One kind of profile.** A drawn profile is a `built_draft2d::Sketch`
  (`rust/built_draft2d`, a pure Rust crate linked by `built_core` and
  `built_render`). The same JSON is a document's `sketch` row body and the
  `sketch` of a Library `.builtprofile` file (`built_draft2d::file::ProfileFile`:
  `{"format": "built.profile", "v": 1, "name", "use", "family"?,
  "provenance"?, "sketch" | "table"}`). Catalogue families fill `table`
  (lane 1's generator); a family size opened in the Sketch workspace becomes
  a drawn copy (follow-up: the families could emit sketches whose curve ids
  are their edge roles, so a size opened as a drawing keeps its roles).
  `tests/test_sketch_rows.py` sweeps a drawn 45 × 145 rectangle as a model
  sketch row, as a saved Library file and as the catalogue's
  `basic/timber`, and checks the three members are the same.
- **Regions.** `built_draft2d::profile(&sketch)` returns outer loops (CCW)
  with holes (CW), exact lines and arcs per edge with the curve id and
  direction, the net area and the problems (open ends, branches,
  crossings); the attachment points are
  `Sketch::attachment(name)` (nine bounding-box names, `origin`, named
  extras such as `shear_centre`), the default in `Sketch::attach`.
- **Rows.** `kind: "sketch"`, no owner, the body in lane 1's typed fields
  (`curves` and `SketchFields`: `use, name, plane, points, origin, attach,
  dims, extrude, next_id`). `live_set` and `sketch_commit` check the body
  with `built_draft2d` first; `size` and `open_top` are written only when
  they differ from the defaults. A sweep names its sketch with
  `profile.sketch`; the scheduler re-evaluates it in the step that commits
  the sketch. A sketch with open ends, branches or crossings is
  `profile_open` (draft2d), then `drawn_profile::outline` decides what a
  sweep needs (`multiple_regions`, `invalid_profile`); users of a failed
  sketch are left as they were (`unchanged`), or marked `profile_open`
  when they have no result yet. Releasing a sketch removes the row; each
  sweep that used it keeps a frozen copy (see the follow-up notes below).
- **Verbs.** `sketch_commit(row)` (protocol command; Done is one History
  step; a row without `uid` gets one), `sketch_profile(scene, uid)`
  (query), `command_protocol.sketch_plane(scene, face, path=None)` (plane
  frame, element reference and loops of a face) and
  `command_protocol.sketch_context(scene, origin, x, y, reach, limit)`
  (section lines, edges behind, inset edges).
- **Entering.** The window command `sketch:<how>[:<arg>]`
  (`rust/built_render/src/sketch_workspace.rs`): `new[:section]`, `face`
  (selection, or `<uid>/<uid>|f:<id>`), `section:<…>` (a cutter section on
  a face: Cut › D and a member end in lane 3 call this), `import:<path>`
  (DXF, SVG; DWG through `dwg_convert`), `open:<path.builtprofile>`
  (Library › Profiles › Edit), `edit:<uid>` (double-click a sweep's
  profile or a section cutter in lanes 1 and 3). The viewport menu on faces
  has Sketch on face. Save to Library writes the `.builtprofile` into
  `ProfilesProvider.saved_directory()` (`sketch_workspace.profiles_dir()`
  is the same folder), where the Profiles tab lists drawn profiles beside
  the families; sections are written there too and filtered out by `use`.

Files both lanes touch (resolved on the rebase onto lane 1):
`rust/built_core/src/live/{mod.rs,table.rs}` (the sketch checks in
`evaluate` and `live_set`, `sketch_commit`, the `unchanged` handling in
`run`), `rust/built_core/src/protocol/api/mod.rs` (`VERBS`),
`tests/fixtures/commands_modules_python.json`, `rust/built_render/src/ui.rs`
(the workspace hooks sit next to the Sheets ones), `rust/built_render/Cargo.toml`
and `rust/built_core/Cargo.toml` (the `built_draft2d` path dependency).
Sheets has not adopted `draft2d` yet (its tools keep their own geometry).

## Live cut interchange (lane 3)

A target row lists its cut row UIDs in `cuts`. Each cut has `kind: "cut"`,
the target placement in `owner`, an `operation`, and a `cutter` with one
of the keys `plane`, `bisector`, `body`, `section` or `opening`. A body
or bisector names a placement. A section names a sketch row. Plane points
are in definition metres. Anchor offsets, cope clearance and section depths
are in millimetres. An optional `anchor` binds a stable reference independently
of both the cutter and the target. Set `on` to false to suppress a cut without
losing its references. The first cut on a plain solid retains its uncut mesh
in the document. A copied placement gets its own cut rows.

The Cut tool takes a target and then a cutter. Shift-click toggles additional
targets before the cutter pick. Their cut rows share one undo step. Tab cycles
the operation, L opens preset search, A picks a separate anchor and D draws a section on the
hovered face. Select a target to see its cutters, or enable **Show All Cutters** in
View ▸ Sections or the Cut tool strip. Pick a grip and drag or type the offset
or plane angle. Release on the cut row in the project tree or record bakes the
stack through that row.

A fitting can specify a zero-based path `span` and its `start` or `end`.
Without a span the fitting applies to the corresponding outer end. The saved
`butt_planes` list carries both resolved planes for every span, in definition
coordinates, including the mitre planes of adjacent spans. Plane angle is in
degrees. Free plane and section offsets are in millimetres. For a cut with
an anchor the offset is stored on the anchor.

IFC4 Design Transfer exports analytic member end planes as clipping results.
It composes supported single-span sweeps and planar solids as CSG. Rounded
clearance or a curved operand without an IFC solid definition uses evaluated
faces and adds the reason to the export report. Subtractions carry a void
relationship. Hosted windows and doors reuse the opening GUID for their fill
relationship. STEP retains certified analytic trims, including the elliptical
boundary of a cylindrical member cut by an oblique plane.

## Notes from the sketch follow-ups (feat/sketch-followups)

- **Entering.** Library › Profiles has *New profile…* and *Import drawing…*
  in its header; the new Library › Sections tab (drawn sections: the
  Library's `use: section` files as *My library*, this model's section rows
  as *In this model*) has *New section…*; Enter or a double-click edits a
  section. The Sweep picker (L) ends with *Other sources*: Pick a face
  (Alt-click), Draw a profile… (D), Import DXF, DWG, SVG…. Draw › New
  Profile…, Draw › New Section… and File › Import › Drawing into a Sketch
  (`sketch:new`, `sketch:new:section`, `import:sketch`) are in the menu bar
  and command search. Import pickers go through `import:sketch` (and
  `import:sweep_sketch` from the picker); scripts and tests use
  `sketch:import:<path>`.
- **Sweep › D** (`sketch:sweep`): the workspace on a new profile; Done
  commits the sketch row (one step) and returns to the armed Sweep tool with
  it in the chip (the chip names a drawn profile by its name); Cancel
  returns too; a profile that is not closed yet is saved and Sweep keeps
  its profile, with a warning.
- **A member's profile** (`sketch:member[:<placement uid>]`): a double-click
  on a swept member's section face (an end cap, square or mitred:
  `record_queries::is_cap`) or right-click › *Edit profile*. A drawn member
  opens its sketch row (Done updates every user in one step); a catalogue
  size opens as an exact drawing (`profile_sketch(key)`: lines, one arc per
  root fillet, circles for round tubes) and Done runs
  `sketch_commit(row, adopt=[sweep row uid])`: the new sketch row and the
  member re-pointed to it in one History step.
- **Release of a sketch** gives each sweep that used it its own copy: a
  document-local sketch row with `frozen: true` (and no plane), the sweep's
  `profile.sketch` pointing to it, all in the release step. Users stay
  `ok`, never `profile_missing`; undo restores the shared row and the
  links. Frozen copies are not offered as project profiles (Library,
  Sweep picker); editing one keeps it frozen and moves only its member.
- **DWG.** `tests/fixtures/sketch/plate-mm.dwg` is `plate-mm.dxf` written by
  LibreDWG 0.14.8597 `dxf2dwg` (see `tests/fixtures/sketch/SOURCES.md`).
  Without `dwg2dxf` the import says so (info toast, no workspace); a
  converter failure is an error toast with LibreDWG's reason. The vendored
  `vendor/libredwg/bin/dwg2dxf` is a Linux binary; on a Mac checkout the
  converter comes from the release build or the PATH.
- **Rectangle.** Its opposite corner infers no axis from its first corner
  (that made a line on the red axis), and while values are typed the
  preview is the typed `w; h` rectangle towards the cursor's quadrant.
