# Junctions

Technical design for owner review, against `origin/main` at `08e172aa6`
(2026-11-27, Repeat merged). Design only: no product code lands with this
document.

The UX is settled: pass 6, option A (`ux-pass6/junction/options.md`), with
C's grouped "By arrangement" list as a view and B's double-click to edit
inside a junction. The owner's remarks and decisions after the boards are
binding:

1. **Hosting is a junction.** A door in a wall is the meeting of a host and a
   hosted element. The junction holds the host's Opening and the added parts
   (frame, lining, lintel, sill, fixings) sized from the host. Wall corners
   and wall-to-slab meetings are junctions too. IFC keeps
   `IfcRelVoidsElement` / `IfcRelFillsElement` for openings.
2. **Junctions are recursive.** A bolt on a plate is a junction with a
   cylindrical cut. A weld or a glue line is a junction between two parts. A
   knee contains a weld junction, bolt junctions (often a Repeat of one) and
   the end cut. Saving a detail captures the whole tree; a bolt group detail
   can be reused alone.
3. **Tree:** junctions under the members they connect, plus one project
   Junctions view.
4. **Tolerances** (angle, size ranges per role) are adjustable per detail;
   default angle ± 2°; near misses shown with one click to widen.
5. **Portable detail packs** for a future online library: stable ids,
   revisions, provenance, author, licence, units, dependencies on profiles
   and components, importable without the source model, each detail with its
   own adjustable values.

Binding principles (pass-5 and pass-6 briefs): general primitives with
trades as presets; no node graph; .skp-style interaction; facets are
geometry; every element filed with an IFC class and a record; never
auto-apply silently; Release bakes.

## 1. Summary

A junction is one new live recipe kind, `junction`, on the general-base core
(`built_core::live`). It adds no geometry kind of its own. It **owns**
ordinary rows and placements that already exist in the core:

| A junction holds | As | Exists today |
|---|---|---|
| Members by role (Main, Joining, Host, Hosted, Ply 1…n) | role bindings to placements (and their recipe rows) | references (general-base §2.3) |
| Cuts on each member | ordinary `cut` rows on the member, marked as owned by the junction | Live cut (lane 3) |
| Added parts (plates, cleats, bolts, screws, dowels, hangers, linings, lintels) | ordinary placements under the junction's own assembly, each a sweep, a fastener or a component | Sweep, components |
| Child junctions (bolts, welds, glue) | junction rows whose `parent` is this junction | new, same kind |
| A Repeat of a child junction (a bolt grid) | an ordinary `repeat` row whose source is the child junction's assembly | Repeat (lane 4) |
| Parameters, some *sized from* the members | named numbers with expressions, pushed into the owned rows' fields | new |
| The detail it follows, and per-use overrides | a preset of `primitive: "junction"` plus an override map | Presets (lane 5) |
| A status | `ok`, `outside`, `failed`, `lost`, `stale`, `cycle` | general-base §3.5 |

The meeting itself (where the members meet, at what angle, which faces) is
computed from the members' **stock** (path and profile before cuts), so the
junction can cut the very members it reads without a cycle (§3.1).

Meetings the user has not detailed are found by a **meeting index** over
member stock (never over faces), which also answers "7 similar" and the
review list (§4). A detail is a template written only in role-relative terms,
so it travels in a pack and applies in a model that has never seen its source
(§10).

## 2. Data model

### 2.1 Where junction data lives

| Data | In memory | In the file (format 2) |
|---|---|---|
| Junction rows | `LiveTable` rows, `kind: "junction"`, journalled like every row | `ext["built.live"].rows` on the document |
| Ownership of cut rows and part rows | a `junction` field on the owned row; index `owned: junction uid → set of row uids` rebuilt on load | the `junction` field on each owned row |
| The junction's assembly and its parts | ordinary placements; the assembly's `ext["built.live"]` marker names the row, as a Repeat owner's does | ordinary placements and geometry blocks |
| Details used by the document | `LiveTable.presets` rows with `primitive: "junction"` | `ext["built.presets"]` (embedded, as every preset) |
| Skips, Follow opt-ins | `JunctionPrefs` beside the table, journalled | `ext["built.junctions"] = {"v": 1, "skips": {detail uid: [meeting key]}, "follow": [detail uid]}` on the document |
| Meeting index, suggestions, ghosts | `MeetingIndex` and `Review`, not journalled | none (rebuilt) |
| Library details and packs | files | `.builtpreset` rows, pack folders (§10) |

The general-base rules hold unchanged: the file always carries the evaluated
geometry; opening never re-evaluates (stamps only); the record stays on the
placement; row uids are 16 hex for life.

### 2.2 The junction row

All rows share `{uid, kind, owner, state, v}`.

