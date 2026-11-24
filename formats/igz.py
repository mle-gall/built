# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Native IngeTrazo document format (``.igz``).

Plain JSON, schema-versioned. Trivial to inspect, edit by hand, and diff
in source control. Will grow as new entity types (faces, groups,
components, materials) land — old documents must keep loading.

Layout::

    {
      "igz_format": 1,
      "app_version": __version__,
      "scene": {
        "edges": [
          {"a": [x, y, z], "b": [x, y, z]},
          ...
        ],
        "faces": [
          {"vertices": [[x, y, z], ...]}
        ]
      }
    }

**Textured documents are ZIP containers** (format 2). A texture is an image
*file*, so a plain-JSON document could only point at one — an absolute path
that broke the moment the ``.igz`` travelled to another machine. When the scene
uses textures the document becomes a ZIP holding the very same JSON as
``document.json`` plus the images under ``textures/``, and each texture entry
carries ``"embed": "textures/<hash>-<name>"`` instead of ``"path"``. The
document is then **self-contained and portable**. Opening one unpacks the
images into the app's texture cache (content-addressed, so documents sharing an
image share the file and the GPU upload) and points the entries back at real
paths — the rest of the app keeps seeing ``attrs["texture"]["path"]`` and knows
nothing about containers.

Untextured documents stay **plain JSON at format 1**: the common case remains
diffable, hand-editable, and readable by older builds. ``load_into`` sniffs the
ZIP magic, so both shapes open transparently.

**Large documents are format 3** (see ``_LARGE_DOCUMENT``): always a ZIP, and
each big mesh is a ``{"geom": {...}}`` block instead of ``edges``/``faces``
lists — its distinct points once as float32, edges and face rings as int32
indices, edge flags and a shared table of face attributes, in one raw
member under ``geom/`` (layout in the JSON, no pickles). Builds that read
only up to format 2 refuse it with a message instead of opening it empty.
"""
from __future__ import annotations
from contextvars import ContextVar

from contextlib import contextmanager
import json
from types import SimpleNamespace
import math
from pathlib import Path

from core.version import __version__
from formats import built
from formats.igz_stream import Fragment, SaveStream
from formats.native_io import DocumentArchive, native_io

from ingetrazo_core import (
    igz_geometry_arrays as _geometry_arrays,
    igz_document_geometry_arrays as _document_geometry_arrays,
    igz_load_geometry as _load_geometry,
    igz_build_geometry as _build_geometry,
    igz_load_mesh as _load_mesh,
    igz_load_mesh_json as _load_mesh_json,
    igz_without_nonfinite as _without_nonfinite,
    igz_normalize_text_curve_ids as _normalize_text_curve_ids,
    igz_store_columns as _store_columns,
    igz_write_geometry as _write_geometry,
    igz_pack_plan as _pack_plan,
    Vec3 as QVector3D,
    igz_in_worker as _in_worker,
    igz_decode_document as _decode_document,
    igz_parse_native as _parse_native,
    igz_parse_document as _parse_document,
    igz_read_document as _read_document,
    igz_read_members as _read_members,
    igz_save_scene_inner as _save_scene,
    igz_prepare_and_write as _prepare_and_write,
    igz_spool_json as _spool_json,
    igz_write_prepared as write_prepared,
    igz_write_prepared_inner as _write_prepared,
    igz_texture_slots as _texture_slots,
    igz_texture_entries as _texture_entries,
    igz_pack_textures as _pack_textures,
    igz_unpack_textures as _unpack_textures,
    igz_archive_scene_fields, igz_reset_scene, igz_restore_views, igz_finish_scene,
    igz_pack_geometry as _pack_geometry,
    igz_plan_json as _plan_json,
    igz_payload_from_arrays as _payload_from_arrays,
    igz_mesh_json as _mesh_json,
    igz_face_json as _face_json,
    igz_edge_json as _edge_json,
    igz_face_attrs_json as _face_attrs_json,
    igz_face_attrs_from_json as _face_attrs_from_json,
    igz_load_mesh_small as _load_mesh_small,
    igz_json_arrays as _json_arrays,
)

from core.group import Group
from core.mesh import Mesh


PLAIN_FORMAT = 1        # bare JSON document (no images to carry)
CONTAINER_FORMAT = 2    # ZIP container: document.json + textures/
GEOMETRY_FORMAT = 3     # + big meshes as binary blocks under geom/
CURRENT_FORMAT = GEOMETRY_FORMAT   # the newest this build reads
_ZIP_MAGIC = b"PK\x03\x04"
_DOC_ENTRY = "document.json"
_TEX_PREFIX = "textures/"
# Fixed timestamp for every archive member: two saves of the same scene must
# produce byte-identical files (diffable documents, and no pointless churn in
# whatever the user syncs them with).
_ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)

# A document with more faces + edges than this is written COMPACT (no
# indentation) and deflated at level 1. Indented, the 14 M-face model of
# #158 was 3.3 GB of text — every coordinate on a line of its own — and
# saving it peaked at 16 GB of memory; compact it is 1.4 GB, and level 1
# deflates it in 4 s instead of 20 (the file grows ~35 %). Nobody reads a
# document that size by hand, while a small one stays indented and
# diffable, at the default level where deflating costs nothing anyway.
_LARGE_DOCUMENT = 20_000

# Format 3 — a LARGE document (above) stores each mesh of at least this many
# corners as a binary block instead of JSON lists: every distinct point once
# as float32 (exactly what the QVector3D of the live mesh holds, so nothing
# is lost), and edges and faces as int32 indices into it. On the #158 model
# the JSON spelled out 19 M points to describe 2 M distinct ones, and
# parsing that text was half of the minute it took to open. Structure —
# groups, layers, materials, the face attributes — stays JSON. Small meshes
# stay JSON too: the reader rebuilds a small mesh entity by entity, a big
# one in one bulk pass (see :func:`_bulk_sized`), and the two number their
# vertices differently — a mesh goes binary only where the JSON reader
# would take the bulk pass, so either way it reopens the same.
_GEOM_PREFIX = "geom/"
_BULK_MIN = 1024
#: A document this long has its mesh blocks read by the engine (_parse_native).
_NATIVE_JSON_MIN = 32 << 20
#: The dtypes a geometry block may declare — anything else is refused, so a
#: crafted document cannot make the reader build arbitrary objects.
_GEOM_DTYPES = frozenset(("<f4", "<i4", "<i8", "|u1"))


# Keys the texture walk never needs to enter: pure coordinate blocks, and
# the edge list (fixed ``_edge_json`` schema — no attrs, no textures).
# Descending into every vertex triple and edge dict made the walk itself
# cost seconds on a 300k-face document.
_TEXTURE_WALK_SKIP = frozenset(("vertices", "holes", "a", "b", "edges",
                                "arrays"))


def _group_ifc_record(group):
    """Keep serializable fields from script-supplied group records."""
    import logging
    record = getattr(group, "ifc", None)
    if isinstance(record, str):
        return {"class": record.strip()} if record.strip() else None
    if record is None:
        return None
    if isinstance(record, dict):
        clean = {}
        for key, value in record.items():
            try:
                json.dumps({key: value}, allow_nan=False)
            except (TypeError, ValueError, RecursionError):
                logging.getLogger(__name__).warning(
                    "Skipping invalid IFC field %r on group %r", key, group.name)
            else:
                clean[key] = value
        return clean
    logging.getLogger(__name__).warning(
        "Skipping invalid IFC record on group %r", group.name)
    return None


_attrs_key = json.JSONEncoder(sort_keys=True, separators=(",", ":")).encode


def _bulk_sized(n_edges: int, n_faces: int) -> bool:
    """Whether a mesh this size is rebuilt in one bulk pass (a rough corner
    count) — the same test for the JSON reader and the format-3 writer."""
    return n_edges + n_faces * 4 >= _BULK_MIN


def _backup_previous(path: Path) -> bool:
    """Keep the previous explicit save, including the headless saves that
    missed the ride-model agent's backup on 2026-11-09."""
    import shutil
    from tempfile import TemporaryDirectory
    from core import general_settings

    if not path.is_file():
        return True
    if str(general_settings.value("backup", "1")) == "0":
        return True
    try:
        with TemporaryDirectory(dir=path.parent) as tmp:
            copy = Path(tmp) / "previous"
            shutil.copy2(path, copy)
            copy.replace(path.with_name(path.name + ".bak"))
    except OSError:
        return False
    return True


