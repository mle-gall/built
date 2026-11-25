# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Built and IGZ document codecs, schema constants and script-facing exceptions."""
from __future__ import annotations
from contextvars import ContextVar
from functools import partialmethod

import json
import math
from pathlib import Path

from core.version import __version__
from formats import built
from formats.igz_stream import Fragment, SaveStream
from formats.native_io import DocumentArchive, native_io

from ingetrazo_core import (
    igz_members_init, igz_empty_mesh as _empty_mesh, igz_note_constant as _note_constant,
    igz_mesh_blocks as _mesh_blocks,
    DocumentOutput as _atomic_output,
    igz_backup_previous as _backup_previous,
    igz_classify_document as _large_document,
    igz_live_element_count as _live_element_count,
    igz_non_finite_where as _non_finite_where,
    igz_bulk_sized as _bulk_sized,
    igz_export as export_igz,
    igz_save_scene as save_scene,
    igz_write_container as _write_container,
    igz_write_atomic as _write_atomic,
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
    igz_finite_points as _finite_points, igz_without_nonfinite as _without_nonfinite,
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


_EMPTY = []


#: Entities left out of the document being opened because a coordinate
#: is not a number (NaN / inf) — counted per load, reported on the scene.
_dropped_nonfinite = [0]
#: Whether the document text holds any NaN / Infinity at all. The JSON
#: parser reports them as it reads (``parse_constant``), so a clean
#: document — nearly every one — skips the per-coordinate check entirely.
_saw_nonfinite = [False]
_finite_flag = ContextVar("file_nonfinite", default=_saw_nonfinite)
_repair_count = ContextVar("file_repairs", default=_dropped_nonfinite)


class _GeomMembers(dict):
    """A document's ``geom/`` members (name → bytes), and in ``checked``
    the block (its JSON ``meta`` dict, by identity) each one was already
    validated for by the Rust engine — :func:`_geometry_arrays` then
    builds its arrays without scanning them again."""
    __slots__ = ("checked", "built_format")

    __init__ = partialmethod(igz_members_init)


#: ZipInfo flag bits the native reader handles like ``zipfile``: the
#: deflate options, a trailing data descriptor and UTF-8 names. Anything
#: else (encryption, patches) is read by ``zipfile``, which says no.
_NATIVE_ZIP_FLAGS = 0x2 | 0x4 | 0x8 | 0x800
