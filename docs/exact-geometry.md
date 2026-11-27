# Exact geometry in Built

Design for owner review, against `036645d52`. A mitred pipe needs just two cylinders and two trimmed planes. The sloping rim is an ellipse shared by a wall and an end face. Keeping only the cylinder radius, or attaching a cylinder to every triangle, cannot reconstruct that solid after the end is moved.

## 1. Ownership and invariants

Built owns one topology. Definitions hold vertices, edges and faces. Placements transform definitions. Add exact supports to logical edges and faces. Picking, snapping, tools and the renderer keep their existing entry points. Internally they must map display triangles and sampled boundary segments back to the logical face, edge and occurrence. Tessellation vertices have no persistent identity. An edit to a shared definition affects every placement.

An exact face needs **a support, a sense and ordered trimming loops**. Vertex rings alone cannot distinguish a full circular edge from a zero-length edge, two parameter-space branches of a seam, or the pole of a sphere. Today store/access.rs exposes face_rings as vertex slots, and find_edge(a,b) returns one edge joining the pair. Distinct curved edges may share both endpoints. For exact operations, resolve incidence by edge id and use, never by endpoint pair alone. Extend the topology of exact faces to admit repeated oriented edge uses, closed edges with one endpoint, and singular vertex bounds. One oriented use records the persistent edge id, forward/reverse sense, its 2D curve on this face (pcurve), parameter range and periodic branch. Keep outer vs inner loop identity. Model a seam twice on the same face with distinct pcurves. Store face and edge orientation independently of loop order. Support a shell's inward-facing voids and open shells explicitly. Existing planar rings can supply this view implicitly where unambiguous.

Store f64 in definition-local model units, with explicit unit conversion at exchange. Normalize directions and store a reference axis for parameter zero. Surfaces: plane, cylinder, cone (reference radius and signed half-angle), sphere, torus (two radii), tensor-product rational B-spline. Curves: line, circle, ellipse, rational B-spline. An arc is an oriented interval on a periodic circle or ellipse, with unwrapped parameters. For splines retain degrees, control-point order, distinct knots with multiplicities, weights and periodicity in each direction. Support linear extrusion and revolution of an exact generator as first-class records: they preserve design intent and parameterization for push/pull and Follow Me and avoid duplicating a control net. Intern generators as immutable rows and prohibit cycles.

**Admission test.** Require finite parameters, a valid domain and knot vector, valid weights, monotone edge intervals in their stored sense, closed non-self-crossing UV loops and consistent incidences. Check both endpoints and a bounded whole-interval deviation between each 3D edge and each lifted pcurve. Use analytic bounds or recursive spline bounds with a depth limit. A few coincident samples are insufficient. Use the representation's length uncertainty in model units, carry a bounded per-use tolerance and treat angle and parameter tolerances separately. Over-budget or ambiguous geometry gets a diagnostic. Never inflate a weld tolerance to close a shell.

**Editing boundary.** Introduce an exact-aware face boundary iterator in front of planar-only callers. Plane repair and planar drawing must bypass curved regions until the operation supports UV trimming. On an unsupported edit, transactionally materialize the affected patch's current conforming model-tolerance mesh as ordinary planar faces, then execute the existing operation. Reconcile shared edge splits and pcurves on both sides. Expand the patch only where the neighbour cannot represent the new boundary. Dropping a flag on a nonplanar ring would leave an invalid polygon. Allocate identities by the same succession rules as any split. A tessellation failure leaves the previous state intact and returns the reason.

## 2. Storage and display

Use sparse sorted columns scoped to (definition uid, kind, packed element id). Keep document-local u32 indices into immutable, type-tagged geometry pools. Never send a pool index as an identity. The existing i128 `edge_curve` groups connected segments (`store/curve_groups.rs`), including legacy groups without a known analytic equation. Retain that column and its origin-0 migration. A separate edge binding supplies the exact support and interval. Reuse validated source arc parameters where available. A group number alone cannot establish circularity.

For format 2 propose `doc.ext["built.exact_geometry"]` describing version, units, pools and binary members. Geometry blocks optionally carry paired arrays `face_surface_id/ref` and `edge_exact_curve_id/ref` (id = packed u64, ref = u32). Additional sparse columns encode sense, edge interval, face loop ranges and coedges. All arrays are little-endian with checked lengths, range offsets, kinds and references. Keep a stable pool index while any deferred or raw-copied block refers to it. Compact only on a whole-document rewrite that can remap every live reference. Save atomically. Foreign documents remap both origins and geometry pools before binding.