def save_scene(scene, path: Path, *, backup=True, progress=None) -> dict:
    """Synchronous explicit save; autosave drives the same preparation in slices."""
    import gc

    enabled = gc.isenabled()
    gc.disable()
    try:
        return _save_scene(scene, path, backup=backup, progress=progress)
    finally:
        if enabled:
            gc.enable()


def export_igz(scene, path, confirm_loss, *, progress=None):
    """Write a compatibility copy. The callback receives maximum displacement in mm."""
    return _save_scene(scene, path, backup=True, compatibility=True, confirm_loss=confirm_loss,
                       progress=progress)


def prepare_scene_steps(scene, mesh_steps=None, *, stream=None, large=None, progress=None):
    """Yield between model blocks, returning an unencoded document payload."""
    # Accept a Scene or a bare Mesh (the M1 read-compat path feeds a Mesh).
    mesh = scene.mesh if hasattr(scene, "mesh") else scene
    # Decided BEFORE anything is serialized: a large document writes its
    # big meshes as binary blocks (format 3) and never builds their JSON.
    if large is None:
        large = mesh_steps is not None or _large_document(scene)
    geom = stream if stream is not None else ({} if large else None)
    placements = built.export_placements(scene) if getattr(stream, "compatibility", False) else None

    def file_block(m):
        if placements is not None:
            stream.current_placements = placements.get(id(m), ())
        block = _mesh_json(m, geom)
        if stream is not None:
            for key, value in block.items():
                block[key] = _spool_json(value, stream)
        return block

    def block(m):
        if mesh_steps is not None:
            return (yield from mesh_steps(m))
        return file_block(m)

    payload = yield from block(mesh)
    groups = getattr(scene, "groups", None)
    kept = getattr(scene, "kept_components", None)
    if groups or kept:
        # Component prototypes: each shared instance mesh is written ONCE —
        # including the ones only NESTED placements reference, which is what
        # keeps a component's internal repetition (the hedge: 9600 faces
        # placed 48 times) from being written out 48 times over.
        proto_index: dict = {}
        protos: list = []

        from core import links as _links

        io = native_io()
        precise = mesh_steps is not None or getattr(stream, "built", False)

        writer_type = getattr(io, "FileGroupWriter", None)
        writer = None
        if writer_type and mesh_steps is None and getattr(scene, "_root", None) is not None:
            destination = stream.destination if getattr(stream, "compatibility", False) else None
            writer = writer_type(scene, file_block, precise, destination)
            protos = writer.protos

        def _entry(g):
            if writer is not None:
                row = writer.entry(g)
                yield
                return row
            direct = io is not None and getattr(g, "_doc_key", 0)
            xf = g._file_xform(precise) if direct else getattr(g, "xform", None)
            # A linked file (unpacked) is saved as an empty placement with
            # its link: the source holds the geometry (core/links.py).
            placeholder = xf is not None and _links.save_as_placeholder(
                g, scene)
            if xf is not None:
                mesh = _empty_mesh() if placeholder else g.mesh
                idx = proto_index.get(id(mesh))
                if idx is None:
                    idx = proto_index[id(mesh)] = len(protos)
                    protos.append(None)          # reserve: recursion is depth
                    protos[idx] = yield from block(mesh)
                values = (xf if direct else built.placement_values(xf) if precise
                          else built.compact_values(xf.data()))
                entry = {"proto": idx, "xform": values}
                # Group or component (issue #90): a group of groups and a
                # copied group carry a matrix too. Written for EVERY
                # instance — a file without the key is an older one, which
                # the reader tells apart its own way. Older readers ignore it.
                entry["component"] = bool(getattr(g, "component", True))
            else:
                entry = yield from block(g.mesh)
            if direct:
                entry.update(g._file_fields(_group_ifc_record(g) or None))
            else:
                # The name — never written until 2026-09-11, so every reopened
                # document renumbered its groups and a "Pérgola" came back as
                # "Group 7". Older readers ignore the key.
                if getattr(g, "name", None):
                    entry["name"] = g.name
                axes = getattr(g, "axes", None)
                if axes is not None:
                    # A classic group's own axes (issue #44), column-major like
                    # "xform"; older readers ignore the key.
                    entry["axes"] = list(axes.data())
                if getattr(g, "layer", None) is not None:
                    entry["layer"] = g.layer
                if ifc := _group_ifc_record(g):
                    entry["ifc"] = ifc
                if getattr(g, "billboard", False):
                    # True = legacy textured-quad face-me; "mesh" = imported
                    # silhouette whose real geometry turns toward the camera.
                    entry["billboard"] = g.billboard
                if getattr(g, "material", None):
                    # The container's own paint (issue #47); older readers
                    # ignore the key and show the faces in the default.
                    entry["material"] = dict(g.material)
                if getattr(g, "text3d", None):
                    # A 3D text keeps what it was made from, so it reopens
                    # editable. Older readers ignore the key.
                    entry["text3d"] = dict(g.text3d)
                if getattr(g, "ext", None):
                    # Extensions' parameters for this container (JSON-safe by
                    # contract); older readers ignore the key.
                    entry["ext"] = json.loads(json.dumps(g.ext, default=str))
                if getattr(g, "uid", None):
                    # The identity a scene's hidden-object list names.
                    entry["uid"] = g.uid
                if getattr(g, "hidden", False):
                    entry["hidden"] = True
                if getattr(g, "locked", False):
                    entry["locked"] = True          # older readers ignore it
                if getattr(g, "exploded", None):
                    # An exploded view and each part's share of it, so the
                    # document reopens able to reassemble (core/explode.py).
                    # Older readers ignore both and see the parts where they
                    # stand.
                    entry["exploded"] = dict(g.exploded)
                if getattr(g, "explode_offset", None):
                    entry["explode_offset"] = list(g.explode_offset)
            if getattr(stream, "compatibility", False) and _links.link_of(g):
                link = entry["ext"][_links.LINK_KEY]
                link.pop("rel", None)
                link.update(_links._link_record(link["path"], stream.destination))
            kids = None if placeholder else getattr(g, "children", None)
            if kids:
                entry["children"] = []
                for c in kids:
                    entry["children"].append((yield from _entry(c)))
            yield
            return entry

        payload["groups"] = []
        for i, g in enumerate(groups or ()):
            payload["groups"].append((yield from _entry(g)))
            if progress and i % 64 == 0:
                progress(0.05 + 0.65 * (i + 1) / len(groups), "Preparing placements…")
        if kept:
            # Definitions kept with no copy placed: written like instances
            # through the SAME prototype table, so one still in use is not
            # written twice. Older readers ignore the key (and lose them).
            payload["kept_components"] = []
            for t in kept:
                payload["kept_components"].append((yield from _entry(t)))
        if protos:
            payload["protos"] = protos
    igz_archive_scene_fields(scene, payload, stream)
    survey_blobs = {}
    # The photogrammetric survey (Track G, G6) rides inside the document too,
    # for the same reason the textures do: a reference mesh that lives in a
    # folder next to the .igz is one move away from being lost.
    survey = getattr(scene, "photo_mesh", None)
    if survey is not None and getattr(survey, "triangle_count", 0):
        if mesh_steps is not None:
            from formats.igz_snapshot import survey_steps
            survey_blobs = yield from survey_steps(survey)
        else:
            from georef.photomesh import pack_mesh
            meta, survey_blobs = pack_mesh(survey)
            payload["photo_mesh"] = meta

    thumbnail = getattr(scene, "_home_thumbnail_png", None)
    if thumbnail:
        survey_blobs["thumbnail.png"] = thumbnail
    return payload, geom, large, survey_blobs


