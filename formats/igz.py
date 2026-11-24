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
import math
from pathlib import Path

from core.version import __version__
from formats import built
from formats.igz_stream import Fragment, SaveStream
from formats.native_io import DocumentArchive, native_io

from ingetrazo_core import (
    igz_load_into as load_into,
    igz_load_links as _load_links,
    igz_unpack_photo_mesh as _unpack_photo_mesh,
    igz_bulky as _bulky,
    igz_release_in_steps as _release_in_steps,
    igz_prepare_scene_steps as prepare_scene_steps,
    igz_native_blocks as _native_blocks,
    igz_merge_native_attrs as _merge_native_attrs,
    igz_adopt_native_blocks as _adopt_native_blocks,
    igz_read_member_flat as _read_member_flat,
    igz_read_geometry_members as _read_geometry_members,
    igz_read_geometry_native as _read_geometry_native,
    igz_load_into_inner as _load_into_inner,
    igz_store_columns as _store_columns,
    igz_write_geometry as _write_geometry,
    igz_pack_plan as _pack_plan,
    igz_geometry_arrays as _geometry_arrays,
    igz_document_geometry_arrays as _document_geometry_arrays,
    igz_load_geometry as _load_geometry,
    igz_build_geometry as _build_geometry,
    igz_load_mesh as _load_mesh,
    igz_load_mesh_json as _load_mesh_json,
    igz_without_nonfinite as _without_nonfinite,
    igz_normalize_text_curve_ids as _normalize_text_curve_ids,
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


_EMPTY = []


def _empty_mesh():
    """The one empty mesh every linked placeholder is written with."""
    if not _EMPTY:
        from core.mesh import Mesh
        _EMPTY.append(Mesh())
    return _EMPTY[0]


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