| Field | Value |
|---|---|
| `owner` | the junction assembly placement, uid `h64("built:junction:v1:<row uid>:owner")`, created beside the Main member (in its parent's children, as a Repeat owner is beside its source) |
| `meeting` | `end_face`, `end_end`, `crossing`, `corner`, `base`, `hosted`, or `parts` (a child junction between parts: weld, glue, bolt through plies) |
| `roles` | `{name: role binding}` (§2.3) |
| `parent` | the parent junction's row uid, for a child junction |
| `along` | for a junction applied along a Repeat (§2.8): `{"repeat": "<uid>", "end": "start" \| "end" \| "both", "skips": [station key], "stations": {station key: {"overrides": {…}}}}` |
| `detail` | `{"uid", "rev", "name"}` of the followed detail, or `null` for a custom junction |
| `params` | resolved values, `{key: number}`, written at evaluation (saved so the record shows them without evaluating) |
| `overrides` | `{key: number or expression}` set on this use only (the amber dot) |
| `local` | `{"removed": [content key], "added": [row uid]}`: content this use dropped from, or added to, its detail |
| `contents` | `[{"key", "row"? , "placement"?, "junction"?, "repeat"?}]`: each owned item by its template key |
| `bindings` | `[{"param", "row", "field"}]`: where each parameter's value is written (`field` is a JSON pointer into the owned row, `/sweep/profile/params/height`, `/anchor/offset`, `/rule/count`) |
| `frames` | the last resolved role frames (§2.3), 16 floats each, in the owner's frame, saved so markers and badges draw on open |
| `state`, `reason` | status and its structured reason (`{"role": "main", "criterion": "size", "value": 260, "limit": [152, 240]}`) |

Owned rows carry two new fields: `junction` (the owning row uid) and `key`
(the template key, `end_cut`, `plate`, `bolts`, `weld`). Owned items have
**derived uids**: `h64("built:junction:v1:<row uid>:<key>[:<role>]")` for rows
and placements, the scheme Repeat copies use (general-base §2.6). Applying the
same detail again, or re-expanding after a detail edit, gives the same uids,
so element ids, dimensions, BCF pins and IFC GlobalIds survive.

`LiveTable` gains `junctions: OrdSet<String>` and `owned: HashMap<String,
OrdSet<String>>`, maintained by `insert`/`remove` exactly as `repeats` and
`cut_targets` are today (`live/table.rs`).

### 2.3 Roles and role frames

A role binding:

```
{"placement": "<uid path>",          // the member occurrence
 "row": "<sweep or repeat row uid>", // when the member has a recipe
 "end": "start" | "end",             // for end_face, end_end, base
 "face": "<role id or element id>",  // the contact face, when it matters
 "station": "<station key>",         // a Repeat copy (§2.8)
 "attached": false}                  // true: the junction places this member (hosted, §8.1)
```

Role names come from the detail: `main` and `joining` for two members, more
for three or more (`main`, `joining`, `brace`), `host` and `hosted`,
`ply_1`…`ply_n` for a bolt stack, `a` and `b` for a weld. The main role is
the IFC Relating element and the member whose face the others meet.

Owned rows never name a member directly. They anchor to **role frames**,
computed by the junction at each evaluation from the members' stock:

| Frame | Origin and axes |
|---|---|
| `meet` | the meeting point (end_face: the Joining axis ∩ the Main contact plane; end_end: the mid point of the two ends; crossing: the mid point of the axes' closest points; base: the axis ∩ the slab top; hosted: the hosted placement's origin projected onto the host's reference face). Z = contact normal, out of Main towards Joining; X = Joining's axis projected onto the contact plane (Main's axis when they are parallel); Y = Z × X |
| `<role>.end` | the role's stock axis frame at its meeting end, X pointing out of the member |
| `<role>.at` | the role's axis frame at the station closest to the meeting (crossing, hosted) |
| `<role>.contact` | the plane of the role's face that meets the other role: a profile edge role (`flange_outer`, `web_left`, `top`) found by facing, or the host's face |
| `<role>.<point>` | a profile datum at the meeting station: the nine attachment points, `origin`, named points such as `shear_centre` |

Frames are exposed through one new reference kind, resolved by
`live/reference.rs` like the others:

```
{"junction": "<row uid>", "frame": "meet" | "joining.end" | "main.contact" | …}
```

so an owned cut's `anchor`, a plate's anchored `ends`, or a fastener's pose
is written once against a frame and follows every move. A **mirrored**
meeting (the right-hand knee of a detail captured on the left) resolves the
same frames with Y negated, and the contents are placed in that frame;
direction of modelling is ignored (the Advance Steel lesson): a Joining member
drawn the other way binds its other end.

**Role dimensions** for expressions come from the member's profile or record:

| Member | Dimensions |
|---|---|
| Sweep, catalogue profile | the family parameters as stored (`width`, `height`, `web`, `flange`, `radius`; `live/profile.rs`), plus `length` |
| Sweep, drawn profile | `width`, `height` of the profile's bounding box, `area` |
| Wall (Wall preset sweep) | `thickness` (= `width`), `height`, layer thicknesses when the record has them |
| Slab, plate, plain classified group | `thickness`, `width`, `length` from its oriented box |
| Fastener part | `d`, `length`, `head`, `washer` (§2.7) |
| Any role | `angle` (between this role's axis and Main's, degrees), `skew`, `slope` |

### 2.4 Contents and ownership

Ownership is the only new relation; everything owned stays an ordinary row or
placement and evaluates with its kind's own generator.

| Content | Owned as | Anchored to |
|---|---|---|
| End cut, cope, notch, birdsmouth, tenon, mortise, half-lap, mitre | a `cut` row on the member (`owner` = member placement, listed in the member's `cuts`) with `junction` and `key` set | a role frame (`anchor.ref = {"junction", "frame"}`) or a cutter in the junction's assembly |
| Hole (a bolt's, a dowel's) | a `cut` row with the new `hole` cutter (§2.7) on each ply | the fastener's frame |
| Plate, cleat, stiffener, lintel, sill, lining | a placement under the junction assembly with its own `sweep` row (owner = that placement) | `ends` / path points anchored to frames |
| Fastener (bolt, screw, dowel, nail, anchor) | a placement of a generated, shared fastener definition (§2.7) | the child junction's frame |
| Component part (hanger, bracket, flange, coupler) | a placement of a Library component under the assembly | a frame; the junction writes its transform |
| Child junction | a junction row with `parent` | its own roles, bound to members and to the parent's parts |
| Repeat of a child junction | a `repeat` row whose source is the child junction's assembly | the run is anchored to a frame |

Rules, checked by `live_set` and the junction verbs:

- An owned row may be edited, but its bound fields (those in `bindings`) are
  written only by its junction. Editing one by hand is an override on the
  junction, routed by the inspector, never a silent detach.
- Deleting a member: the junction keeps its last good parts, state `lost`
  (§3.3). Deleting the junction removes everything it owns in the same step.
  Deleting an owned part from the viewport removes its content key from this
  use (`local.removed`), with a toast offering Undo and "Remove from the
  detail".
- A row has at most one owning junction. A cut made inside a junction context
  (§6.3), or dragged onto a junction row, becomes owned (`junction_adopt`).
- Cycles of ownership (a junction owning its own parent) are refused by the
  verb.

### 2.5 Parameters and *sized from* expressions

A parameter:

```
{"key": "plate_t", "name": "Plate thickness", "group": "Plate",
 "expr": "15", "unit": "mm" | "deg" | "count",
 "limits": {"range": [8, 40]} | {"list": [10, 12, 15, 20, 25]},
 "prompt": "Thickness of the end plate", "lock": "free" | "fixed", "hidden": false}
```

`expr` is a small pure expression, evaluated in Rust (one Pratt parser in a
new `live/expr.rs`, no host language, nothing executable):

| Grammar | |
|---|---|
| Literals | numbers in the parameter's unit |
| Names | other parameters by key; role dimensions as `role.dim` (`joining.width`, `main.flange`, `host.thickness`, `ply_1.thickness`) |
| Operators | `+ − × ÷` (`* /` accepted), parentheses, unary minus |
| Functions | `min`, `max`, `abs`, `floor(x, step)`, `ceil(x, step)`, `round(x, step)`, `clamp(x, lo, hi)`, `pick(list, x)` (the smallest list value ≥ x: standard bolt lengths), `sum(role.dim over plies)` |
| Results | lengths and angles as f64; `count` parameters rounded to integers ≥ 0; a value outside `limits` is clamped and the junction reports it (`failed: limit`) |

Parameters are ordered by dependency when the detail is saved; a cycle is
refused there ("Bolt pitch depends on itself through Bolt rows"). Examples
from the starter set:

```
plate_width  = joining.width + 30
plate_height = joining.height + 2 * edge + 20
bolt_rows    = max(2, floor((joining.height - 2 * edge) / pitch, 1) + 1)
bolt_length  = pick([40, 45, 50, 55, 60, 65, 70, 80], sum(ply.thickness) + nut + washer + 2 * pitch_thread)
frame_depth  = host.thickness          // a door lining
lintel_len   = hosted.width + 2 * bearing
```

**Discovery at capture** (§5) turns every number of the captured items into a
parameter named after its field: an anchor offset becomes Gap, a plate
profile's `height` Plate height, a Repeat rule's count Bolt rows, a hole's
clearance Clearance. A *sized from* relation is proposed when the value equals
a dimension of a role the item is anchored to, plus a constant that is 0 or a
multiple of 5 mm with |constant| ≤ half that dimension, or that dimension × ½,
1 or 2. Exact equality ranks first, then the smallest constant; alternatives
are listed in the capture popover. Equal values from the same preset field in
several items merge into one parameter (four bolts' Ø 16 → one Bolt Ø).

### 2.6 The detail: a preset of primitive `junction`

A detail is a preset row (general-base §2.2, lane 5) with `primitive:
"junction"`. Its `values` are a **template** written only in role-relative
terms, so it is model-independent by construction:

```
{"meeting": "end_face",
 "roles":  {"main":    {"name": "Column", "prompt": "The member whose flange is met"},
            "joining": {"name": "Rafter"}},
 "params": [ … §2.5 … ],
 "contents": [
   {"key": "end_cut", "on": "joining",
    "row": {"kind": "cut", "operation": "trim", "end": "@joining.end",
            "cutter": {"plane": {}}, "anchor": {"ref": "@main.contact", "offset": "gap"}}},
   {"key": "plate", "part": {"preset": "<Plate preset uid>",
            "fields": {"profile.params.width": "plate_width", "profile.params.height": "plate_height",
                       "profile.params.flange": "plate_t"},
            "at": {"frame": "@meet", "offset": [0, 0, "gap"]}}},
   {"key": "bolts", "junction": {"detail": "<Bolt M16 8.8 uid>", "roles": {"ply_1": "@plate", "ply_2": "@main"},
            "params": {"d": "bolt_d"},
            "repeat": {"rule": {"count": "bolt_rows", "step": "pitch"}, "run": "@meet.y", "nest": {"count": 2, "step": "gauge"}}}},
   {"key": "weld", "junction": {"detail": "<Fillet weld uid>", "roles": {"a": "@joining", "b": "@plate"},
            "params": {"throat": "weld_a"}}}],
 "rule": { … below … }}
```

`@` marks a symbolic reference (a role, a frame, another content key); a
quoted name in a numeric field is a parameter expression. Expansion (§3.2)
replaces them with references to the junction's frames and its derived uids.

**The match rule**, with per-detail tolerances (owner decision 4):

```
"rule": {
  "meeting": "end_face",
  "roles": {
    "main":    {"class": ["IfcColumn", "IfcBeam", "IfcMember"],
                "profile": [{"family": "i", "library": "steel/EN 10365/HEA"}, {"library": "steel/EN 10365/HEB"}],
                "size": {"height": [152, 240]}, "face": ["flange_outer"]},
    "joining": {"profile": [{"library": "steel/EN 10365/IPE"}], "size": {"height": [200, 300]}, "end": "any"}},
  "angle":  {"value": 75, "tol": 2},
  "mirror": "allow",
  "direction": "ignore",
  "offset": {"tol_mm": 5}}
```

| Criterion | Matches when |
|---|---|
| `meeting` | the meeting kind is the same (corner and end_end are distinct; a hosted rule never matches a structural meeting) |
| `class` | the role's record class is in the list (empty: any) |
| `profile` | the role's profile library key, or family, is in the list; a drawn profile matches only by `size` |
| `size` | every listed dimension is inside its range (mm) |
| `face` | the contact face role on Main (`flange_outer`, `web`…); a Main binding on a flange names it, so bolt grips measure through it |
| `end` | which end meets (`start`, `end`, `any`), checked only with `direction: "exact"`: a member drawn the other way binds its other end |
| `angle` | the angle between Joining and Main (or the rule's `about` pair) is within `value ± tol`; default tol 2° (owner decision 4) |
| `mirror` | `allow` (default) or `exact`; with `exact`, the meeting's hand must equal `hand` |
| `hand` | the source meeting's mirror class (±1, the side of Main's section the other comes from), written at capture; a meeting of the other hand is "mirrored" |
| `arrangement` | for `corner` rules: `tee` matches an end on a run; without it a corner rule matches L corners only |
| `offset` | the Joining axis passes within `tol_mm` of where the detail expects it (an eccentric beam does not match a centred detail) |

A rule is evaluated by the meeting index (§4.3); a near miss is a meeting that
fails exactly one criterion, reported with the value that would include it.
`offset` without `value` expects 0 mm. On a symmetric meeting (end to end,
a crossing at mid spans, an L corner) equal fits take the thicker member,
then the older one, as Main.

### 2.7 Recursion: fasteners, holes, welds, glue

**A bolt junction** (owner remark 2) is a child junction of meeting `parts`:

- roles `ply_1`…`ply_n`, the parts or members it clamps, in order;
- one fastener part: the bolt, with nut and washers;
- one `hole` cut row on each ply;
- parameters `d`, `clearance` (hole Ø = d + clearance), `length` sized from
  the grip, `grade`, `standard`.

**The `hole` cutter** is a new cutter kind on the existing cut row:

```
"cutter": {"hole": {"d": mm, "depth": "through" | mm, "segments": 24,
                    "countersink": null | {"d": mm, "angle": 90}}},
"anchor": {"ref": {"junction": "<bolt junction>", "frame": "meet"}}
```

It exists, rather than a body cutter, because holes are the hot path (128
holes in a garage) and the export vocabulary (IFC `IfcVoidingFeature` HOLE,
BTLx Drilling, schedules count them). When the hole axis is normal to two
parallel planar faces of the ply and the hole stays inside one face region,
the evaluator builds the hole analytically: the bore faces with an exact
cylinder binding (general-base §4.1) and two new inner rings, no boolean.
Otherwise it falls back to the body boolean path (`solids/`). This mirrors the
Repeat notch fast path (`live/repeat/notch.rs`).

**Fasteners are generated, shared definitions.** A fastener table
(`resources/fasteners/*.builtfastener`, JSON like a `.builtprofile` family
table: ISO 4014/4017 bolts, ISO 4032 nuts, ISO 7089 washers, wood screws,
dowels, nails, anchors) gives head, shank and nut dimensions per size. A
fastener part's geometry is generated from (standard, d, length, stack) with
the profile generator's revolution and prism code, keyed by that tuple, and
every bolt of the same key instances one definition. 128 bolts of two sizes
are two definitions. Each bolt is still its own placement (own uid, own IFC
GlobalId, countable, pinnable).

**A Repeat of a bolt junction** is an ordinary `repeat` row whose source is
the bolt junction's assembly. Its copies are adaptive (general-base §2.5: the
source has external references, its plies). The hole cut row on each ply
keeps its hole specification and stores the Repeat uid in `cutter.hole.repeat`.
The evaluator drills the source and each station. A 2 × 2 grid cuts the plate
and the flange with one row each. Copy bolts take derived uids from the Repeat row
and station (general-base §2.6). In the tree: "Bolts M16 × 4 (Repeat)".

**A weld junction** is a child junction of meeting `parts`, roles `a` and
`b`:

- one weld part: a placement with record `IfcFastener` WELD whose geometry is
  either edges only (the weld path, drawn as a line with the weld symbol
  overlay: the default) or a bead (a triangle profile of throat × √2 swept
  along the path, a sweep row with `segments`), chosen by the detail;
- the path is where `a` meets `b`, computed from stock: `a`'s profile
  outline in `b`'s contact plane (for an end plate, the Joining section at
  the end cut plane). It needs no evaluated faces, so a preparation cut on
  `a` cannot loop. The run is all-round, one side or both sides;
- parameters `throat`, `length` (derived, read-only), `type` (fillet, butt),
  `process`;
- optionally a preparation cut (a bevel) on `a`.

**A glue junction** is the same with `IfcFastener` GLUE, a contact area
instead of a length, and no geometry by default.

**Depth.** Child junctions nest to depth 8; a ninth level is refused by the
verb. A child junction is matched and reusable alone: a bolt group detail has
roles `ply_1`, `ply_2` and appears in the Junction tool's list wherever two
plies meet.

### 2.8 Junctions along a Repeat

Two different cases:

| Case | Data | Copies |
|---|---|---|
| The junction's roles are all inside the Repeat's source (a truss with its gusset junctions) | nothing new: the junction is part of the source; the Repeat's copies are identical where the source is self-contained | instances of the source, parts included |
| The junction joins each copy to something outside it (each purlin end on the rafter it meets) | one junction row with `along: {"repeat", "end"}` | each station resolves its own meeting |

The second is the storyboard's purlin cleats: "the junction joins the Purlins
recipe: 48 cleats, and new copies get theirs". One row, like a repeated
cutter:

- Roles: `joining` is `{"row": "<repeat uid>", "station": "*"}`; `main` is the
  copy's bound slot at that end (`start_host`, `end_host`, general-base §2.4).
- Each station is a meeting; its parts sit in a per-station assembly
  `h64("built:junction:v1:<row>:<station key>:owner")` under the row's
  owner, so IFC writes one `IfcElementAssembly` per meeting (§9).
- Cuts on the copies (cleat holes in the purlin web) are fed into the Repeat's
  copy evaluation like the source's own cuts, with the copy's frames bound.
  Cuts on the main members (holes in each rafter's flange) are one owned cut
  row per main member with a repeated `hole` cutter listing that member's
  stations.
- Skips and per-station overrides are stored by station key, so they stay
  with their purlin when the count changes, as Repeat skips do.
- A new copy gets its junction in the same step; a removed copy's parts go.

### 2.9 Undo

Unchanged from general-base §2.8. A junction verb, its expansion and its
evaluation are one History step: row edits are journalled ops, generated and
owned definitions are store snapshots, placements added or removed are
journalled placement ops. Undo and redo never evaluate.

## 3. Evaluation

### 3.1 Dependency order: stock before result

A junction reads its members and cuts them. Today that is a cycle: an `Axis`
reference depends on the whole sweep row (`Key::Row`), and
`reference::origin` resolves it from the marker `path` the sweep wrote, only
when that row's state is `ok` (`live/reference.rs`). The member's row in turn
depends on its cut rows. A junction cut on R-1 anchored to R-1's own axis would
loop, and one failed hole on a column would make every junction on that
column `ref_lost`.

Lane J0 splits each sweep and Repeat into two schedule nodes:

| Node | Depends on | Produces |
|---|---|---|
| `stock(R-1)` | path references, anchored ends' hosts, profile, slots | the axis, the profile placement, the role frames' inputs, a stock state |
| `result(R-1)` | `stock(R-1)`, its cut rows | the generated definition |
| `stations(Repeat)` | source stock, run, rule | copy poses and slot bindings |
| `result(Repeat)` | `stations`, the source recipe, along-junction cuts | copies' geometry |

`Key` gains `Stock(row uid)`. Axis, datum and role-frame references resolve
against stock; element references (a face id) still resolve against the
result. A junction depends on its roles' stock. For a knee the order is:

```
stock(C-1), stock(R-1)
  → junction J-1                     (frames, parameters, bindings written)
    → cut rows owned by J-1          (end cut on R-1, repeated holes on C-1 and PL-1)
    → sweep(PL-1) stock and result   (sized from R-1)
    → child junctions (bolts, weld)  → their Repeat, fasteners, weld path
  → result(C-1), result(R-1)         (with their junction cuts)
```

The scheduler (`LiveTable::schedule`, Kahn over the dirty set, ties by
creation order) is unchanged in shape. Two additions:

- **Early cut-off.** A node whose output hash did not change (frames and
  parameters quantised to 1 µm and 1 µrad, as general-base §2.5) does not
  dirty its users unless they are seeds themselves. Moving a purlin then
  re-evaluates its cleat and the rafter's hole row, never the rafter's other
  junctions.
- **Junction writes.** A junction patches its owned rows' bound fields before
  they evaluate (they come after it in the order). It may write only those
  fields, and only into rows it owns; `live_set` validates this once, so the
  pass never changes graph structure. Structure changes (a content added, a
  detail re-expanded) happen in the verb, before the pass.

Cycles that would be made are refused by the tool before the step ("R-1 is
the Main of J-3 already"), as general-base §3.2 says; on load the newest row
in a cycle is marked `cycle`.

### 3.2 Expansion and evaluation of one junction

**Expansion** (in `junction_apply`, `junction_set` and on a detail edit):

1. Bind roles from the chosen meeting (or the user's picks).
2. For each template content not in `local.removed`: derive its uid, build the
   owned row or placement from the template, replacing `@` references with
   `{"junction", "frame"}` references and derived uids; add a binding for
   every numeric field that holds an expression.
3. Diff against the current owned set by content key: keep matching uids,
   insert new ones, remove gone ones. Rows in `local.added` are kept.
4. Recurse into child junction templates.

**Evaluation** (in the pass, per junction, in dependency order):

1. Resolve role stock; any role missing → `lost` (keep everything, stop).
2. Test the rule against the meeting; failing → `outside` (§3.3).
3. Compute frames; for an `along` junction, per station.
4. Evaluate parameters in dependency order, then overrides.
5. Write changed bound fields into owned rows and transforms of non-recipe
   parts (fasteners, components, attached hosted roles). Unchanged values
   write nothing.
6. Owned rows evaluate after it with their own generators. Their failures
   come back to the junction as `failed` with the child's reason.

### 3.3 Status and last good parts

A junction never deletes or moves what it cannot resolve; it keeps its last
good parts and says why, on the marker, the tree row and the record (Tekla's
green, yellow, red, without the red: nothing is ever not created once it
was).

| State | Cause | Parts and cuts | Ways out |
|---|---|---|---|
| `ok` | resolves, fits its rule | follow | — |
| `outside` | a role changed outside the rule (HEA 260 on a 152–240 detail, angle 79° on 75 ± 2°) | parameters frozen at the last good values; frames still followed while they resolve, so a moved column carries its plate | Another detail (list of those that fit), Widen the detail (one click, the exact change), Keep as custom |
| `failed` | an owned row failed (a cut misses, a hole leaves the plate, a limit was clamped) | the failing item keeps its last good result (general-base §3.5); the others follow | the child's own action, shown in the record |
| `lost` | a role's member is gone, or its stock no longer resolves | as saved | Pick again…, Release, Delete |
| `stale` | geometry edited by a reader without recipes (stamp mismatch) | as found | Re-evaluate, Release |
| `cycle` | from a merge or a hand-edited file | as saved | Remove the dependency |

`outside` is amber, `failed` amber with the child named, `lost` amber with a
dashed marker. Toasts appear only when the user's own step caused the change
("J-4 no longer fits Knee end plate: HEA 260 · kept the plate").

### 3.4 Budgets

Release build, M5 Max, measured on the ride model plus the garage beat (8
knees, 24 purlins × 2 cleats), and a synthetic steel hall (2,000 members,
4,000 meetings) as the lived-in large case:

| Budget | Target |
|---|---|
| Evaluate one knee (frames, parameters, end cut, plate, 4 holes on 2 plies, 4 bolts, weld), evaluation only | < 2 ms |
| Move a column carrying two knees and commit, ride open | < 16 ms (general-base member budget) |
| Detail edit across 8 knees, to committed state | < 50 ms |
| Detail edit of the purlin cleat along the Repeat (48 meetings) | < 100 ms |
| Detail edit across 1,000 junctions (the hall) | < 500 ms, with a progress toast past 200 ms |
| A step that touches no junction input, ride open | < 0.1 ms over today |
| Open a file with 5,000 junction rows | stamps checked, nothing evaluated, < 30 ms over today |

Hole fast path, fastener definition sharing and early cut-off are what keep
these; lane J0 and J1 measure them first.

## 4. Meetings and similarity

### 4.1 What is a member

The meeting index never reads faces. It reads, per element:

| Element | Stock used |
|---|---|
| Sweep row (members, walls, plates) | the stock axis (polyline) and the profile's envelope (bounding rectangle in the profile plane, rotated) |
| Repeat copy | the same, per station, from `stations(Repeat)` |
| Plain group or component with a structural or hosting record class (`IfcBeam`, `IfcColumn`, `IfcMember`, `IfcPlate`, `IfcWall`, `IfcSlab`, `IfcFooting`, `IfcRoof`, `IfcDoor`, `IfcWindow`, …) | its oriented box, cached per definition uid and revision; an axis when the longest extent is ≥ 3 × the others |
| Anything else (unclassified imported geometry: most of the ride) | nothing; skipped at no cost |

This is what keeps it fast on a lived-in model: the ride's millions of faces
are never visited, and a classified imported member costs one cached box.

### 4.2 Meeting detection and the index

A broad phase over capsules (axis segment inflated by the envelope's half
diagonal plus 5 mm) in a uniform grid sized from the median member length
gives candidate pairs. A classifier labels each pair:

| Kind | Test |
|---|---|
| `end_face` | an end of B lies within tolerance of A's envelope surface, B's axis is at ≥ 10° to A's; main = A |
| `end_end` | two ends within the larger envelope radius; mitre or apex when the axes are at < 175°, splice when nearly collinear |
| `crossing` | the axes' closest points are interior to both (outside their end zones) and within the sum of half depths + tolerance: purlin on rafter (stacked), half-lap (interpenetrating). A **seated end** (a purlin end bearing on a rafter's top flange, its axis along the face) is a crossing with the end named, because `end_face`'s frame needs the axis to pierce the face; two ends that both sit on one member meet that member, not each other |
| `corner` | `end_end` or `end_face` between two planar members (walls: height ≫ thickness): an L (two ends) or a tee (an end on a run) |
| `base` | an end of a member, or a wall's bottom face, on the top face of a slab or footing |
| `hosted` | an existing host relation (an Opening cut, a `wall_host` marker), or a hostable component (door, window, sleeve) whose box penetrates the host's thickness |
| `parts` | not detected model-wide: found only inside a junction's own parts (plies under a bolt, a plate on a member) |

Each meeting gets:

- a **key**, stable while its members exist:
  `h64("built:meeting:v1:<kind>:" + sorted "<uid path>|<tag>")`, the tags
  `start`, `end`, `side`, `mid`, `top`, `bottom`, `host`, `hosted`. The
  contact face is not in the key, so a skip survives a rotated section.
  Skips and Follow are stored by this key.
- a **signature** for grouping and matching, independent of modelling
  direction: `(kind, canonical role descriptors, angle bucket, mirror
  class)`, where a role descriptor is (class, profile library key or family,
  size key, contact face role, end).

The index (`live/meetings.rs`, `MeetingIndex`) is built lazily the first
time something asks (the capture chip, the Junction tool, the Junctions view,
a review), then kept up to date from the same change records the live pass
collects: a member whose stock changed is removed and re-queried against the
grid. It is never saved and never journalled.

| Budget (same machine and models) | Target |
|---|---|
| Build the index, garage + ride open | < 5 ms (the ride contributes only its classified boxes) |
| Build the index, steel hall (2,000 members) | < 20 ms, on a worker thread |
| Update after a step that changed one member's stock | < 0.5 ms, off the History step |
| Memory, steel hall | < 2 MB |

### 4.3 Similarity search

Matching a detail's rule against the index:

1. Bucket meetings by `(kind, main profile key or class, joining profile key
   or class)`; the rule names its buckets, so a search touches only those.
2. Test size ranges, angle, face and end, mirror and offset on each
   candidate; bind roles (swapping when the rule's Main is the candidate's
   B).
3. Classify each result as **match**, **near miss** (exactly one criterion
   fails: the criterion, the value, the limit and the widened value that
   would include it), or **already detailed** (has a junction of another
   detail: listed, unticked, never replaced by default: Tekla's "existing
   connections are not modified").

| Budget | Target |
|---|---|
| Search one detail, steel hall (4,000 meetings) | < 10 ms, on a worker thread |
| Search all project details after open (for the Junctions view counts) | < 50 ms |
| The capture chip's live "· 7 similar" count while editing the rule | < 5 ms per change |

Searches run after a detail is saved or edited, when the Junction tool arms,
and when the Junctions view opens. They never run inside a History step and
never write the document.

### 4.4 Suggestion and review data

```
Suggestion { detail: (uid, rev), meeting: key, roles: binding map,
             mirrored: bool, fit: Match | Near { criterion, value, limit, widen_to },
             existing: Option<junction uid>, why: [reason atoms] }
Review     { detail, suggestions, ticked: bitset, ghosts: LRU<meeting key, preview> }
```

- `why` atoms render the review list's "mirrored" (the meeting's hand is not
  the rule's `hand`), "sized differently: IPE 270" (only when the rule names
  one catalogue size), "angle 74.2°".
- **Ghosts** are the junction evaluated into the preview layer
  (`tools/preview.rs`), not committed. The 32 nearest visible ones are built
  eagerly; the rest when they scroll into view or are hovered.
- **Apply** is one verb, `junction_apply(detail, [binding])`, one History
  step, one undo entry; only ticked rows go.
- **Skip** writes the meeting key to `ext["built.junctions"].skips[detail]`
  (journalled, undoable). Skipped meetings are shown, greyed, under "Skipped
  (1)" with Un-skip.
- **Follow** (from option C, opt-in per detail): when on, new matching
  meetings are listed with a badge on the detail's row; they are never
  applied without the user.
- **Widen** on a near miss is a detail edit (a new revision), with the count
  of uses it affects shown before the click.

## 5. Capture

**When the chip appears.** After a step, the live pass knows which rows were
written. When two or more items are anchored to the same pair of members and
no junction owns them, the meeting gets the quiet chip "Save as junction · N
similar" (overlay mark, `tools/overlay_marks.rs`). It fades after 8 s or
when the user works elsewhere, and stays in each item's context menu.

**Gathering** (`junction_capture(seed)`), from a seed item or the two members:

1. Cut rows on either member whose anchor, cutter or bisector references the
   other member.
2. Placements whose recipe anchors to both members, or to one member and to a
   gathered part (the plate anchored to C-1's flange and R-1's axis).
3. Components and fasteners whose carried hole cut is on a gathered part or
   member, and the Repeats of them.
4. Weld and glue records between gathered items.
5. Repeat until nothing is added; cap at 200 items.

The popover lists the result as **Includes**, each with a checkbox, so a wrong
gather is visible and fixable. Roles are painted on the members (filled
circle Main, numbered circles for the others; Revit's glyphs) with Swap.

**Conversion**, in one History step:

- create the junction row and its assembly; move gathered parts under it.
  Their uids are kept. A GlobalId is derived from the uid *path*
  (persistent-ids §7.1), so moving a part under the assembly would change it:
  capture first writes each moved part's current GlobalId into its record's
  `guid` (the existing override), so nothing the user made gets a new one;
- rewrite every gathered anchor from its member reference to the equivalent
  role frame plus an offset (exact: the frame and the old anchor resolve to
  the same pose, checked);
- run parameter discovery (§2.5) and show the found parameters and relations;
- build the detail template from the same data with symbolic references, and
  the rule from the meeting's signature: profile library keys, size ranges
  defaulting to the family sizes within ± 1 catalogue step of each role, the
  angle with ± 2°, mirror allowed;
- Save in: This project (default), My presets, or a pack folder.

Content keys are named from the items' kinds and presets (`end_cut`,
`plate`, `bolts`, `weld`, with a numeric suffix on repeats). The capturing
use records its existing uids against those keys in `contents`; derived uids
(§2.2) are used only for items a later expansion creates. Re-expanding the
capturing use after a detail edit therefore keeps every uid it had.

## 6. Editing

### 6.1 This one, or all uses

The record has one switch, "All 8 · This one" (Revit's type vs instance as a
visible choice), defaulting to All for a detail with more than one use.

| Edit | All | This one |
|---|---|---|
| A parameter | a detail revision; every follower re-expanded and re-evaluated in one step (a warning first when > 20 uses or when some uses have overrides on that parameter) | written to `overrides` (amber dot) |
| Add a part or cut (inside, §6.3) | added to the template | `local.added` |
| Remove a part | removed from the template | `local.removed` |
| The rule (tolerances, sizes) | a detail revision; uses that fall outside become `outside`, listed before the click | not available (a use has no rule) |

Overrides survive re-evaluation and detail revisions (Tekla's lesson), unless
the revision removes the parameter, which is reported on the use. "Upgrade to
detail" pushes this use's overrides into the detail (Advance Steel's "Upgrade
to master"); "Reset" drops them.

### 6.2 Detail revisions

The detail's `rev` is bumped on every edit; each use stores the revision it
follows. A project detail edit updates its uses in the same step, as Repeat
presets linked to project revisions do today (`live/presets.rs`). A newer
revision of the same detail in the Library (an updated pack) is offered, never
applied by itself: the Library row shows "Update available · rev 4 → 5 ·
8 uses", and Update is one step.

### 6.3 Edit inside (option B's double-click)

Double-clicking a junction marker or row enters its assembly as an edit
context, as group editing does (`scene.edit_group` = the junction assembly):
breadcrumb, everything else faded, the members painted with their roles and
snappable. Parts drawn there are children of the assembly by construction;
cuts made there on the members are owned by the junction (the Cut tool sets
`junction` when the edit context is a junction assembly). Done asks once, as a
chip, "Apply to all 8 · This one only" when the junction follows a detail with
several uses. Cancel restores the step.

Dragging a cut row from under a member onto a junction row adopts it
(`junction_adopt`), with its anchor rewritten to a frame.

### 6.4 Detach

Detach makes the use custom (`detail: null`), keeping its current contents
and values; "Save as detail…" on a custom junction starts capture from its
contents, so it becomes a new detail.

## 7. Release

Release bakes **one level**, keeping what each element is (general-base §4.4):

| Released | Becomes | Kept |
|---|---|---|
| Junction | the row removed; owned cut rows become ordinary cut rows on their members (frames rewritten to plain anchors); parts stay ordinary placements under a plain group, each with its own recipe | uids, records, GlobalIds; the group keeps `IfcElementAssembly`, and the connection is written into its record (`ifc.connects`: relating, related, realizing uids, connection type) so the IFC export is unchanged |
| Child junction | the same, inside its parent | as above |
| Junction along a Repeat | one plain group per station | as above |

"Release all" on the context menu also releases the owned rows (cuts baked into
the members, plates into ordinary definitions), in one step. Released geometry
never re-evaluates.

## 8. Migration

### 8.1 Hosted doors and windows

Today (`hosted_openings.rs`, `live/cut.rs`): a door or window placed on a wall
writes `ext["wall_host"]` on the fill (frame normal, anchor, `relative` pose,
offset, sill, flipped), an entry in `wall.ifc["openings"]` (GUID, `wall_rect`,
`fill_uid`, `cut`), and an Opening cut row on the wall
(`uid = h64("built:opening:v1:<wall>:<fill>")`, `cutter: {"opening": {fill,
guid, frame, rect}}`). `hosted_openings::update` re-cuts when the fill is
inserted, moved, flipped or deleted (`doc/transform_steps.rs`,
`doc/group_steps.rs`), and `regenerate` moves the fill with the wall. Overlaps
are refused with an error.

Becomes a junction of meeting `hosted`:

| Today | Junction |
|---|---|
| The Opening cut row | an owned content `opening`, uid kept (so the opening GUID, stored in the cutter, is kept) |
| The fill (door, window) | role `hosted` with `attached: true`: the junction places it from its pose relative to the host frame (today's `relative`); it keeps its own uid, record and GlobalId. Its pose is an output, not an input: when the step moved the fill itself, the junction first stores the new relative pose (the user's intent) and re-cuts; otherwise it re-places the fill from the stored pose. Moving wall and door together leaves the relative pose unchanged |
| The wall | role `host` |
| Reveal, overlap and sill rules in `hosted_openings.rs` | parameters of the built-in **Opening** detail (`sill`, `offset`, `reveal`) |
| `update` / `regenerate` call sites | removed: the junction's evaluation covers host moves (hosted re-placed) and fill moves (opening re-cut) |
| "Wall faces have changed" refusal and overlap refusal | state `failed` with the reason, last good opening kept |
| `wall.ifc["openings"]` | written at save from the junction until lane J5 switches the IFC writer to read junctions; read on open for files from before |

**Door and window families as details.** A Library component may name a
hosted detail in its metadata (`"junction": "<detail uid>"`). The detail adds
parts sized from the host: lining depth `= host.thickness`, frame, lintel
`length = hosted.width + 2 × bearing` (a sweep part, `IfcBeam` LINTEL), sill,
fixings; its rule is the host kind and a thickness range. Placing the door by
hovering a wall applies that detail: an explicit placement, not a silent
suggestion. A component without one gets the built-in Opening detail (cut
only).

**Migration on open**, as general-base Q5 decided for walls: in memory on
open, written in the new form at the next save, never rewriting a file on
open. Each Opening cut row gets a junction row
`h64("built:junction:v1:hosted:<wall>:<fill>")`. Openings the converter cannot
read stay as they are, with their cut row.

### 8.2 Wall corners, tees and wall on slab

Today walls are Wall-preset sweeps (migrated from unit-cube boxes,
`live/cut.rs` `migrate_wall`) with no corner logic: corners overlap or leave
gaps. New built-in cut-only details:

| Detail | Meeting | Contents |
|---|---|---|
| Wall corner · by priority | `corner` (L) | the lower-priority wall trimmed to the other's inner face, the higher one's end extended to the outer face (Archicad's priority; priority = load-bearing over partition, then thickness, then creation order, from the records) |
| Wall corner · mitre | `corner` (L) | a bisector cut on both |
| Wall tee | `corner` (T) | the joining wall trimmed to the main wall's near face |
| Wall on slab | `base` | none by default; optional trim of the wall to the slab top |

Applying them: the Wall tool, when a wall's end snaps to another wall, applies
the corner detail as part of its own commit, with the marker and Undo (open
question Q4). Existing walls are not migrated; their corners appear in the
Junctions view's By arrangement list and in a review of the corner detail.
IFC adds `IfcRelConnectsPathElements` (ATSTART, ATEND, ATPATH) for them (§9).

## 9. Export

### 9.1 IFC

The IFC writer (`formats/ifc/export.py`, `relationships.py`) already writes
assemblies through `IfcRelAggregates` for a group whose record is an
assembly, openings from the record with void and fill relations, mapped items
under the `instancing` option, and record-driven classes. Junctions add:

| Built | IFC4 / IFC4X3 |
|---|---|
| Junction assembly (one per meeting; one per station along a Repeat) | `IfcElementAssembly`, PredefinedType ACCESSORY_ASSEMBLY, AssemblyPlace from the detail record (FACTORY for shop connections, SITE otherwise); GlobalId `stable_guid(namespace, "occurrence", owner uid path)` (persistent-ids §7.1) |
| The detail | `IfcElementAssemblyType` (Name = the detail name), `IfcRelDefinesByType` from every use |
| Parts | aggregated into their junction's assembly by `IfcRelAggregates`; a child junction's parts are aggregated into the **top** junction's assembly (flat, open question Q2) |
| Each pair (Main, other role) of every junction | `IfcRelConnectsWithRealizingElements`: RelatingElement = Main, RelatedElement = the other role, RealizingElements = the junction's parts touching both (for a top junction, its children's parts included), ConnectionType = the detail name; GlobalId `stable_guid(namespace, "junction", row uid, main uid, other uid)` |
| Bolts, screws, dowels, nails, anchors, rivets | `IfcMechanicalFastener` (BOLT, SCREW, DOWEL, NAIL, ANCHORBOLT, RIVET; COUPLER in IFC4X3), one per fastener; `IfcMechanicalFastenerType` per shared fastener definition with NominalDiameter and NominalLength, its representation map shared |
| Welds, glue | `IfcFastener` WELD or GLUE, with `Built_Weld` (throat, length, type, sides, process); bead geometry when present, else the path as an `Axis` curve representation |
| Plates, cleats, stiffeners | the part preset's record class (`IfcPlate` for plates), as for any sweep |
| Hangers, brackets | `IfcDiscreteAccessory` SHOE, BRACKET (the component's record) |
| Lintel, sill, lining of a hosted detail | the part's record class (`IfcBeam` LINTEL, …), aggregated into the hosted junction's assembly only when the detail adds parts |
| Holes | `IfcVoidingFeature` HOLE through `IfcRelVoidsElement` on the ply, with an extruded circle body, in the Design Transfer profile; baked into the tessellation in Reference View |
| Other cuts | as general-base §4.2 and lane 3 write them (clipping, `IfcBooleanResult`, voids) |
| Openings | `IfcOpeningElement` + `IfcRelVoidsElement` (host) + `IfcRelFillsElement` (hosted), read from the junction's opening content; no connection relationship unless the detail adds fixings |
| Wall corners and tees | `IfcRelConnectsPathElements` (RelatingConnectionType / RelatedConnectionType ATSTART, ATEND, ATPATH) |
| Every junction, and each part (naming the detail it came from) | the property set `Built_Junction` (not `Pset_`: that prefix is reserved for buildingSMART sets; options.md said `Pset_BuiltJunction`): DetailName, DetailId, DetailRevision, Pack, Status, Overridden (list), and each parameter by key |

A junction whose status is not `ok` exports what it has and lists itself in
the export report ("J-4 outside Knee end plate: exported with its last good
parts").

A BTLx writer does not exist yet. When it does, owned cut rows map to
processings per member (Lap, Tenon, Mortise, BirdsMouth, Drilling for holes)
and a junction can be a BTLx 2.3 `Connector`; nothing in this design needs to
change for it.

### 9.2 STEP

Parts are products under the junction assembly's product (NAUO), fasteners of
one definition share one product definition. Members stay where they are.

### 9.3 Schedules and BCF

Schedules count from part records and junction rows, never from geometry
guesses:

| Schedule | Grouped by | Columns |
|---|---|---|
| Bolts and fasteners | standard, grade, d × length | count, detail(s) that use them, junction count |
| Plates and cleats | material, thickness × width × length (or profile × length) | count, unit mass, total mass |
| Welds | type, throat | total length, count |
| Holes | ply material and thickness, Ø | count (for the workshop) |
| Junctions | detail, revision | uses, overrides, status |

They extend the quantities backend (`core/bim.py` `quantities_csv`) with
fastener and weld record fields, and appear in Deliverables as tables; the
storyboard's frame 8 ("32 M16 and 96 M12 bolts, 8 plates, 48 cleats, 8.5 m of
weld, each traced to its detail") is the acceptance case.

BCF: a topic can point at a junction (its assembly's GlobalId as the
component; `authoring_tool_id` `built:<owner uid path>`) or at one part.

## 10. Portable detail packs

A detail must open, list and apply in a model that has never seen its source,
and later come from an online library. The template (§2.6) already uses only
roles, frames, content keys and preset uids. A pack adds identity,
provenance and dependencies.

**Layout** (a folder, and the same folder zipped as `.builtpack` for
download):

```
steel-connections-en/
  pack.json
  details/knee-end-plate.builtpreset
  details/bolt-m16-88.builtpreset
  parts/plate.builtpreset
  profiles/…builtprofile        (only profiles that are not built in)
  fasteners/iso-4014.builtfastener
  components/hanger-ba-80.built (component geometry, when not generated)
  thumbnails/knee-end-plate.png
```

**`pack.json`**, version 2 of the presets envelope (version 1 packs stay
readable):

```
{"format": "built.presets", "v": 2,
 "pack": {"id": "<32 hex>", "name": "Steel connections · EN", "version": "1.3.0",
          "author": {"name": "…", "url": "…"}, "licence": "CC-BY-4.0",
          "units": {"length": "mm", "angle": "deg"},
          "requires": {"built": ">= 1.0", "features": ["junction.v1", "hole.v1"]},
          "files": {"details/knee-end-plate.builtpreset": "sha256:…", …}},
 "rows": [ … preset rows, or paths to them … ]}
```

**Each detail row** carries, beside the preset fields:

| Field | Meaning |
|---|---|
| `uid` | 16 hex, stable for life; with the pack id, globally unique |
| `rev` | integer, bumped on every published change; `hash` = sha256 of the row's canonical JSON |
| `provenance` | `{"created": date, "created_with": "Built 1.2", "derived_from": {"pack", "uid", "rev"} \| null}`; no document ids, paths or user names are written unless the author typed them |
| `author`, `licence` | inherited from the pack, overridable per detail; licence as an SPDX id |
| `units` | inherited from the pack; an imperial pack (AISC) declares inches and is converted on import |
| `depends` | `[{"kind": "profile" \| "fastener" \| "component" \| "detail" \| "preset" \| "material", "key": "steel/EN 10365/HEA", "rev"?: n, "hash"?: "sha256:…", "bundled": "path" \| null}]` |
| `params` | each with `name`, `prompt`, `group`, `limits`, `lock`, default `expr` (the adjustable values) |
| `thumbnail` | a path in the pack |

**Import** (Library › Presets › Import, or a download):

1. Check the envelope, the version, the `requires` features and every file
   hash; refuse with the reason if any fails. Size limits as for other
   archives (`archive_limits.rs`).
2. Resolve each dependency: built in (by key), already in the Library (by key
   and hash), bundled in the pack, or missing. A detail with a missing
   dependency is listed as "Needs Steel AISC W shapes" and cannot be applied
   until it is resolved; nothing is half-imported.
3. Validate the template: references only to its own roles, frames, content
   keys and declared dependencies; parameter expressions parse and have no
   cycles; component geometry passes the solid check.
4. Store it in the Library as built-in (read-only), local, or modified.

Packs are data only: no scripts, no executable expressions, nothing that
reaches the file system or the network.

**In a project**, an applied detail is embedded (`ext["built.presets"]`) with
its pack id, uid, rev and hash, and its bundled dependencies (profile tables,
fastener tables, component definitions) as the document already embeds
presets and drawn profiles, so the file opens anywhere. Editing a detail that
came from someone else's pack saves a new uid in This project or My presets
with `derived_from` set; the pack's copy is never edited in place.

**Export** ("Share pack…"): choose details; the writer collects their
dependencies, bundles what is not built in, asks for author and licence (no
default licence; required to publish to the online library, optional for a
local folder), writes hashes and the zip.

## 11. Interaction contracts

The UX is the pass-6 board (option A). The contracts below are what the tools
call; every commit is one protocol command through `ToolHost::run`, so
scripts, the agent bridge and tests use the same path, and each new tool
regenerates `docs/tool-coverage.md`.

| Surface | Contract |
|---|---|
| Junction tool (rail, J) | chip *detail · roles · fit*; L opens the Library on Junction details that fit; ↓ cycles details that fit the meeting under the cursor; hover previews (ghost); click applies (`junction_apply`); Alt-click applies outside the rule as custom; on a Repeat copy asks once: every copy (`along`), one end, or this meeting |
| Capture chip | §5; `junction_capture` |
| Review | toast "Knee end plate saved · 7 similar · Review"; badges and ghosts; list with why, near misses with Widen, Skip remembered; Enter applies the ticked ones in one step |
| Marker | an overlay mark at `meet` per junction (`tools/overlay_marks.rs`), coloured by state; click selects the junction, double-click enters it (§6.3); shown for the selection's junctions, for all while the Junction tool is armed or a detail is selected, and under View › Show junctions |
| Tree | under each member, its junctions (a link row under the other members); one project Junctions view grouped by detail, with a By arrangement view (signatures from §4.2: has a detail on all / none / some outside, counts) and a Problems filter (`outliner/`) |
| Record | roles with Swap, parameters (with the sized-from relation shown, "= Rafter width + 30"), "All 8 · This one", overrides with amber dots, Includes, status with its ways out, Upgrade to detail, Detach, Release (`inspector/records.rs`) |
| Library | Junction filter in Presets, thumbnails, the rule in words ("HEA 152–240 column flange, IPE 200–300 rafter end, 75° ± 2°, mirrored"), locks, Apply…, packs and updates (`library_host/presets.rs`) |

Protocol verbs: `junction_capture(seed, roles?, include?)`,
`junction_apply(detail, bindings)`, `junction_set(row)`,
`junction_adopt(junction, rows)`, `junction_detach(uid)`,
`junction_release(uid, deep=False)`, `junction_skip(detail, meeting,
skip=True)`, and the queries `meetings(scene, filter)`,
`junction_suggest(scene, detail)`, `junction_frames(scene, uid)`.

## 12. Implementation lanes

Each lane merges alone into `main` with Rust unit tests, Python protocol
tests, window tests and frames in lived-in state (full recents, a missing
linked file, a 1280 × 800 window, the ride open beside the garage). At most
four lanes at once. J0 first; then J1 and J3 in parallel; then J2, J4, J5 and
J6.

| Lane | Delivers | Depends on | Effort | Gates |
|---|---|---|---|---|
| J0 Junction core | the `junction` row kind; `junction`/`key` on owned rows and the `owned` index; the junction assembly and derived uids; roles, role frames and the `{"junction", "frame"}` reference; `Key::Stock` and the stock/result split for sweeps and Repeats; early cut-off; `live/expr.rs` parameters and bindings; expansion and re-expansion by content key; statuses with last good; `ext` read/write; verbs `junction_set`, `junction_apply` (explicit bindings), `junction_release`; cut-only details (Corner mitre) as test fixtures; a minimal marker and tree row | — | L | a mitred corner junction follows moves and profile swaps; undo/redo restores ids and GlobalIds exactly; one failed cut on a column leaves its other junctions `ok`; a no-op step < 0.1 ms; 5,000 rows open < 30 ms; cycle refused with its message |
| J1 Parts and fasteners | part contents (preset parts with bound fields, component parts with written transforms); the `hole` cutter with the analytic fast path; fastener tables and generated shared fastener definitions; the bolt junction; a Repeat of a bolt junction with repeated holes; weld and glue junctions (path by role ids, optional bead); child junctions to depth 8; the knee built through verbs | J0 | L | storyboard frames 1–3 built through the protocol; 32 holes on plate and flange re-evaluate < 5 ms; bolts of one size share one definition; weld length exact to 0.01 mm; one knee < 2 ms |
| J3 Meetings and suggestions | member extraction (stock, classified boxes cached by definition revision); the grid broad phase and classifier for all kinds; keys and signatures; `MeetingIndex` with incremental updates; rule matching, near misses, already detailed; the review overlay (badges, ghosts, list, Skip, Enter applies); Follow; the data for By arrangement | J0 | L | §4 budgets on the ride, the garage and the 2,000-member hall; the garage's 8 knees and 48 cleat meetings found with no false positives; mirrored and reversed members match; skips survive save and reopen |
| J2 Capture and editing | gathering, the capture popover (roles painted, Swap, Includes, rule with live count, parameters found with relations), conversion to frames; the Junction tool (J, chip, L, ↓, Alt-click, Repeat question); `along` junctions; tree rows under members, the Junctions view and By arrangement; the record (All / This one, overrides, Upgrade, Detach, Release); edit inside; adopt by drag | J1, J3 | L | the storyboard beat frames 4–7 from the tool and chip only; a captured detail applied to the 7 other knees gives the same parts as the source, mirrored where needed; overrides survive a detail revision |
| J4 Hosting and walls | the `hosted` meeting with attached roles; migration of hosted openings on open; the Opening detail; door and window components with hosted details (lining, frame, lintel, sill); wall corner, tee and wall-on-slab details; the Wall tool applying corners | J0, J1 | M | every existing hosted-opening test passes on the junction path; a moved and lengthened wall carries its doors; old files open with the same opening GUIDs; an overlap is `failed` with the last good opening, not an error |
| J5 Export and schedules | IFC assemblies and assembly types, connection relationships, fasteners with types, weld and glue fasteners, voiding features for holes, `IfcRelConnectsPathElements`, `Built_Junction`, the export report; STEP product structure; the junction schedules in Deliverables; BCF on junctions | J1 (J4 for openings and walls) | M | IfcOpenShell validation of the garage; GlobalIds stable across two exports without saving; the frame-8 counts exact; a released junction exports the same relationships |
| J6 Library and packs | the junction primitive in Library › Presets (filter, thumbnail, rule in words, Apply…, update offers); pack format version 2 (identity, provenance, licence, units, dependencies, hashes); import without the source model, with dependency resolution and validation; Share pack…; starter packs: Steel connections · EN (end plate 4 bolts, fin plate, base plate, apex haunch, purlin cleat), Timber joints (birdsmouth, half-lap, tenon and mortise with dowels, joist hanger), Generic (corner mitre, butt with screws), Architecture (door and window openings, wall corner, wall tee, wall on slab), MEP (flange, coupler) | J0, J1 (J4 for the Architecture pack) | M | every starter detail applies in an empty document; a pack exported from the garage imports on a clean profile and applies to a new frame; a missing dependency is reported, nothing half-imported; a tampered file is refused by its hash |

The demo beat (pass-6 storyboard: knees saved once and re-applied, purlin
cleats across the Repeat, schedules and IFC) is the acceptance test for J0–J5
together.

### Exercising J0 through the protocol

The cut-only Corner mitre fixture in `tests/test_live_junction.py` creates two
timber sweeps and a project detail. Each trim uses `@meet` and binds its
offset to `gap`, which follows `main.width / 45` in millimetres. Apply the
saved detail to two existing members in one History step.

```python
reply = commands.run(history, "junction_apply", detail=detail_uid, bindings={
    "main": {"placement": main_uid, "end": "end"},
    "joining": {"placement": joining_uid, "end": "start"},
})
```

The reply includes the new junction row uid. Read the saved role matrices
with `junction_frames(scene, uid)`. Edit the row with `junction_set` and use
`overrides` for a value on this use. Setting `detail` to `null` keeps its
cuts and parameter values as a custom junction. `junction_release` accepts
`uid` and an optional `deep` boolean as described in §7. J1 adds template
expansion for parts and fasteners to the ownership and frame machinery.
For a low-level owned sweep, pass `junction`, `key`, `profile` and `path`
to `live_set`. The assembly creates the child placement. The row uid uses
the content key and the placement uid uses the same key with `:owner`
appended.

### J1 protocol fixtures

`tests/test_live_junction.py::knee_fixture` builds the end cut, adds the
plate, then adds a 2 × 2 bolt group and a weld. A plate uses a sweep preset
with an IfcPlate record. Its `thickness` field is the length of its local
Z path in millimetres. A component part takes the uid of a placed or kept
component. Part poses use `at.frame` and an optional three-axis millimetre
offset. A child role can reference a parent role or an earlier part by
`@key`, optionally overriding its contact face or end.

A bolt detail contains a `part.fastener` and ordinary hole cuts on the
ordered plies. Expansion supplies a bore for each ply unless the template
specifies its own hole on that ply. The metric table supplies bolt heads,
nuts, washers and stock lengths. A fastener defaults to ISO 4017, two washers and a nut. Its grip
defaults to the sum of ply thicknesses. The template's nested Repeat uses
counts including the source. Each hole stores the Repeat uid inside its
hole specification so its diameter, depth and bore frame remain available
alongside the station source.

A `part.weld` uses the stock section of role `a` projected onto
`b.contact`. Its record contains length in millimetres. An optional
`edges` array selects profile edge roles and `bead: true` creates the
triangular fillet. A `part.glue` clips the projected outline to the stock faces
on the contact plane and stores their overlap in square millimetres.
The junction exposes the measured `length` or `area` as a derived field.

### Hosted families and Wall tool (J4)

A hosted component can set its placement extension `junction` to the UID
of a hosted detail. The Architecture door and window details accept walls
60–650 mm thick. Their lintels bear 150 mm at each jamb by default. A
component without a detail receives the cut-only Hosted opening. The
`hosted_opening` content entry adopts the existing rectangular cut. Its
UID and stored opening GUID survive migration. The wall opening record
also remains available to the IFC writer.

The Wall tool requests `wall_junctions=true` on `create_sweeps` for new
walls. A shared end applies the priority corner. An end on the run of
another wall applies the tee. The new wall and its junctions share a
History step. Loading existing walls leaves their intersections available
for review.

### Junction interchange and schedules (J5)

In IFC, each connection has one assembly. Its plate, bolts and welds are
direct children. Child details retain their own connection relationships
and each part carries the detail that produced it. For a cut-only connection
the writer uses `IfcRelConnectsElements` because the realizing-elements
relationship requires at least one product. Wall corners and tees use path
connections. Hosted openings retain their void and fill relationships.

Design Transfer exports a persistent hole feature at each drilled station.
Reference View includes the evaluated bores in the ply geometry. A weld
without a bead has an IFC axis and a STEP product without a solid. STEP
shares the product definition across instances of a fastener.

Open **Deliveries → Junction schedules** to inspect the model quantities.
**Place on new sheet** places the displayed table as editable paper text.
The sheet is a snapshot. A package with Quantity schedule enabled includes
the nonempty fastener, plate, weld, hole and junction CSVs alongside the
ordinary takeoff. Reviewing an issued package shows its saved tables.
Plate dimensions and unit masses describe stock. A missing material density
leaves mass blank. The BCF package includes a viewpoint targeting the
exported top assembly for each junction problem.

### Meetings and the review (J3)

`live/meetings.rs` builds the index the first time a query asks and keeps
it per document. It never reads faces:

| Element | Member |
|---|---|
| Sweep | its stock path (the stock cache, else the marker's `path`) and its profile's envelope, one capsule per span |
| Repeat copy | the source sweep's stock in the copy's frame; a direct copy binds as `{"row", "station"}`, a copy inside a nested copy, or a copy of a sweep's placement sharing its definition, binds by placement |
| Adaptive copy (own geometry) | the source's section over the copy's extent along the source axis |
| Classified plain element | its box, cached by definition key and store revision: a bar when the longest side is at least 3 × the next, a wall, a slab (IfcSlab, IfcFooting, IfcRoof, IfcPlate), or a hostable door or window |
| Junction assemblies, linked files, unclassified geometry | skipped |

Section axes are read in the member's canonical direction, so a member
drawn the other way meets the same way; its `hand` (mirror class) is the
side of the main member's section the other comes from.

Seated ends, L and tee corners, meeting keys without the contact face, and
the rule's `hand`, `arrangement` and `end` readings are in §4.2 and §2.6. A
`base` end or wall bottom may lie inside the slab's thickness. The signature
(By arrangement) is the kind, `class:profile:face or end` per role (sorted for
symmetric kinds) and the angle to the degree; mirrored meetings share a group.

The index follows the document through `Doc.touched`, the keys written
since a reader asked: each touched subtree is walked again and only the
members that changed are re-queried in the grid. Undo, redo and plugin
writes are covered the same way. The log wraps at 65,536 keys and the
index then rescans.

Skips and Follow are journalled on the live table and saved as
`ext["built.junctions"]`. Follow lists a detail's open matches in
`junction_counts` (`new`); nothing is applied.

| Protocol | |
|---|---|
| Verbs | `junction_apply(detail, bindings)` with one binding map or a list (one step), `junction_skip(detail, meeting, skip=True)` with one key or a list, `junction_follow(detail, on=True)` |
| Queries | `meetings(scene, filter)`, `junction_suggest(scene, detail)`, `junction_counts(scene)`, `junction_arrangements(scene)`, `junction_ghosts(scene, detail, keys)`, `meeting_index_stats(scene)` |

`junction_suggest` lists each meeting of the detail's kind as `match`,
`near` (one criterion out, with `widen_to`), `skipped`, `detailed` (a
junction of another detail) or `applied`, with `why` atoms and the role
bindings `junction_apply` takes.

Measured on the synthetic fixtures (release build, M5 Max;
`tests/test_junction_meetings.py`, `BUILT_PERF=1` asserts these targets):

| §4 budget | Target | Measured |
|---|---|---|
| Build, garage beside 400 unclassified ride parts | < 5 ms | 0.22 ms |
| Build, steel hall (2,005 members, 3,713 meetings) | < 20 ms | 8–12 ms, on the main thread |
| Update after a step that moved one member | < 0.5 ms | 0.02 ms |
| Memory, steel hall | < 2 MB | 1.93 MB (estimated from the index's own sizes) |
| Search one detail, hall | < 10 ms | 1.3–1.6 ms |
| Search all project details (5), hall | < 50 ms | 5.9–8.3 ms |

**The review** (`built_render/src/junction_review.rs`). Saving a junction
detail that has matches shows "Knee end plate saved · 7 similar meetings ·
Review". The review draws a badge at each meeting (ticked, unticked, near,
skipped) and dashed ghosts of the detail's planes for the ticked ones, and
lists the meetings beside the rail: Near with Widen (a detail revision by
the one change), Skipped with Un-skip, Apply N and Skip all. A click on a
badge ticks it, Alt-click skips it, Enter applies the ticked in one History
step, Esc leaves with nothing applied. Scripts and the agent bridge use
`built_render.junction_review(window, detail)`,
`junction_review_action(window, "tick:3" | "apply" | …)` and
`junction_review_state(scene)`.

### Library and packs (J6)

Junction details are presets of primitive `junction` in Library › Presets.
The Show menu filters presets by kind (Sweep, Cut, Repeat, Junction). A
detail's row gives the rule in words, what it adds, its parameters, its
IFC class, its uses, and its pack, revision, author and licence. A row
also says what the detail still needs. **Apply…** binds the selected
members, main role first; for two members either order works. It embeds the
detail and the presets it needs, then applies them in one History step
(`junction_apply` with `definitions`). A Library copy with a newer pack
revision than the model's shows "Update available · rev a → b · N uses".
Its **Update this model** command is one `preset_update` step.

The core functions are `pack_export`, `pack_read`, `junction_bind`,
`junction_definitions`, `junction_words` and `junction_thumbnail` in
`built_core.command_protocol` (`live/pack.rs`). The Library wraps them as
`wgpu_library.share_pack` and `wgpu_library.import_pack`. `pack_read`
writes nothing. It checks the envelope, the required features (refused
when unknown), and the units (millimetres only for now). It checks every
listed file against its sha256 and every row against its own `hash`
(canonical JSON without `hash` and `source`). It validates each template:
references only to its roles, `meet`, content keys and declared
dependencies, and parameters without a cycle. It then resolves
dependencies (built in, in the pack, or already in the Library) and marks
each row ready or `needs`. A row inherits what its child details need.
`import_pack` stores every row at the chosen destination or none. Rows
that need something are listed with what they need and cannot be applied.
This model takes only ready rows. Bundled components become Library
components; bundled profile sizes travel in the part preset as `family`
plus `params`.

Two template additions came with the starter packs:

| Addition | Use |
|---|---|
| `cutter: {"block": {"size": [x, y, z], "at": [x, y, z]}}` (mm, posed in the anchor's junction frame; subtract, union, intersect) | laps, tenons, mortises, shoulders |
| `part.at.axis`: `x`, `y`, `z`, `-x`, `-y`, `-z` (which frame axis the part's own Z follows) | fin plates, hanger sides, dowels across a tenon |
| `part.fastener.kind`: `bolt` (default), `screw`, `nail`, `dowel`, `anchor`; `holes` overrides the bore default (screws and nails none) | timber fixings, anchors |

The starter packs ship as format 2 packs with inline rows in
`resources/presets/packs/`. They are Steel connections · EN (End plate,
4 bolts; Fin plate; Base plate; Apex haunch; Bolt M16 8.8; Anchor M16;
Fillet weld), Timber joints (Birdsmouth, Half-lap, Tenon and mortise,
Joist hanger) and Generic (Corner mitre, Butt with screws). The
Architecture pack is J4's. `tests/junction_starter_cases.py` draws the
members for each one in an empty document.

### Capture and editing (J2)

**Gathering** (`live/junction/capture.rs`) starts from a seed: a cut row, a
part, a junction, or the two members. The seed's meeting is the index
meeting whose members the seed references, nearest to it; meetings that
involve a part (`IfcPlate`, fasteners, accessories) are parts of a meeting,
never one. Then, until nothing is added (at most 200 items):

| Gathered | When |
|---|---|
| Cut on either member | its anchor or cutter references the other member or a gathered part |
| Part (a sweep classed `IfcPlate`, `IfcFastener`, `IfcMechanicalFastener`, `IfcDiscreteAccessory`, `IfcBuildingElementPart`) | it follows both members, or a member and a gathered part (*anchored*); or it rests on one member near the meeting (*near*, still ticked); a part with no anchor near the meeting is listed unticked |
| Junction (a bolt group, a weld) | every role is a member or a gathered part, and one is a part |

A part bound in a junction role while anchored to a face of another role's
member (a plate drawn on a column flange, then bolted through that flange)
would read the member's result while the junction cuts it. Its anchored end
is pinned where it is and `path.rests_on` keeps the member, so capture still
gathers it with that member.

**Conversion** keeps every uid: the detail is inserted, the gathered rows are
owned under their template keys, and expansion adopts them (`contents`
records their uids; later uses derive theirs). Moved placements and their
descendants get their current GlobalId written into the record first. A plane
cut is re-anchored to the role frame whose Z is its normal (`meet`, the other
role's `contact`, its own `contact` or `end`) with the offset that keeps the
plane; a part keeps its pose as `at: {"frame": "@meet", "offset", "basis"}`
(the part's axes in the frame, column-major 3 × 3) and its section's turn,
mirror and attachment; a part without a preset carries its section and record
inline (`part.sweep`, `part.record`). A child junction keeps its detail and
binds its roles to `@main`, `@joining` or a part's key.

**Parameters found**: a part's thickness and section dimensions, non-zero
offsets, a cut's gap or clearance, and a child junction's own parameters.
Equal values of the same field merge. Relations, best first: equal to an
earlier parameter, equal to a role dimension, that dimension plus a multiple
of 5 mm up to half of it, then × ½ or × 2. The popover shows the relation in
words ("= R-1 width + 30"); a click tries the next one, then a fixed value.
The rule takes the meeting kind, each role's catalogue series within one step
(`profile` and `size.height`), the angle ± 2°, `mirror: allow` with the
source `hand`, `direction: ignore` and the source offset ± 5 mm.

**Along a Repeat** (Q1): one row with `along: {"repeat", "end", "skips",
"stations"}`. Its children are ordinary junctions, one per copy and end
(`<copy uid>:<end>`), each bound to the member that copy meets; a station's
own values are its child's overrides, kept by that key when copies change. A
step that adds or removes copies re-expands the row in the same History step.

**Editing.** The record shows status with its ways out (Widen the detail,
Keep as custom), roles with Swap, "All N · This one" (All writes a detail
revision; This one writes `overrides`, marked ●), the parts, and Edit inside,
Select all, Similar…, Upgrade to detail, Reset, Detach, Release and Release
all. Inside a junction (double-click it, or Edit inside) the Cut tool's cuts
are this use's (`local.added`); leaving with new cuts or parts asks "Apply to
all N · This one only". A cut row dragged onto a junction's tree row is
adopted with its anchor rewritten to the junction's frame. Release is one
level; Release all bakes everything (Q5).

**The Junction tool** (rail, Draw › Junction): the chip names the detail,
the meeting and its fit; hovering shows the detail's ghost; a click applies a
fitting detail, Alt-click applies it outside its rule (`local.forced`, the
rule is not checked); ↓ cycles the details that fit the meeting; L opens the
junction details. On a Repeat copy it asks once: every copy, both ends; every
copy, this end; this meeting only. A detail between parts (a bolt group)
takes its plies by pick order.

The project Junctions view (the tree's last section) lists uses by detail,
by arrangement (§4.2 signatures with how many are detailed), or the
problems; its context menu switches.

| Protocol | |
|---|---|
| Verbs | `junction_capture(seed, options)` (`include`, `swap`, `name`, `tol`, `mirror`, `exprs`, `sizes`), `junction_adopt(junction, rows, detail=False)`, `junction_upgrade(uid)`, `junction_apply(detail, bindings, force=False, along=None)` |
| Queries | `junction_capture_preview(scene, seed, options)`, `junction_capture_offers(scene)`, `junction_record(scene, uid)` |

## 13. Risks

| Risk | Mitigation |
|---|---|
| The stock/result split touches the scheduler every recipe relies on | J0 lands it with the existing sweep, cut and Repeat tests unchanged before any junction code; stock is a strict subset of what a sweep computes today |
| Booleans per hole are too slow (Python `manifold3d`) | the analytic hole fast path covers holes normal to plates and flanges (nearly all of them); repeated holes are one row per ply; J1 measures before the starter packs |
| Gathering picks up the wrong items at capture | Includes is a visible checklist; gathering only follows anchors, never proximity; capped at 200 items |
| Meeting detection is noisy on imported, unclassified models | unclassified geometry is skipped; only classified elements with a clear axis participate; the review never applies without a tick |
| Detail edits fan out to hundreds of uses in one step | early cut-off, shared fastener definitions, a progress toast past 200 ms, the use count shown before the click |
| Role frames are ambiguous for exotic meetings (skewed, eccentric, three members at one node) | the rule's `offset` tolerance and an explicit Swap; three or more members bind by pick order only after the user confirms the painted roles; a frame that cannot be resolved is `lost`, never guessed |
| Templates reference profile edge roles that another family lacks (an I-section flange on a tube) | the rule's `profile` list keeps such meetings out; a forced apply (Alt-click) of a template whose face role is missing is `failed: face_missing` with the last good parts |
| Packs from an online library carry unsafe or broken data | data only, hash-checked, validated before listing, dependencies resolved or the detail refused whole |
| Older Built builds reading new files lose openings in IFC | `wall.ifc["openings"]` keeps being written until J5; recipe data stays optional extension data (general-base Q1) |

## 14. Open questions for the owner

| # | Question | Recommendation |
|---|---|---|
| Q1 | A junction applied along a Repeat (the purlin cleats): one row with per-station overrides, or one row per meeting? | **Implemented in J2: one row with `along`**, like a repeated cutter: new copies get their cleat in the same step, skips and overrides stay with their purlin by station key, and the file stays small. A station that needs a different detail is detached into its own junction. |
| Q2 | IFC for child junctions: aggregate their parts flat into the top junction's assembly, or nest one `IfcElementAssembly` per child junction? | **Flat.** One assembly per connection is what Tekla, Advance Steel and most checkers expect; each child junction still writes its own `IfcRelConnectsWithRealizingElements` (plate to rafter realized by the weld; plate to column realized by the bolts), and `Built_Junction` on each part names the child detail it came from. |
| Q3 | Bolt, nut and washer geometry: generated from standard tables and shared per size, or one Library component file per size? | **Implemented in J1.** ISO 4014/4017 bolts use generated, shared definitions keyed by standard, diameter, length and grip. Grip sets the nut and washer positions. Library components stay for hangers, brackets and couplers. |
| Q4 | Should the Wall tool apply the corner detail when a wall ends on another wall? | **Implemented in J4**, as part of the Wall tool's own commit, with the marker and Undo: it is the tool's visible behaviour, not a silent suggestion. Existing walls get corners only through the review. |
| Q5 | Release: one level (cuts stay live cut rows, a plate stays a sweep), or bake everything? | **Implemented in J2: one level**, consistent with Repeat's Release; "Release all" in the same menu bakes everything in one step. |