def _large_document(scene):
    classify = getattr(native_io(), "igz_large_document", None)
    if classify and getattr(scene, "_root", None) is not None:
        return classify(scene.mesh, scene.groups, scene.kept_components, _LARGE_DOCUMENT)
    return _live_element_count(scene) > _LARGE_DOCUMENT


def _live_element_count(scene) -> int:
    """Faces + edges the document will hold: the loose mesh, and every
    distinct mesh of the groups (a component prototype is written once,
    however many times it is placed). Only list lengths."""
    from core.lazy_mesh import entity_count
    mesh = scene.mesh if hasattr(scene, "mesh") else scene
    total = entity_count(mesh)
    seen: set = set()
    stack = (list(getattr(scene, "groups", None) or ())
             + list(getattr(scene, "kept_components", None) or ()))
    while stack:
        g = stack.pop()
        m = getattr(g, "mesh", None)
        if m is not None and id(m) not in seen:
            seen.add(id(m))
            total += entity_count(m)
        stack.extend(getattr(g, "children", None) or ())
    return total


class NonFiniteDocumentError(ValueError):
    """The document holds a coordinate that is not a number; it is not
    written, so the file on disk keeps its last good version."""

    def __init__(self, where: str) -> None:
        from core.i18n import tr
        self.where = where
        super().__init__(tr(
            "The model has a point with invalid coordinates (not a number), "
            "so it was not saved and the file on disk keeps its last good "
            "version. Undo the last step and save again. Where: {where}",
            where=where))


