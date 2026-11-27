# Built document format, version 2

A Built document uses the extension `.built` and is a ZIP archive. The
required UTF-8 member `document.json` identifies it with the top-level
integer `"built_format": 2` (version 1 files still open). The identifier
is mutually exclusive with `igz_format`. Readers reject unknown Built
versions.

## Version 2: persistent ids

Version 2 adds persistent identities (design and rules:
[persistent-ids.md](persistent-ids.md)). Everything else is version 1.

- `doc`: `{"id": 32 hex, "origins": [hex, …]}`. `id` names the document
  (drawn when it is created; a version 1 or `.igz` file gets one at its
  first version 2 save). `origins` is the table element ids index:
  index 0 is always `"0"`, the migration origin.
- `requires`: feature names a reader must implement to open the file
  correctly; empty in version 2 (see Stability).
- Every geometry block (`scene.geom`, each `protos[i].geom`) has `uid`, its
  definition's uid (16 hex; the loose geometry is `0000000000000000`),
  and per kind (`vertex`, `edge`, `face`) exactly one of
  `ids[kind]`, the packed id of the first element as hex (the others
  follow by +1), or an array `<kind>_id` of dtype `<u8` with one entry
  per element: wrapping differences from the previous packed id (the
  first from 0). A packed id is `origin index << 40 | counter`. Ids are
  unique per kind within a block. A block without them (version 1) gets
  migration ids: origin 0, the counter the element's position in the
  mesh as loaded.
- Rows gain `uid` (16 hex): `materials`, `saved_views`, `dimensions`,
  `text_labels`, `image_planes`, `geo_paths`, `guides`, sheet items
  whose records already had a `uid` field. A sheet frame that shows a
  scene adds `view_uid`. Section planes keep their `uid` (integers in
  older files, 16 hex for new planes).

## Version 1

Version 1 inherits the Built format 3 scene, prototypes, groups, sheets,
embedded textures, survey blobs and thumbnail. Each geometry block describes
a member under `geom/` and its arrays by name, little-endian dtype, shape
and byte offset. Writers align arrays to eight bytes. Edge endpoints and
face ring corners index the point array. Ring sizes, counts, flags, curve
and layer indices and face attribute tables retain their format 3 meanings
and validation. Empty meshes can use empty JSON edge and face lists.

Points are triples in metres. A block with even one coordinate that changes
on conversion to IEEE binary32 uses `<f8` for the whole point array.
Other blocks may use `<f4`. Placement `xform` is a column-major list of
16 binary64 values encoded as round-trippable JSON numbers. The document
camera uses the same JSON precision for its model-space `target` and optional
`up` triples. The default up is (0, 0, 1). Yaw, pitch and distance determine
the eye relative to the target. A compatibility export rounds the coordinate
triples to binary32. Element keys
are session-local; version 2 adds persistent ids beside them.

The Built compatibility export writes a format 3 ZIP using binary32
points. Its precision warning reports the largest Euclidean displacement
of a placed mesh point in world coordinates, in millimetres when it exceeds 0.0254 mm. Opening existing
Built formats 1, 2 and 3 follows their existing rules.

## Storage measurement

Level-1 DEFLATE on a synthetic 300 × 300 point grid with horizontal and
vertical edges and 89,401 unit quads produced 1,119,670 bytes with binary32
points and 1,118,540 bytes with binary64 points. The same test on one unit
quad produced 443 and 445 bytes respectively. Sizes include the ZIP and the
minimal scene manifest. Repeated integer coordinates compress well at either
width. The writer uses binary32 for exactly representable blocks to halve the
uncompressed point payload and retain the existing fast deferred-load path.

## Stability

Version 2 is the long-term format.

- Every Built release reads every earlier Built format and every `.igz`
  format.
- Ids are never reused and never change meaning. A field, array or table
  is never repurposed: a new meaning gets a new name.
- New optional data goes into `ext` maps (`"ext": {"<namespace>": …}`),
  which readers keep verbatim through edits and saves, or into new
  optional keys an older reader can safely ignore. Version 2 keeps the
  document's top-level `ext` and every placement's `ext`; giving another
  table an `ext` map later is itself an additive change. Unknown keys
  outside `ext` are ignored, not kept.
- Unknown geometry arrays survive only while their block is unchanged; a
  rewritten block drops them. Per-element data that must survive goes
  through `requires`.
- `requires` lists features a reader must implement to open the file
  correctly (like PNG's critical chunks); a reader that lacks one refuses
  the file and names it. The major number moves only when an existing
  field changes meaning, and the reader for the old one stays.
- The compatibility export (`.igz` format 3) carries no ids: reopening it
  derives migration ids, not the `.built` file's.
- Reserved for exact geometry ([persistent-ids.md](persistent-ids.md)
  §11, [exact-geometry.md](exact-geometry.md) §2): the critical feature
  `built.exact_geometry.v1` in `requires`, the namespace
  `ext["built.exact_geometry"]`, and the block array families
  `face_surface_*` (`face_surface_id` / `face_surface_ref`),
  `edge_exact_curve_*` (`edge_exact_curve_id` / `edge_exact_curve_ref`),
  `face_loop_*` and `coedge_*`. Only files carrying exact data use them;
  version 2 readers without the feature refuse such a file.