**Request to the persistent-id lane.** At `feat/persistent-ids` revision `e66032626`, §§5.2–5.5 specify id arrays, extension maps, raw copying and `requires`. They do not name the proposed surface/trim arrays. Reserve the namespace, these array families and the critical feature `built.exact_geometry.v1` together. Files carrying any such data set the feature so an older editor refuses before it can drop bindings. Ordinary format-2 files omit the feature and all its arrays. If duplicate ids are repaired, drop and report any ambiguous binding. Keep zero migrations for legacy faceted blocks. Include added/removed bindings, coedges and referenced geometry values in the net patch and undo snapshot; snapshot the pool root with all definition states. Handle a dangling or malformed critical pool as file damage before applying edits. Coordinate the names with any reservation made after this revision.

The resident pack caches view-independent model-tolerance samples for tools and export fallback. Tessellate each shared edge once per tolerance, then triangulate constrained loops in UV with holes and refine the interior for chordal and normal error. Reuse identical boundary vertices in both faces. Handle periodic cuts and poles explicitly. Keep screen-space refinement optional: quantize zoom into bands with hysteresis, cache by support + trims + transform scale + tolerance, build off-thread and swap adjacent patches together. A stale job cannot replace a newer revision. Reflection flips winding. Increasing zoom costs CPU, transient buffers and cache space; bound the cache and retain coarse picking while rebuilding. Snap to the exact curve or surface after broad-phase hit testing. Save topology and parameters independently of view zoom.

## 3. Edit contract

The persistent-id rules in §4 of the sibling design apply independently of exactness. Reusing a face id does not prove its surface survived. Publish one atomic transaction containing changed topology, geometry bindings, derived/merged ids and cache invalidations.

| Edit | Exact result and limit |
|---|---|
| Move, rotate, uniform scale | Transform the complete support, trim and endpoints once in local coordinates. A placement-only edit updates its matrix. For a partial vertex move, rederive and validate every incident support and trim, otherwise facet the affected patch. |
| Nonuniform scale or shear | Invertible affine maps preserve lines and planes; circles can become ellipses. A cylinder remains circular only when its transformed generators and cross-section admit that axis and radius. Otherwise use an exact affine image of rational spline patches, transforming homogeneous controls, with reparameterized trims. Split at patch boundaries as needed. Rebuild UV and face sense on reflection. Singular transforms enter the existing degeneracy path. |
| Paint, tag, hide, soften/smooth | Change attributes or shading normals only. A softened chain remains faceted unless an exact support already exists. Reverse face flips sense and oriented loops. |
| Line across curved face | Resolve the tool's intended path on the surface first. A straight generator can remain a 3D line. A drawn projected path needs a defined projection, continuous branch, exact 3D/UV correspondence and inside tests. Split loops in UV, reuse one support on both halves, share the new edge. Larger trimmed area inherits the id, creation order breaks ties. Reject ambiguous projection or use the documented faceted operation. A spatial chord through a cylinder is not a surface trim. |
| Push/pull | Translate a planar cap and its holes. Lines swept along a vector give plane walls, perpendicular circular rims give cylinders, other curves give linear extrusions. For a segmented chain with exact intervals, join only compatible uses. Keep base or translated cap id per §4. Push-through ends the removed id. Curved-face normal extrusion needs a separate offset-surface operation; use the faceted path where unsupported. |
| Offset | Constant-distance offsets in a plane preserve lines and circular arcs until intersection/collapse. Recompute joining trims and holes. Keep concentric cylinders/spheres for a proven constant normal offset over a regular domain. General spline offsets, cusps or changing topology require validated construction or faceting. Outer ring id survives. |
| Follow Me | Straight path gives extrusion. A profile rotating about a fixed axis gives revolution, including toroidal cases. Check frame law and mitres at joints. A general transported profile lacks that support; facet unsupported spans. Start cap inherits profile id. |
| Intersect faces | For supported pairs construct one 3D intersection curve plus both pcurves, split only the participating trims. Begin with plane/plane and regular plane/cylinder sections. Tangency, parallel/coincident regions, singularities and branch ambiguity require explicit classification. Triangle intersections alone cannot certify a curved trim. |
| Erase, merge, divide, weld | Erase releases bindings. Merge adjacent trimmed regions only on provably equivalent supports with a valid UV union (including seam shifts). Divide an exact edge by splitting its parameter interval and every incident use; first-end successor keeps the id. Join only contiguous compatible intervals. Check weld displacement against every incident trim. |
| Explode, copy, Make Unique | Explode bakes the placement by the affine rule, remaps binding ids on collision, reports copies. Make Unique clones definition bindings and keeps element ids under the new uid. Share immutable pool rows until edited. Loose copies get fresh ids. |
| Solid union, subtract, trim, intersect, split, outer shell | The mesh result has new elements, with support and paint donors (`rust/built_core/src/solids/output.rs`). Unchanged input definitions keep their supports. Result faces inherit a donor support when their entire boundary passes the same admission check as an edited facet. Other result faces remain faceted. Future exact booleans split coedge loops on intersection curves and classify shells. Fillets bind their round strips to cylinders (§10); variable-radius and spherical corner blends remain future supports through the same boundary API. |
| Import, export, undo | STEP creates fresh element ids per definition. Mesh-only imports keep the empty representation. Preserve verified analytic source curves on .skp import. Export reads a snapshot. Undo restores topology, bindings and pool together; redo never fits curves again. |