def _non_finite_where(node, path: str = "scene") -> str:
    """The first place in the payload holding a NaN or an infinity."""
    import math as _math
    stack = [(node, path)]
    while stack:
        cur, here = stack.pop()
        if isinstance(cur, float) and not _math.isfinite(cur):
            return here
        if isinstance(cur, dict):
            stack.extend((v, f"{here} › {k}") for k, v in
                         reversed(list(cur.items())))
        elif isinstance(cur, (list, tuple)):
            stack.extend((v, f"{here}[{i}]") for i, v in
                         reversed(list(enumerate(cur))))
    return path


@contextmanager
def _atomic_output(path):
    """Publish only a closed, complete file; simultaneous saves own separate temps."""
    output = getattr(native_io(), "AtomicDocument", None)
    if output:
        stage = output(path)
        try:
            with open(stage.fileno(), "wb", closefd=False) as target:
                yield target
            stage.publish()
        finally:
            stage.discard()
        return
    from uuid import uuid4

    tmp = path.with_name(f".{uuid4().hex}.part")
    target = tmp.open("xb")
    try:
        with target:
            yield target
        tmp.replace(path)
    finally:
        tmp.unlink(missing_ok=True)


def _write_atomic(path: Path, data: bytes | Fragment) -> None:
    with _atomic_output(path) as dest:
        writer = getattr(native_io(), "igz_write_plain", None)
        if writer:
            if isinstance(data, Fragment):
                data.file.flush()
                writer(dest.fileno(), None, (data.file.fileno(), data.offset, data.size))
            else:
                writer(dest.fileno(), data, None)
        elif isinstance(data, Fragment):
            for chunk in data.chunks():
                dest.write(chunk)
        else:
            dest.write(data)


def _write_container(path: Path, doc: str, blobs: dict,
                     fast: bool = False, deflated: dict | None = None, *, meter=None) -> None:
    """The ZIP shape: ``document.json`` deflated, images stored as-is (PNG/JPG
    are already compressed — deflating them again costs time for nothing).
    ``fast`` deflates at level 1 (see ``_LARGE_DOCUMENT``). ``deflated``
    holds members compressed like the document — the geometry blocks."""
    import zipfile
    with _atomic_output(path) as target:
        writer = getattr(native_io(), "igz_write_zip", None)
        if writer is not None:
            members = []
            def member(name, data, compressed):
                if isinstance(data, Fragment):
                    data.file.flush()
                    source = data.file.fileno(), data.offset, data.size
                    members.append((name, compressed, None, source))
                else:
                    if isinstance(data, str):
                        data = data.encode("utf-8")
                    members.append((name, compressed, data, None))
            member(_DOC_ENTRY, doc, True)
            for name in sorted(deflated or ()):
                member(name, deflated[name], True)
            for name in sorted(blobs):
                member(name, blobs[name], False)
            options = {"meter": meter} if meter is not None else {}
            if writer(target.fileno(), members, fast, **options):
                return
        with zipfile.ZipFile(target, "w") as zf:
            def write(member, data, compression):
                info = zipfile.ZipInfo(member, date_time=_ZIP_EPOCH)
                info.compress_type = compression
                level = 1 if fast else None
                if isinstance(data, Fragment):
                    info.file_size = data.size
                    info._compresslevel = level
                    with zf.open(info, "w") as dest:
                        for chunk in data.chunks():
                            dest.write(chunk)
                else:
                    zf.writestr(info, data, compresslevel=level)

            write(_DOC_ENTRY, doc, zipfile.ZIP_DEFLATED)
            for member in sorted(deflated or ()):
                write(member, deflated[member], zipfile.ZIP_DEFLATED)
            for member in sorted(blobs):
                write(member, blobs[member], zipfile.ZIP_STORED)


