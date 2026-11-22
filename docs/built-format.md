# Built document format, version 1

A Built document uses the extension `.built` and is a ZIP archive. The
required UTF-8 member `document.json` identifies it with the top-level
integer `"built_format": 1`. The identifier is mutually exclusive with
`igz_format`. Readers reject unknown Built versions.

Version 1 inherits the IngeTrazo format 3 scene, prototypes, groups, sheets,
embedded textures, survey blobs and thumbnail. Each geometry block describes
a member under `geom/` and its arrays by name, little-endian dtype, shape
and byte offset. Writers align arrays to eight bytes. Edge endpoints and
face ring corners index the point array. Ring sizes, counts, flags, curve
and layer indices and face attribute tables retain their format 3 meanings
and validation. Empty meshes can use empty JSON edge and face lists.

Points are triples in metres. A block with even one coordinate that changes
on conversion to IEEE binary32 uses `<f8` for the whole point array.
Other blocks may use `<f4`. Placement `xform` is a column-major list of
16 binary64 values encoded as round-trippable JSON numbers. Element keys
are session-local. Later versions may introduce additional geometry arrays
and metadata.

The IngeTrazo compatibility export writes a format 3 ZIP using binary32
points. Its precision warning reports the largest Euclidean displacement
of a placed mesh point in world coordinates, in millimetres when it exceeds 0.0254 mm. Opening existing
IngeTrazo formats 1, 2 and 3 follows their existing rules.

## Storage measurement

Level-1 DEFLATE on a synthetic 300 × 300 point grid with horizontal and
vertical edges and 89,401 unit quads produced 1,119,670 bytes with binary32
points and 1,118,540 bytes with binary64 points. The same test on one unit
quad produced 443 and 445 bytes respectively. Sizes include the ZIP and the
minimal scene manifest. Repeated integer coordinates compress well at either
width. The writer uses binary32 for exactly representable blocks to halve the
uncompressed point payload and retain the existing fast deferred-load path.