## 4. STEP exchange and legacy recovery

Read Part 21, resolve references in representation context, normalize length and angle units, then traverse product definitions and occurrence transforms. Map VERTEX_POINT, EDGE_CURVE, ORIENTED_EDGE, EDGE_LOOP, FACE_BOUND/OUTER_BOUND, ADVANCED_FACE, open/closed shells, MANIFOLD_SOLID_BREP and BREP_WITH_VOIDS to the above model. Follow SURFACE_CURVE, SEAM_CURVE and PCURVE as well as analytic and rational spline bases. Preserve the edge's same_sense, coedge orientation, bound orientation and face same_sense separately. Record original entity ids for diagnostics only. Resolve assembly placements without duplicating prototypes. Report unsupported entities by part, kind and source id, with counts for missing, tessellated and exact faces. A tessellated-only AP242 representation enters as facets. If neither a faithful exact representation nor a validated mesh exists, offer explicit partial import with the missing items listed.

Write one representation per definition and occurrences per placement. Write canonical exact supports, shared bounded edges, explicit oriented loops, shell senses, units, context uncertainty and product structure to AP242. Check seams, manifold incidence and shell orientation before claiming a closed solid. Write open sheets as shells. Use true piecewise-planar topology or an explicitly selected tessellated representation for legacy facets. Check completeness per body in mixed documents. A writer unable to represent a body reports it before replacing a destination.

**Recover exact geometry** is a separate undoable command. On selected faceted bodies, cluster coplanar patches, test coaxial cylinders and bores, identify rounded-rectangle profiles from tangent segments/arcs, and solve planar end cuts including oblique ones. Fit and validate in physical units with a user-visible tolerance. Compare bidirectional boundary/surface deviation, normals, shell closure, solid count and volume; reject ambiguous fits. Preview per-part method, support counts, max residual, input/output volumes and reasons for each untouched region. On accepted export reopen a temporary file, check validity, body count and relative volume error (CleanSTEP uses 1e−7 for export/reopen; use a separate absolute tolerance for near-zero volumes). Replace the destination only after validation. CleanSTEP's 0.1 mm fit, 1 mm sampled smoothing and 10% reconstruction-volume guard are separate checks on an approximation. They are not an exactness guarantee.

## 5. Incremental cost

These are layout estimates per **definition**, above the existing topology and persistent ids. Use packed structure-of-arrays for the proposed in-memory representation and binary members. A face binding costs 21 B (id 8, support 4, loop start/count 8, sense 1). An empty document has no pool, sparse rows, array directory entries or per-element bitmap. A side table on the document can provide this without enlarging each slot or loading deferred geometry.

| Support | Pool bytes for one unique row | One face, resident column bytes / raw file bytes |
|---|---:|---:|
| Faceted | 0 | 0 / 0 |
| Plane | 1 + 72 for origin, axis, reference direction | 94 / 94 |
| Cylinder or sphere | 73 + 8 radius | 102 / 102 |
| Cone or torus | 73 + 16 | 110 / 110 |
| Linear extrusion | 1 + 4 generator + 24 vector | 50 / 50 |
| Revolution | 1 + 4 generator + 48 axis line | 74 / 74 |
| Rational spline | 33 header + 32mn homogeneous controls + 12(k+l) knot values and multiplicities | 54 + 32mn + 12(k+l) each |

The spline example with 4×4 controls and two distinct knots per axis costs 614 B per unique bound face. Reusing any support costs just the next 21 B binding plus trims. Actual RSS includes allocation capacity, page rounding, indexes and active tessellations. Deflate size depends on the data. The table gives pre-compression file size and packed resident payload, not a prediction of compressed spline coordinates.

A synthetic column trial selected every hundredth of 4 M face ids (40,000 exact rows), one unique support per face, random origins/radii, common axis and zero angle reference, one loop each. Packing the id/ref/loop/sense and support columns and deflating each column at level 1 gave 31.33 B/plane, 39.18 B/cylinder or sphere, 47.19 B/cone and 46.97 B/torus, excluding loop and edge data. Other orientations or control nets change compression.