def _native_blocks(meta) -> dict:
    """What :func:`_json_arrays` makes of a mesh block, from what the engine
    read of it (ingetrazo_core.json_mesh_blocks): the same arrays, the same
    attribute table in the same order."""
    import numpy as np
    from core.topology import _maximal_holes
    pos, flags, curves, layers, raw_names, sizes, counts, face_attr, \
        raw_attrs, multi = meta
    ne = len(flags)
    n_edge_pts = 2 * ne
    nf = len(counts)
    small = not _bulk_sized(ne, nf)
    # The names kept, numbered in the order they first come. Two spellings
    # of one name are one layer.
    names: dict = {}
    lut = []
    for text in raw_names:
        lay = json.loads(text)
        lut.append(-1 if small and not lay else names.setdefault(lay, len(names)))
    if names:
        layers = np.asarray(lut + [-1], dtype=np.int32)[layers]
    table = [json.loads(text, parse_constant=_note_constant)
             for text in raw_attrs]
    if len(multi):
        # Nested holes dropped, in Python like every other reader: only
        # these few faces pay for it.
        first_ring = np.cumsum(counts) - counts
        starts = n_edge_pts + np.cumsum(sizes) - sizes
        pieces = []
        keep_rings = np.ones(len(sizes), dtype=bool)
        new_counts = counts.copy()
        new_sizes = sizes.copy()
        at = 0
        for f in multi.tolist():
            r0 = int(first_ring[f])
            nr = int(counts[f])
            ends = starts[r0 + 1:r0 + nr] + sizes[r0 + 1:r0 + nr]
            lo, hi = int(starts[r0 + 1]), int(ends[-1])
            loops = [[QVector3D(*v) for v in pos[int(a):int(b)].tolist()]
                     for a, b in zip(starts[r0 + 1:r0 + nr].tolist(),
                                     ends.tolist())]
            kept = _maximal_holes(loops)
            pieces.append(pos[at:lo])
            flat = [(p.x(), p.y(), p.z()) for loop in kept for p in loop]
            if flat:
                pieces.append(np.asarray(flat, dtype=np.float64))
            at = hi
            keep_rings[r0 + 1 + len(kept):r0 + nr] = False
            new_sizes[r0 + 1:r0 + 1 + len(kept)] = [len(k) for k in kept]
            new_counts[f] = 1 + len(kept)
        pieces.append(pos[at:])
        pos = np.concatenate(pieces)
        sizes = new_sizes[keep_rings]
        counts = new_counts
    meta_out: dict = {"face_attrs": table}
    if names:
        meta_out["edge_layers"] = list(names)
    # In the dtypes of a format-3 block, for the arrays this stays alive as:
    # a saved point is a float32 written out in full, so nothing is lost
    # (hundreds of MB on the plant of #158). A value that is not one keeps
    # its double.
    pos32 = pos.astype(np.float32)
    if pos32.astype(np.float64).tobytes() == pos.tobytes():
        pos = pos32
    index = np.int32 if len(pos) < 2 ** 31 else np.int64
    return dict(
        meta=meta_out,
        pos=pos,
        edges=np.arange(n_edge_pts, dtype=index).reshape(-1, 2),
        flags=flags,
        curves=curves if (curves >= 0).any() else None,
        layers=layers if names else None,
        rings=np.arange(n_edge_pts, len(pos), dtype=index),
        sizes=sizes.astype(np.int32),
        counts=counts.astype(np.int32),
        face_attr=face_attr,
        sequential=small)


def _merge_native_attrs(src) -> None:
    """The last of :func:`_json_arrays` for an engine-read block: equal
    attributes are one table entry, and those that say nothing none. Run
    when the mesh is read, as _json_arrays is: textures unpacked by then."""
    import numpy as np
    table: list = []
    table_index: dict = {}
    lut = []
    for attrs in src["meta"]["face_attrs"]:
        if _face_attrs_from_json(attrs):
            key = json.dumps(attrs, sort_keys=True, default=str)
            k = table_index.get(key)
            if k is None:
                k = table_index[key] = len(table)
                table.append(attrs)
            lut.append(k)
        else:
            lut.append(-1)
    src["face_attr"] = np.asarray(lut + [-1], dtype=np.int32)[src["face_attr"]]
    src["meta"]["face_attrs"] = table


def _adopt_native_blocks(data, blocks) -> None:
    """Hang each block's arrays where the document left its number. The very
    places the engine looked (jsondoc.rs), no deeper."""
    from ingetrazo_core import FileGroups

    def adopt(raw):
        n = raw.pop("_njb", None)
        if n is not None:
            raw["_arrays"] = _native_blocks(blocks[n])
    scene = data.get("scene")
    if not isinstance(scene, dict):
        return
    adopt(scene)
    for raw in scene.get("protos") or []:
        if isinstance(raw, dict):
            adopt(raw)
    for raw in FileGroups.records(scene):
        adopt(raw)


def _read_member_flat(archive, name):
    """``archive.read(name)``. A long member goes into a buffer of its size,
    a piece at a time: read() grows its result as it inflates, and for an
    instant holds the whole of it twice (6.6 GB for the document of #158)."""
    info = archive.getinfo(name)
    if info.file_size < _NATIVE_JSON_MIN:
        return archive.read(name)
    buf = bytearray(info.file_size)
    view = memoryview(buf)
    at = 0
    with archive.open(info) as f:
        while at < len(buf):
            got = f.readinto(view[at:at + (1 << 24)])
            if not got:
                break
            at += got
    view.release()
    if at < len(buf):
        del buf[at:]
    return buf


def _unpack_photo_mesh(payload: dict, archive):
    """Rebuild the stored survey, or ``None`` when the document has none."""
    meta = payload.get("photo_mesh")
    if not meta:
        return None
    from georef.photomesh import unpack_mesh

    def read_blob(member: str) -> bytes:
        try:
            return archive.read(member)
        except KeyError:
            return b""

    return unpack_mesh(meta, read_blob)


_EMPTY = []


def _empty_mesh():
    """The one empty mesh every linked placeholder is written with."""
    if not _EMPTY:
        from core.mesh import Mesh
        _EMPTY.append(Mesh())
    return _EMPTY[0]


def load_into(scene, path: Path, progress=None, *, include_links=True, link_meshes=None) -> None:
    """Replace ``scene`` contents with what's stored at ``path``.

    ``progress(fraction, text)``, when given, is called at the loader's
    milestones so the window can show a bar: a big document took seconds
    with nothing on screen and read as frozen (issue #59, @pacaeiro)."""
    import gc

    # Mass object construction ahead — see formats.skp.apply_payload: the
    # generational GC re-scanning the growing heap can dominate big loads.
    # The linked sources are read with it still off: once the host's
    # millions of objects exist, a generation-1/2 pass is a walk of all of
    # them, and two such passes fell inside the ride's link resolution
    # (0.41 s in _rename_tags, 0.48 s in _populate, of 4.1 s; 2026-11-13).
    # Nothing is lost by waiting: the window collects once the document
    # shows (MainWindow.settle_heap).
    from core import _native
    staging = getattr(getattr(_native.native, "DocumentOpen", None), "staging", None)
    _gc_was_enabled = gc.isenabled() and not (staging and staging())
    if _gc_was_enabled:
        gc.disable()
    flag = _finite_flag.set([False])
    repairs = _repair_count.set([0])
    try:
        reading = (lambda f, phase: progress(0.8 * f, phase)) if progress else None
        linking = (lambda f, phase: progress(0.8 + 0.2 * f, phase)) if progress else None
        _load_into_inner(scene, path, reading, link_meshes)
        _load_links(scene, path, include_links, linking)
        if getattr(scene, "_root", None) is not None:
            scene._root._index_assemblies()
        if progress:
            progress(1.0, "Finishing…")
    finally:
        _finite_flag.reset(flag)
        _repair_count.reset(repairs)
        if _gc_was_enabled:
            gc.enable()


def _load_links(scene, path: Path, include_links: bool, progress=None) -> None:
    """The second half of :func:`load_into`: the project file and the
    linked sources of the document just read."""
    # Linked files: the copies are filled in from their sources. A source
    # that cannot be read leaves its copies empty — the document still opens
    # (scene.link_errors says which, for the window to report).
    from core import links
    scene._project_host_path = str(Path(path).resolve())
    from core.project import read as read_project
    try:
        shared = read_project(scene)
        if shared:
            scene.plugin_data["ifc_identity"] = shared["ifc_identity"]
    except (OSError, ValueError) as exc:
        scene.project_warning = str(exc)
    scene.link_errors = {}
    if not include_links:
        # Packed links can already have children on disk. Drop the placement
        # itself (at any depth) before resolving paths or evaluating subsets.
        for group in links.linked_groups(scene):
            group._detach()
        scene.groups = [group for group in scene.groups if not links.is_linked(group)]
    elif links.linked_groups(scene):
        # A cancelled open must propagate through nested link readers, which
        # normally turn source errors into recoverable missing-link entries.
        class Interrupted(BaseException):
            pass

        def report(fraction, phase):
            try:
                progress(fraction, phase)
            except Exception as exc:
                raise Interrupted from exc

        try:
            if progress:
                links.resolve(scene, Path(path), progress=report)
            else:
                links.resolve(scene, Path(path))
        except Interrupted as stop:
            raise stop.__cause__
        except Exception as exc:  # noqa: BLE001 — never fail the open
            scene.link_errors = {"": str(exc)}


#: Entities left out of the document being opened because a coordinate
#: is not a number (NaN / inf) — counted per load, reported on the scene.
_dropped_nonfinite = [0]
#: Whether the document text holds any NaN / Infinity at all. The JSON
#: parser reports them as it reads (``parse_constant``), so a clean
#: document — nearly every one — skips the per-coordinate check entirely.
_saw_nonfinite = [False]
_finite_flag = ContextVar("file_nonfinite", default=_saw_nonfinite)
_repair_count = ContextVar("file_repairs", default=_dropped_nonfinite)


def _note_constant(token: str) -> float:
    _finite_flag.get()[0] = True
    return float(token)


def _finite_points(*points) -> bool:
    try:
        return all(math.isfinite(float(c)) for p in points for c in p)
    except (TypeError, ValueError):
        return False