Budget separately for edge bindings (8 id + 4 support + 16 interval + 1 sense = 29 B), curve rows (line 49 B, circle 81 B, ellipse 89 B), loop headers (9 B) and coedges (8 edge id + 4 pcurve + 16 range + 8 period shifts + 1 sense = 37 B). Implicit planar pcurves save the 2D payload. An explicit UV line takes another 33 B, plus its reference already counted. A rational UV spline or a curve-on-surface takes variable storage. Generator rows are shared. These columns can be paged by definition with copy-on-write chunks. A lookup index, if built, scales with exact rows only.

For 5 M vertices, 9 M edges and 4 M faces, an entirely faceted ride has **0 extra element bytes** resident or saved. A dense u32 face+edge reference alone would cost 52 MB. As an upper illustration for a simple patch mix, attach 40,000 distinct cylinder supports, one four-use loop per face, two distinct circle edges per face on average and an explicit UV line per use. Packed resident and raw file cost = 40,000 × (102 + 9 + 4×(37+33) + 2×(29+81)) = 24.44 MB. Instances multiply none of this. Splines, long trims, locators and display caches can exceed this example. Gate phase 2 on synthetic ride geometry with 0%, 1% and clustered exact faces: measure actual allocator/RSS, compressed bytes, open/save, single local edit, cache high-water and undo. For 0%, require unchanged geometry members and zero allocations proportional to element count.

## 6. Reuse survey

Latest published crate versions observed in the registry on 2026-11-27. Licences below are from the particular published package's manifest and notices. Check the locked transitive graph and retain notices when adopting a crate. The age of the STEP adapter and the age of its repository can differ.