def _mesh_blocks(payload: dict):
    """Every local geometry block in the file, including nested placements."""
    from ingetrazo_core import FileGroups
    yield payload
    yield from payload.get("protos", [])
    for raw in FileGroups.records(payload):
        if raw.get("xform") is None or "proto" not in raw:
            yield raw


class _GeomMembers(dict):
    """A document's ``geom/`` members (name → bytes), and in ``checked``
    the block (its JSON ``meta`` dict, by identity) each one was already
    validated for by the Rust engine — :func:`_geometry_arrays` then
    builds its arrays without scanning them again."""
    __slots__ = ("checked", "built_format")

    def __init__(self):
        super().__init__()
        self.checked = {}


#: ZipInfo flag bits the native reader handles like ``zipfile``: the
#: deflate options, a trailing data descriptor and UTF-8 names. Anything
#: else (encryption, patches) is read by ``zipfile``, which says no.
_NATIVE_ZIP_FLAGS = 0x2 | 0x4 | 0x8 | 0x800


def _read_geometry_members(archive, path: Path, payload: dict, skip=(), *, built_format=False) -> dict:
    """Format 3's binary geometry blocks, read while the archive is open.

    With the Rust engine the members are inflated in parallel off the GIL
    and each block is checked against the layout its JSON declares in the
    same call (``ingetrazo_core.igz_read_geometry``). On the ride model
    that was 11,604 members inflated one after the other, then a dozen
    NumPy reductions per block. Whatever the engine cannot vouch for —
    an unusual member, a block it does not accept, an engine that fails —
    is read and checked here exactly as before, so a damaged document
    fails with the same message either way."""
    names = [n for n in archive.namelist() if n.startswith(_GEOM_PREFIX) and n not in skip]
    out = _GeomMembers()
    out.built_format = built_format
    native = None
    if names:
        from core import _native
        mod = _native.native
        native = getattr(mod, "igz_read_geometry", None) if mod else None
    if native is not None:
        try:
            _read_geometry_native(native, archive, path, payload, names, out)
        except Exception as exc:  # noqa: BLE001 — Python reads them below
            import logging
            logging.getLogger(__name__).warning(
                "Native geometry reader failed, using Python: %s", exc)
            out.clear()
            out.checked.clear()
    # Whatever the engine did not read, inflated side by side.
    out.update(_read_members(archive, [n for n in names if n not in out]))
    return out


def _read_geometry_native(native, archive, path, payload, names, out):
    metas: dict = {}                   # member → its one block's meta
    shared: set = set()
    for block in _mesh_blocks(payload):
        meta = block.get("geom") if isinstance(block, dict) else None
        member = meta.get("member") if isinstance(meta, dict) else None
        if not isinstance(member, str):
            continue
        if member in metas and metas[member] is not meta:
            shared.add(member)         # two blocks, one member: Python checks
        metas[member] = meta
    todo, entries, checks = [], [], []
    for n in names:
        info = archive.getinfo(n)
        if info.flag_bits & ~_NATIVE_ZIP_FLAGS or \
                info.compress_type not in (0, 8):
            continue
        try:
            local = info.orig_filename.encode(
                "utf-8" if info.flag_bits & 0x800 else "cp437")
        except UnicodeEncodeError:
            continue
        check = None
        meta = metas.get(n) if n not in shared else None
        if meta is not None:
            try:
                check = (meta["arrays"], len(meta.get("face_attrs") or ()),
                         len(meta.get("edge_layers") or ()))
            except (KeyError, TypeError):
                check = None
        todo.append((n, meta if check is not None else None))
        entries.append((info.header_offset, info.compress_type,
                        info.compress_size, info.file_size, info.CRC, local,
                        info.flag_bits))
        checks.append(check)
    if not todo:
        return
    options = {"built_format": out.built_format}
    if isinstance(archive, DocumentArchive):
        offsets = []
        for name, _ in todo:
            with archive.open(name) as member:
                offset = getattr(member, "_orig_compress_start", None)
                if offset is None:
                    break
                offsets.append(offset)
        if len(offsets) == len(todo):
            options.update(fd=archive.fp.fileno(), member_offsets=offsets)
    results = native(str(path), entries, checks, **options)
    for (n, meta), (blob, ok) in zip(todo, results):
        if blob is None:
            continue                   # read by zipfile, which decides
        out[n] = blob
        if ok and meta is not None:
            out.checked[n] = meta