| Package / last release | Licence | Boundary for reuse |
|---|---|---|
| [truck-stepio 0.3.0](https://docs.rs/crate/truck-stepio/0.3.0), 2024-09-20 | Apache-2.0 | Exact shell conversion and STEP writer; truck-geometry 0.5 supplies rational splines, evaluation and transforms. Reuse the maths behind Built-owned value types. Meshing the released shell API needs unit, assembly and failure handling. Repository main has a newer assembly example. |
| [ruststep 0.4.0](https://docs.rs/crate/ruststep/0.4.0), 2024-09-20 | Apache-2.0 | Part 21 AST, EXPRESS tooling and typed schemas. Strong candidate for raw diagnostics and tolerant syntax intake. A parsed entity is not yet a mapped surface. |
| [step-io 0.4.0](https://docs.rs/crate/step-io/0.4.0), 2026-11-26 | MIT or Apache-2.0; generated entity model has separate ISO EXPRESS terms | First choice to prototype typed AP203/214/242 import and AP242 output. Exposes raw entities, exact supports, assembly handles and entity-drop reports. Surface topology must survive conversion into Built. Gate adoption on schema notices and malformed-header handling. |
| [stepmesh 0.1.0](https://docs.rs/crate/stepmesh/0.1.0), 2026-11-03 | MIT or Apache-2.0 | Placed solid meshes in mm, shared boundaries, bounded tessellation. Use for validation and as a faceted import option. Exact decoding is exposed as unstable internals. Require a stable face/edge provenance interface before using its output to drive editable exact topology. |
| [oxideav-step 0.0.1](https://docs.rs/crate/oxideav-step/0.0.1), 2026-10-06 | MIT | Reuses oxideav-ifc Part 21 and geometry code; handles swept, spline and AP242 tessellated input, colours and instances. Study its face provenance and shared sampling. Its scene writer emits tessellated AP242. |
| [vcad-kernel-step 0.10.0](https://docs.rs/crate/vcad-kernel-step/0.10.0), 2026-09-10 | MIT in released package | AP214 interchange, assembly reader, reconstruction into vcad B-reps. Study entity mapping and writer fixtures. Importing the whole kernel would duplicate Built's ownership. Release licence here applies to this crate version. |
| [springrep 2.1.1](https://docs.rs/crate/springrep/2.1.1), 2026-11-25 | MIT or Apache-2.0 | Exact safe-Rust B-rep, booleans, spline and analytic geometry, STEP AP214 read/write. Writer and independent differential oracle in the trial. Consider isolated analytic intersection routines after provenance review. |
| [kentron 0.21.1](https://docs.rs/crate/kentron/0.21.1), 2026-11-26 | AGPL-3.0-or-later | Kernel, STEP and CadQuery-oriented bindings. Reference only. Its release notice identifies OCCT-derived LGPL-2.1 code through 0.19.3, subsequent clean-room rewrites and a separate LGPL-derived test crate. Exclude those sources and tests from Built. |

## 7. Interoperability trial

Twenty unchanged public STEP files, none redistributed here. The table below names every input. The corpus deliberately includes assemblies and imperfect headers. It provides a smoke test, not a population estimate.

| Input | Source revision and rights |
|---|---|
| NIST_1 through NIST_4 | Files NIST_MBE_PMI_1 through 4, AP203 conversions, ST-DEVELOPER v15 header. BRL-CAD's accompanying notice puts both these conversions and the source models in the public domain. [Pinned directory and notice](https://github.com/BRL-CAD/brlcad/tree/cb1311a91fc5d512b5703cd77640d88f040eb690/db/nist) |
| abc-0000, 0006, 0008, 0035 | Koch et al., *ABC: A Big CAD Model Dataset*, CVPR 2019. Downloaded from truck's fixture mirror. Its README says MIT, while the dataset authors assign copyright to individual creators and refer to Onshape terms. Treat redistribution as unverified. [Fixtures](https://github.com/ricosjp/resources4truck/tree/9bf9de1426c5fc2f2b8d63f501dca0d3c53ebd91/step) · [Dataset terms](https://deep-geometry.github.io/abc-dataset/) |
| occt-assy, cone, cube, cylinder, sphere, torus | FreeCAD/OCCT fixtures in the same pinned directory. Five primitive fixtures omit FILE_NAME. No separate asset grant identified for these six. [Fixture provenance](https://github.com/ricosjp/resources4truck/blob/9bf9de1426c5fc2f2b8d63f501dca0d3c53ebd91/Readme.md) |
| as1-oc-214, dm1-id-214, io1-cm-214, sg1-c5-214 | STEPcode AP214 examples from Open CASCADE/Datakit, I-DEAS, CoCreate and CATIA respectively. Collective work BSD-3-Clause; fixture-specific rights unconfirmed. [Pinned files](https://github.com/stepcode/stepcode/tree/24783b448511b8f6bce2829286a3f00015430236/data/ap214e3) |
| block, door | FreeCAD Parts Library, Architectural Parts/Construction blocks/Concrete canal block 14x39x19.step and Architectural Parts/Doors/Generic/Simple door.step. CC BY 3.0, individual part authors. [Pinned library and asset licence](https://github.com/FreeCAD/FreeCAD-library/tree/abe9eaf5243196137c68b25dda740b30dae6a423) |

**Method.** Apple M5 Max, arm64, Rust 1.100 release binaries, one fresh process per file per reader. Wall time includes startup, file read, operation and diagnostic checks. Peak measured RSS from macOS time -l. One pass per reader under other workstation load. Reference uses CadQuery 2.8 / OCCT 7.9.3, transfers all roots, groups all returned shapes and tessellates each solid. Reference time and RSS include Python and CadQuery startup. The initial reference harness selected only the first returned shape. The reported results use the corrected all-roots run for those files.

Stepmesh, oxideav and springrep requested 0.05 mm linear deflection and 0.3 radians. OCCT used CadQuery tessellate(0.05, 0.3), whose meshing defaults differ. Truck 0.3 converted raw unplaced shells and robust-triangulated at 0.05 in native coordinates, two Rayon threads and an 8 s/file budget after one initial run exceeded 3 GiB. Its timing, scale and unit of topology differ from assembly-aware readers. Springrep had 120 s/file. A timeout counts as unsuccessful; no peak RSS is available for that run.

For stepmesh, require two oppositely directed uses of every triangle edge per placed solid. For OCCT and oxideav, weld by rounded position at 1e−7 mm per solid/shape first and skip collapsed triangles. Truck uses its own Closed test after 1e−7 welding and requires a meshed surface on every face. Springrep uses per-solid boundary, nonmanifold, shared-edge, orientation and missing-face reports. These checks test closure of returned geometry. They cannot alone detect a missing body, displaced occurrence or bad volume. Parser-only results have no mesh denominator.

| Reader / path | Nonempty loads / 20 | All returned meshes closed / 20 | Median successful wall ms | Max recorded RSS MiB |
|---|---:|---:|---:|---:|
| ruststep, syntax | 20 | — | 15.5 | 18.2 |
| step-io, typed | 15 | — | 13.5 | 11.6 |
| stepmesh | 20 | 20 | 19.9 | 112.7 |
| oxideav-step | 15 | 11 | 32.1 | 305.4 |
| truck-stepio + meshalgo | 14 | 11 | 37.5 | 278.1 |
| springrep | 18 | 18 | 16.0 | 152.8 |
| OpenCascade reference | 20 | 20 | 3514.8 | 604.4 |

Per-file results below. Mesh columns show counts of solids, shapes or shells according to the API. An asterisk means at least one returned mesh failed closure. T denotes timeout, E denotes error. The reference and stepmesh columns count placed solids. Truck and oxideav count definition geometry.

| File | Reference solids | stepmesh | oxideav | truck | springrep | step-io entities kept/read |
|---|---:|---:|---:|---:|---:|---:|
| NIST_1 | 1 | 1 | 1 | T | 1 | 4113/4113 |
| NIST_2 | 1 | 1 | 1∗ | T | 1 | 24236/24236 |
| NIST_3 | 1 | 1 | 1 | T | 1 | 4740/4740 |
| NIST_4 | 1 | 1 | 1∗ | T | 1 | 21499/21499 |
| abc-0000 | 6 | 6 | 6∗ | E | 6 | 863/863 |
| abc-0006 | 1 | 1 | 1 | 1 | 1 | 595/595 |
| abc-0008 | 1 | 1 | 1 | 1∗ | 1 | 699/699 |
| abc-0035 | 1 | 1 | 1 | 1 | T | 3137/3137 |
| as1-oc-214 | 18 | 18 | 5 | 5∗ | 18 | 6425/6425 |
| block | 1 | 1 | 1 | T | 1 | 862/864 |
| dm1-id-214 | 7 | 7 | 3∗ | 3∗ | 7 | 1189/1189 |
| door | 4 | 4 | 4 | 4 | 4 | 2529/2530 |
| io1-cm-214 | 1 | 1 | 1 | 1 | 1 | 917/917 |
| occt-assy | 5 | 5 | 2 | 2 | 5 | 315/315 |
| occt-cone | 1 | 1 | E | 1 | 1 | E |
| occt-cube | 1 | 1 | E | 1 | 1 | E |
| occt-cylinder | 1 | 1 | E | 1 | 1 | E |
| occt-sphere | 1 | 1 | E | 1 | 1 | E |
| occt-torus | 1 | 1 | E | 1 | 0 | E |
| sg1-c5-214 | 1 | 1 | 1 | 1 | 1 | 460/460 |

**Coverage and failure modes.** ruststep parsed 73,283 DATA records across 20 files. On the 15 accepted by step-io, its report kept 72,579 of 72,582 entities. Its three drops were PRODUCT_TYPE metadata (block #24 and #853, door #2276). The five missing FILE_NAME headers explain all five typed-reader failures; oxideav rejected the same headers. Retention does not establish semantic fidelity. Across the corpus, STEP entity spellings occur in this many files: planes 17, cylinders 8, cones 5, spheres 2, tori 4, B_SPLINE_SURFACE_WITH_KNOTS 7, PCURVE 6 and SEAM_CURVE 2. No file exercises SURFACE_OF_REVOLUTION, SURFACE_OF_LINEAR_EXTRUSION or BREP_WITH_VOIDS. Those are release-gate fixtures to add. The trial covers AP203 and AP214; AP242 output, PMI and external references need independent fixtures.

Stepmesh matched the reference's placed-solid counts on every file. Comparing summed triangle volumes with exact reference volumes, the largest relative discrepancy was 1.4854% on the cone and cylinder primitives at these display tolerances. Use analytical geometry for measurement. Oxideav returned open or inconsistently oriented output by the stated test on NIST_2, NIST_4, abc-0000 and dm1-id-214. Truck timed out on all four NIST files and the canal block and panicked on the tolerance/radius relationship on abc-0000. Springrep timed out on abc-0035 and rejected the torus solid even though the file read completed. A zero-body import is a failure. In a separate header-only diagnostic, inserting one synthetic FILE_NAME into temporary copies of the five primitive fixtures let step-io load all five as one solid each and retain all 701 DATA entities (129 + 361 + 129 + 41 + 41). The matrix reports the unchanged inputs.

**Rust writer check.** In a standalone temporary Rust program using springrep 2.1.1, subtract a coaxial radius-7 bore from a radius-10 cylinder, then subtract a large cylindrical tool whose flat base lies on z = 40 + 0.5x, in mm. Write AP214 with the crate's native writer. OpenCascade reopened one valid closed solid, four faces. The reopened face types were two cylinders and two planes. The six unique edges comprised two circles, two ellipses and two seam lines. Expected volume is π(10²−7²)40 = 6408.849013323 mm³ since the odd term integrates to zero over the annulus. Reopened volume was 6408.849004208 mm³, relative error 1.42×10⁻⁹. This validates that writer and geometry construction for this part. It does not exercise a future Built AP242 writer or legacy fitting.

## 8. Decisions for the owner

| Question | Recommendation |
|---|---|
| Which dependency first? | Prototype the step-io typed adapter plus truck's spline maths. Use ruststep to diagnose incomplete headers. Permit a narrowly defined, reported synthetic FILE_NAME only on a derivative parse buffer; preserve source entity ids. Keep stepmesh for independent mesh comparison. |
| What can ship as exact? | Start with elementary supports plus rational spline interchange and validated UV trimming. Include extrusion and revolution tags immediately. Advertise individual editing operations only when their trim and boundary checks pass. |
| How should drawing on a cylinder work? | Use the existing hit point for selection, then offer a surface-projected path with a visible preview. Require an unambiguous branch. A free-space chord remains a distinct operation. |
| Are old-format writers allowed to discard exactness? | Gate format 2 through requires and provide explicit faceted-copy export for compatibility. Reference an immutable model-tolerance fallback when an operation needs to materialize. |
| Is best-effort import acceptable? | Report exact, faceted, rejected and missing bodies separately. Require explicit acceptance for a partial document. A zero-solid result for a declared solid is an import error. |
| How much zoom refinement? | Begin with a stable model tolerance, measure close-up faceting and cache pressure, then add bounded view-dependent refinement. |
| Should we adopt a second B-rep engine? | Keep Built's topology authoritative. Use independent engines only in differential tests. The tube demonstrates useful Rust output, while the reader matrix shows why a single success flag cannot qualify a dependency. |

## 9. Phase 2 lanes after approval

1. **Identity and file contract.** Agree the feature names and critical-reader behavior with the persistent-id lane first. Add sparse bindings, immutable geometry, coedges and snapshots. Gates: v1 migration, empty faceted save, seam-with-two-uses fixture, repeated endpoints, deferred raw-copy, undo/redo, cross-document paste and reject damaged pools.
2. **Evaluation and tessellation.** Implement/support evaluators, shared edge sampling, UV constraints and cache invalidation. Test periodic seams, pole holes, narrow trims, tiny and mixed-unit solids, mirrored nonuniform instances, finite bounds and two-use closure. Check model tolerance independently from render tolerance.
3. **Editing.** Connect drawing, push, offset, sweep and intersection donor paths to the transition table. Cover each row with keep, re-trim and fallback cases. Assert orientation, ids, unchanged neighbours, atomic failure and exact undo.
4. **Exchange.** Map source topology into the store and export canonical AP242. Build a redistributable corpus from cleared assets plus authored adversarial fixtures. Compare part and occurrence counts, bounds, curve types, shells, orientation and exact volumes against independent readers. Test mixed assemblies, unit conversion and malformed headers before picking a production reader.
5. **Recovery and scale.** Implement the explicit fit/report command separately. Exercise rounded rectangles with holes, interrupted cylinders, planar mitres and false-positive noisy meshes. Run the storage and latency gates in §5. Later curved booleans and fillets use the new intersections, oriented coedges and trimming API.

## 10. Editing as built (lane 3)

Drawn geometry keeps its conforming facet topology, and each element carries an exact binding beside it (`store/exact.rs` `Curved`, sparse and sorted by slot). An edge carries a curve and the parameter interval from its first to its second end. A face carries a curved support (cylinder, cone, sphere, torus, linear extrusion, revolution); a planar face needs no binding, its plane is its own and its curved sides come from their edges. Pcurves are implicit: a face's trims are its store edges, mapped into the support's parameters by analytic inverses (`exact/bound.rs`). A curve lies on a support only if `deg(curve)·deg(support)+2` distinct points of the whole curve do, by Bezout, so tolerances are never inflated. The facets are always the model-tolerance mesh, so dropping a binding is the fallback, it never fails the step, and undo restores bindings with the state.

The segment count determines the geometry of a circle, arc or curve. Display, selection, highlighting and snapping use the same stored segments and facet rings. At a smooth corner of a facet with a curved support, `display_arrays` supplies the exact support normal for shading. A fillet's tangent lines are soft with smooth normals off, so no bake averages a flat face with the strip beside it; the strip's corners on such a seam take the support normal, which equals the plane's. A planar face across a smooth edge from a curved support (a seam smoothed by hand, the sides of a rounded slot) keeps its own normal at those corners for the same reason. Hard edges and edges softened with smooth normals disabled retain flat shading. Native STEP topology with analytic faces still generates its own mesh.

Every History step runs a rebind pass after persistent-id succession (`store/exact_bound.rs`):

1. a created element whose id was inherited or derived takes the parent's binding;
2. bound elements whose vertices moved follow the affine map fitted to every vertex of their support (a partial move fits no map and drops what no longer holds);
3. new edges are derived: circles fitted to the curve runs the step drew, when the command asks for it (Circle, Arc, Follow Me, Offset, Resegment); a copy of a bound edge at its old place; plane sections of a bound cylinder; turns about a Follow Me axis; mitre sections at Follow Me joints;
4. new faces are derived: an extrusion or cylinder of a bound side along an adjacent straight side, a revolution of a side about a bound circular side's axis, binding translated or rotated rails on the way;
5. every touched binding is validated against its element and dropped if it does not hold. Supports equal to roundoff are interned, so one surface has one row.

Bindings live in copy-on-write chunks of 512 slots (`store/exact.rs` `ChunkMap`): a snapshot shares every chunk, a step that edits a few bindings copies only their chunks, and two states compare chunk by pointer. The pass reads only what the step touched: parents among the removed or rewritten elements, neighbours for interning. With 4 M face slots and 1 % bound (40,000 faces), a snapshot plus one binding edit takes 53 µs distributed and 2 µs clustered, the state diff 31 and 17 µs, against 2.2 ms for a full copy of the map (`curved_cost` test, release build, M5 Max).

Line draws straight segments. A segment inside one facet can divide it, and a segment across facets forms a chord. Line on Surface follows a circle or generator when the stroke lies within 10° of that parameter direction. Its preview follows the support and its endpoints snap to facet generators.

Weld Edges groups a single connected open or closed edge chain. A circular chain also receives exact intervals. Erasing a branch rejoins compatible pieces at a valence-two junction and keeps the longer piece’s persistent id. Temporary vertices inside curve segments disappear. An original arc corner keeps its vertex and recovers its curve group. The file records those junctions by persistent vertex and edge ids. Distinct edge flags, tags, or original face attributes at a dissolved seam preserve the boundary corners.

Commands state their intent through `exact_bound::with_intent`: fit runs, keep chords, project divided vertices, keep the base face. Bindings save in the format-2 exact pool (`ext["built.exact_geometry"].curved`, one JSON row per block, element ids in file space; block array `face_surface_curved`), set the critical feature, and pickle after the store.

| §3 row | Status | Tests (`tests/test_exact_edit.py`) |
|---|---|---|
| Circle, arc | Exact circle per run; Polygon stays straight; Resegment keeps the circle | `test_circle_*`, `test_arc_*`, `test_resegmenting_*` |
| Move, rotate, uniform scale | Supports transform; partial moves re-trim on the surface or fall back | `test_transforms_carry_the_support`, `test_vertex_moved_along_*`, `test_partial_vertex_move_*` |
| Nonuniform scale | Circles become ellipses; a cylinder stays one while its right section is round, else an elliptic extrusion. Spline images are not built (fallback) | `test_nonuniform_scale_*` |
| Paint, tag, hide, soften | Attribute-only, bindings untouched | (existing suites) |
| Line across curved face | Line draws a straight chord. Line on Surface previews and commits a parameter line with endpoints on generators | `test_line_around_a_cylinder_*`, `test_chord_mode_*`, `test_line_across_disk_*` |
| Push/pull | Planar caps translate, walls from circles are one cylinder, from other curves extrusions; keep_base keeps the base id; push-through ends the id | `test_push_pull_*`, `test_second_push_*`, `test_push_through_*` |
| Offset | Concentric circles on planar faces; outer face keeps its id | `test_offset_*` |
| Follow Me | Straight path: extrusion; about an axis: cylinder, cone, plane, torus; mitred joints bind the elliptic section, each span its own cylinder | `test_follow_me_*` |
| Intersect faces | Plane/cylinder sections: circle or ellipse, both halves keep the cylinder | `test_intersect_cylinder_with_a_plane` |
| Erase, merge, divide, weld | Erase releases, undo restores; merge keeps the survivor's binding; divide splits the interval and puts new vertices on the curve where faces stay planar, else falls back | `test_erase_*`, `test_line_across_disk_*`, `test_divide_*` |
| Explode, copy, Make Group | Bindings follow the placement's affine map onto the copies; Make Group keeps element ids (persistent-ids §4) | `test_make_group_*` |
| Fillet | The section arcs of a round profile are circles; each strip between them is one cylinder, so STEP gets a real fillet (redo derives them again from the step's intent); tangent lines are soft with smooth normals off; other profiles (steps, mouldings, profile value ≠ 0.5) and corner patches stay faceted; Chamfer is planar with hard seams | `test_fillet_recover.py` |
| Solids | Manifold face ids identify the donor. An output edge inherits a trim on its donor interval, or a certified plane section of a cylinder. Result faces retain their donor support when all vertices and trims satisfy admission. Cut boundaries stay hard. | `test_curved_solid_tools.py` |
| Import, export, undo | Undo/redo exact; save/open/pickle round trip. STEP and .skp are lane 4 | `test_save_open_*`, undo asserts throughout |

Display tests compare the drawn positions, edge flags and rings with the editable mesh on a torus, cone, sphere, cylinder and mitred pipe. Separate tests cover native STEP seams and shading.

Known costs and limits: spline images under shear stay faceted. Helical strokes more than 10° off the local circle or generator intentionally form facet edges. Use Soften / Smooth Edges to hide those edges and blend shading. The `.skp` source arcs of the exchange lane (`store/source_arcs.rs`) are a second edge-curve column beside `curved`. Both columns are saved independently.