def _load_into_inner(scene, path: Path, progress=None, link_meshes=None) -> None:
    def tick(frac, text):
        if progress is not None:
            progress(frac, text)

    _repair_count.get()[0] = 0
    _finite_flag.get()[0] = False

    from core import _native
    tick(0.05, "Reading the document…")
    data, archive = _read_document(
        path, pump=(lambda: tick(0.1, "Reading the document…"))
        if progress is not None else None,
        read_progress=(lambda f: tick(0.05 + 0.14 * f, "Reading the document…"))
        if progress is not None else None, placements=bool(getattr(scene, "_root", None)) and _native.wanted())
    payload = data.get("scene", {})

    def read_materials():
        from core.materials import Material
        materials = {}
        decode = getattr(native_io(), "igz_load_material", None)
        for raw in payload.get("materials", []):
            mat = decode(raw, Material) if decode else Material.from_dict(raw)
            if mat.name:
                materials[mat.name] = mat
        if link_meshes is not None:
            link_meshes.prepare(materials, payload)
        return materials

    survey = None
    geom: dict = {}
    is_built = "built_format" in data
    try:
        if is_built or built.is_built(path):
            if (type(data.get("built_format")) is not int or data["built_format"] != 1
                    or archive is None or "igz_format" in data):
                raise ValueError("Invalid or unsupported Built document format")
        if archive is not None:
            if 'thumbnail.png' in archive.namelist():
                info = archive.getinfo('thumbnail.png')
                if info.file_size <= 1024 * 1024:
                    scene._home_thumbnail_png = archive.read(info)
            # Format 3's binary geometry blocks, read while the archive is
            # open (it closes below).
            tick(0.2, "Unpacking textures…")
            _unpack_textures(payload, archive)
            materials = read_materials()
            skip = link_meshes.unchanged_members(archive, payload) if link_meshes else ()
            tick(0.25, "Reading geometry…")
            geom = _in_worker(
                lambda: _read_geometry_members(archive, path, payload, skip, built_format=is_built),
                (lambda: tick(0.25, "Reading geometry…")) if progress is not None else None,
                background=isinstance(archive, DocumentArchive))
            # Rebuilt here, while the archive is still open — it closes below
            # and the survey's blobs live inside it.
            survey = _unpack_photo_mesh(payload, archive)
        else:
            materials = read_materials()
    finally:
        if archive is not None:
            archive.close()
    fmt = data.get("igz_format", 1)
    if fmt > CURRENT_FORMAT:
        raise ValueError(
            f"Document format v{fmt} is newer than this build (v{CURRENT_FORMAT})."
        )

    payload = data.get("scene", {})
    _normalize_text_curve_ids(payload, geom)

    tick(0.35, "Building geometry…")
    igz_reset_scene(scene, payload, survey)
    scene.materials = materials

    def read_mesh(mesh, block, identity):
        if link_meshes is not None:
            return link_meshes.read(mesh, block, geom, identity)
        _load_mesh(mesh, block, geom)
        return mesh

    scene.mesh = read_mesh(scene.mesh, payload, ("loose",))
    igz_restore_views(scene, payload)
    proto_meshes: list = []
    import time as _time
    raw_protos = payload.get("protos", [])
    last = _time.monotonic()
    for k, raw in enumerate(raw_protos):
        m = Mesh()
        proto_meshes.append(read_mesh(m, raw, ("proto", k)))
        if _time.monotonic() - last > 0.25:
            # Thousands of components took 25 s here without a word to the
            # window, which the desktop reads as a hang (#158).
            tick(0.35 + 0.25 * k / max(1, len(raw_protos)),
                 "Building geometry…")
            last = _time.monotonic()
    from core.group import document_name_scope
    from ingetrazo_core import FileGroups
    stored_names = [raw["name"] for raw in FileGroups.records(payload) if raw.get("name")]
    raw_groups = payload.get("groups") or []
    kept = payload.get("kept_components") or []
    with document_name_scope(stored_names):
        reader = FileGroups(len(raw_groups) + len(kept))
        if raw_groups or kept:
            tick(0.6, "Placing groups…")
        groups, library = _in_worker(
            lambda: reader.build(raw_groups, kept, proto_meshes, read_mesh, is_built),
            (lambda: tick(0.6 + 0.35 * reader.fraction(), "Placing groups…"))
            if progress else None, background=True, cancel=getattr(reader, "cancel", None))
        scene.groups = groups
        scene.kept_components = library
    tick(0.96, "Finishing…")
    if stored_names:
        from core.group import reserve_group_names
        reserve_group_names(stored_names)   # new groups never reuse a stored "Group N"

    _repair_count.get()[0] += igz_finish_scene(scene, payload)
    scene.load_repairs = _repair_count.get()[0]

    if progress is not None:
        _release_in_steps(data, lambda: tick(0.98, "Finishing…"))
    data = payload = None
    scene.version += 1


def _bulky(node) -> bool:
    """A container worth taking apart before dropping: a long list, or a
    dict holding one (a group, a prototype, the scene). Coordinates and
    small face records just go with their parent."""
    if isinstance(node, list):
        return len(node) > 64
    if isinstance(node, dict):
        return any(isinstance(v, (list, dict)) and len(v) > 64
                   for v in node.values())
    return False


def _release_in_steps(data, pump) -> None:
    """Drop the parsed document a slice at a time, ``pump()`` between
    slices. Released in one go when the loader returned, the JSON tree of
    a 3.3 GB document — tens of millions of objects — kept the window from
    answering for 8 s, long enough for the desktop to call IngeTrazo «not
    responding» (#158)."""
    import time
    last = time.monotonic()
    stack = [data]
    while stack:
        node = stack.pop()
        if isinstance(node, dict):
            for key in list(node):
                child = node.pop(key)
                if _bulky(child):
                    stack.append(child)
        elif isinstance(node, list):
            while node:
                child = node.pop()
                if _bulky(child):
                    stack.append(child)
                if time.monotonic() - last > 0.25:
                    pump()
                    last = time.monotonic()
        if time.monotonic() - last > 0.25:
            pump()
            last = time.monotonic()
