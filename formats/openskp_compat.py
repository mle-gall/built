# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Import bridges for OpenSKP 1.5: layers, folders, kinds and entity IDs.

With OpenSKP 1.5, dimension paths and anchored annotation reading are
upstream (#384, #473, #484): the local replacements below for them detect
that and step aside. The definition-list end fix still applies, but must
preserve 1.5's inline-entity bookkeeping. The 2019 layer-tail check must not
mistake an unrelated version check for support of the extra two bytes.

**Legacy dimensions anchored inside groups** (iamahsanmehmood/openskp#384).
A pre-2021 ``CDimensionLinear`` follows each connection ref with an entity
ref and two lists of refs — the instance paths of the anchored entity. The
pinned reader skips a FIXED 42/82 bytes there, right only while the lists
are empty (a dimension on loose geometry). Anchored to a vertex inside
nested groups, the read slides off the record and the root entity reader,
which stops at the first unreadable item, drops everything after it:
Juan José Noriega's house (a .skp of the 2018 version) opened with 2 of
its 72 root entities — walls and lawn, no upper floor, roofs, openings or
trees (12 placements instead of 2384).

**A face's own tag arrives as a number** (#700). ``Instance.layer`` is
the tag's name, but ``Face.layer`` is the tag's id in the file, and the
id → name table stays inside the parser (``SkpModel`` does not keep it).
A roof tagged "Roof" landed on a layer called 11, so hiding "Roof" left it
on screen. The table is kept from the parse and :func:`resolve_face_layers`
turns the ids into names.

**Annotations carrying their anchor in full** (a .skp of the 2017
version). MFC writes an object in full the first time anything points
at it, so a dimension or leader text anchored to a vertex the stream has not
reached yet carries that ``CVertex`` inline. The dimension reader refused a
new object as a connection point; the text reader skipped it by scanning for
the string delimiter, so its store index went unallocated, every later
reference landed one slot off (edges on edges, a class ref on a dimension)
and the file did not open — silently, as :func:`formats.skp.parse_skp`
reports any parser error as "unreadable". The text delimiter's last byte is
also 0 on some texts, not only 1.

**A definition list that ends before its count.** Edges first written in a
face come back as entity-list back-refs; right after a text, the text
reader takes them as leader attachments, and the list then reads its
definition's tail as items. The tail's signature now ends a definition list
whatever was counted (upstream checked it only after a "burn").

**Groups arrive as components.** The classic reader reads a ``CGroup``
and a ``CComponentInstance`` alike and keeps no trace of which it was, and
the 2021+ reader drops the definition's kind. The definitions placed
through a ``CGroup``, or marked as groups by their kind, are noted during
the parse (:func:`group_definitions`) so the import makes them groups
again.

**A 2019 layer record two bytes longer.** From the 2019 version on, a
layer record ends 23 bytes after its last string, not 21 (the new field is
most likely the layer's dash pattern). The reader skips 21, so in the first
definition the next count read two bytes early, came out at 33 million
and the file did not open («implausible def entity count», #157). The
2020 files still opened only because the reader already steps over zero
words between layers and around their counts there; only 2019 files take
the two extra bytes here.

**Annotations in 2021+ (VFF) files** (#1226). The VFF reader's annotation
handling lags the other ports in three ways. (1) A dimension drawn inside
a group/component is returned among the MODEL-level dimensions with its
endpoints still in the definition's local space (its connection names no
instance to lift by), so it lands displaced from the geometry the import
did transform. (2) The dimension's constraint axis (``5BD1``) is never
read, and the frame fields are taken the wrong way round — ``5BCF`` is
the plane's NORMAL and ``5BD0`` its X axis, as upstream's C++ port
settled in openskp#481: the offset vector is ``(N × X) · offset`` for
axis 1 and ``−X · offset`` for axis 2. Without the axis the importer
guessed ``cross(normal, segment)``, which for most of a staircase's
dimensions points out of the dimension plane entirely. (3) Text records
(``55F0``) are not parsed at all, so leader texts vanish.
:func:`resolve_vff_annotations` re-reads the two record kinds from the
parse's own inputs (kept by wrapping ``_parse_dimensions``, the one
place they meet), gives each dimension its axis, moves the
definition-owned dimensions onto their definition (marked ``vff_owned``
so the adapter places them per instance), and rebuilds the texts on
their owning definition or the model root (marked ``vff_recovered``).
A text whose connection type the reader cannot resolve (e.g. type 4)
keeps its label position but no anchor.

**PBR materials (2025+).** A 2025+ ``material.xml`` carries a
``<mat:pbrMR>`` block (metallic/roughness factors and maps, normal map with
its scale and OpenGL/DirectX style, ambient occlusion) whose images sit in
the material's ZIP folder; OpenSKP 1.5 drops it, so imported PBR materials
lost everything but their base colour/texture. :func:`read_pbr` reads it
during the parse and :func:`pbr_for` hands it to the importer
(docs/skp-pbr-format.md). Steps aside once ``openskp.model.Material`` has
a ``pbr`` field of its own.

All patches are detected by BEHAVIOUR, not by version: once the upstream
reader carries ``_connection_paths`` the first is left alone, the second
only touches a face layer that is still a number, the anchor and list
fixes step aside once the text reader no longer scans for the delimiter
and the list reader no longer ties the tail check to burns, and the layer
fix once the layer reader looks at the version itself.
"""
from __future__ import annotations

import threading

_applied = False
_parse_tables = threading.local()   # the last parse's layer id → name


def apply() -> bool:
    """Install the fixes into ``openskp`` when needed. Returns True when a
    patch was installed (idempotent)."""
    global _applied
    if _applied:
        return False
    try:
        from openskp import legacy as L
    except Exception:  # noqa: BLE001 — no openskp: nothing to fix
        return False
    from openskp import _core
    from functools import partial
    from ingetrazo_core import SkpArchiveCursor, skp_mfc_object, skp_mfc_readers, skp_geometry_nodes
    if L._R is not SkpArchiveCursor:
        skp_mfc_readers(L)
    L._R = SkpArchiveCursor
    L._Archive.read_object = lambda ar, r, expect=None: skp_mfc_object(ar, r, expect)
    if hasattr(_core, "_GeometryRecordNode"):
        _core._parse_tlv_geometry = partial(
            skp_geometry_nodes, _core._TlvNode, _core._GeometryRecordNode,
            _core._tag_strings())
    _core._parse_pages = _parse_pages
    _applied = True
    layers = _keep_layer_table()
    folders = _keep_tag_folders()
    ann = _keep_vff_annotation_inputs()
    groups = _note_legacy_groups(L)
    kinds = _note_definition_kinds()
    pbr = _keep_pbr()
    return layers or groups or kinds or ann or folders or pbr


def _note_legacy_groups(L) -> bool:
    """Remember which definitions a classic (pre-2021) file places through a
    ``CGroup`` — a GROUP — rather than a ``CComponentInstance``.
    The reader reads both the same way and the model it builds keeps no
    trace of which it was, so a group arrived as a component. The ids land
    in a per-thread set, reset before each parse (see
    :func:`reset_group_definitions`)."""
    readers = getattr(L, "_READERS", None)
    original = (readers or {}).get("CGroup")
    if original is None or getattr(original, "_notes_groups", False):
        return False

    def read_group(ar, r):
        rec = original(ar, r)
        if isinstance(rec, dict) and rec.get("def") is not None:
            if not hasattr(_parse_tables, "groups"):
                _parse_tables.groups = set()
            _parse_tables.groups.add(rec["def"])
        return rec

    read_group._notes_groups = True
    readers["CGroup"] = read_group
    return True


def _note_definition_kinds() -> bool:
    """Remember which definitions a 2021+ file marks as GROUPS (#1029). The
    definition record carries its kind in ``8315``: 0 a component, 1 a
    group, 2 the quad behind an Image — the reader only keeps "image". It
    hands each definition's records to ``_extract_geometry_from_nodes``
    once, so that is where the kind is read; the root and the nested calls
    carry no ``8315`` and pass straight through."""
    try:
        from openskp import _core
    except Exception:  # noqa: BLE001
        return False
    original = getattr(_core, "_extract_geometry_from_nodes", None)
    if original is None or getattr(original, "_notes_kinds", False) \
            or not hasattr(_core, "extract_entity_id"):
        return False

    def _extract_geometry_from_nodes(elements, builder):
        for el in elements:
            if (el.get("tag") if isinstance(el, dict) else el.tag) == "8315":
                pay = el["payload"] if isinstance(el, dict) else el.payload
                kind = _core.parse_var_int(pay, 0, len(pay))
                from types import SimpleNamespace
                did = (_core.extract_entity_id({"children": elements})
                       if isinstance(el, dict) else
                       _core._entity_id(SimpleNamespace(children=elements)))
                if did is not None:
                    _parse_tables.kinds = True
                    if kind == 1:
                        if not hasattr(_parse_tables, "groups"):
                            _parse_tables.groups = set()
                        _parse_tables.groups.add(did)
                break
        return original(elements, builder)

    _extract_geometry_from_nodes._notes_kinds = True
    _core._extract_geometry_from_nodes = _extract_geometry_from_nodes
    return True


def group_definitions() -> set:
    """The definition ids the last parse on this thread placed as GROUPS:
    through a ``CGroup`` in a classic file, by the definition's own kind in
    a 2021+ one."""
    return set(getattr(_parse_tables, "groups", None) or ())


def kinds_known(legacy: bool) -> bool:
    """Whether the last parse told groups from components: for a classic
    file when its group records are being noted (its instance records say
    which), for a 2021+ one when its definitions carried their kind."""
    if legacy:
        try:
            from openskp import legacy as L
        except Exception:  # noqa: BLE001
            return False
        reader = (getattr(L, "_READERS", None) or {}).get("CGroup")
        return bool(getattr(reader, "_notes_groups", False))
    return bool(getattr(_parse_tables, "kinds", False))


def reset_group_definitions() -> None:
    """Forget the groups of the previous parse — call before parsing."""
    _parse_tables.groups = set()
    _parse_tables.kinds = False
    _parse_tables.vff = None


def _keep_layer_table() -> bool:
    """Wrap ``openskp._core.full_parse`` so the layer id → name table it
    builds outlives the parse (both eras go through it)."""
    try:
        from openskp import _core
    except Exception:  # noqa: BLE001
        return False
    original = getattr(_core, "full_parse", None)
    if original is None or getattr(original, "_keeps_layers", False):
        return False

    def full_parse(*args, **kwargs):
        parsed = original(*args, **kwargs)
        table = (parsed.get("layer_id_to_name")
                 if isinstance(parsed, dict) else None)
        _parse_tables.layers = dict(table or {})
        # Instance dataclasses discard the retained entity-info subtree.
        from types import SimpleNamespace
        _parse_tables.pids = {}
        for key, definition in parsed.get("defs_dict", {}).items():
            instances = definition["builder"].instances
            _parse_tables.pids[key] = [
                i.get("persistent_id") or
                _core._entity_id(SimpleNamespace(children=i.get("children", [])))
                for i in instances]

        return parsed

    full_parse._keeps_layers = True
    _core.full_parse = full_parse
    return True


# ---- tag folders (.skp 2021+) ----------------------------------------
# openskp reads the tag list (993A) but not its sibling, the folder manager
# (9B3A), so folders never reached IngeTrazo. Decoded from Maxence's
# production files (2026-11-07), whose Tags panel nests them three deep:
#
#   9B3A  folder manager — one root folder, unnamed
#     803E  folder: DC05 its id (the tags' id format), 813E its name
#           (UTF-8), 833E its subfolders (803E records), 843E the ids of
#           the tags in it (each: one length byte, then the var-int),
#           823E hidden (one byte: 1 = hidden), 853E a flag left unread.
#
# A tag's own record (8C3C) does not name its folder (its 903C reads 0
# throughout): membership is the folder's list.

from ingetrazo_core import skp_tag_folders as parse_tag_folders


def _parse_pages(node):
    from ingetrazo_core import skp_pages
    return skp_pages(node["payload"]) if node is not None else []


def _find_node(node, tag):
    if node.tag == tag:
        return node
    for kid in getattr(node, "children", None) or ():
        hit = _find_node(kid, tag)
        if hit is not None:
            return hit
    return None


def _keep_tag_folders() -> bool:
    """Watch the parse go by for the folder manager: openskp hands it over
    as an opaque record inside the model's ``F801`` section (with the tag
    list), so the folders are decoded here and kept for the importer
    (:func:`tag_folder_records`)."""
    try:
        from openskp import _core
    except Exception:  # noqa: BLE001
        return False
    iterate = getattr(_core, "iter_top_level_lazy", None)
    parse = getattr(_core, "full_parse", None)
    if iterate is None or parse is None \
            or getattr(iterate, "_keeps_folders", False):
        return False

    def iter_top_level_lazy(*args, **kwargs):
        for item in iterate(*args, **kwargs):
            el = item[-1]
            if getattr(el, "tag", None) == "F801":
                node = _find_node(el, "9B3A")
                payload = getattr(node, "payload", None) if node else None
                if payload:
                    try:
                        _parse_tables.folders = parse_tag_folders(
                            bytes(payload))
                    except Exception:  # noqa: BLE001 — never fail the open
                        _parse_tables.folders = []
            yield item

    def full_parse(*args, **kwargs):
        _parse_tables.folders = []          # this parse's, not the last one's
        return parse(*args, **kwargs)

    iter_top_level_lazy._keeps_folders = True
    full_parse._keeps_layers = getattr(parse, "_keeps_layers", False)
    _core.iter_top_level_lazy = iter_top_level_lazy
    _core.full_parse = full_parse
    return True


def tag_folder_records() -> list:
    """The last parse's tag folders for the importer: ``[{"key", "name",
    "parent", "hidden", "tags"}]`` with ``key``/``parent`` as strings and
    ``tags`` as tag NAMES (through the parse's id → name table); an id the
    table does not know is dropped. Empty for a file without folders."""
    table = getattr(_parse_tables, "layers", None) or {}
    out = []
    for f in getattr(_parse_tables, "folders", None) or ():
        out.append({
            "key": str(f["key"]),
            "name": f["name"] or "Folder",
            "parent": None if f["parent"] is None else str(f["parent"]),
            "hidden": bool(f["hidden"]),
            "tags": [table[i] for i in f["tags"] if i in table],
        })
    return out


#: Spellings of the modeller's default tag: a face on it is on no layer.
_DEFAULT_TAGS = ("Layer0", "Layer 0", "Untagged")


def hidden_tags(names) -> frozenset:
    """The hidden tags ``names`` as the geometry may carry them: by name, and
    by the ids the last parse's table gives those names."""
    names = set(names)
    table = getattr(_parse_tables, "layers", None) or {}
    return frozenset(names | {i for i, n in table.items() if n in names})


def resolve_face_layers(model) -> int:
    """Turn the face layers that are tag ids into tag names, from the table
    of the parse that built ``model``. A face on the default tag, or on an
    id the table does not know, gets no layer. Returns how many changed."""
    table = getattr(_parse_tables, "layers", None) or {}
    definitions = dict(getattr(model, "definitions", {}) or {})
    if getattr(model, "root", None) is not None:
        definitions["ROOT"] = model.root
    for key, definition in definitions.items():
        for instance, pid in zip(getattr(definition, "instances", []),
                                 getattr(_parse_tables, "pids", {}).get(key, [])):
            instance.persistent_id = pid
        if getattr(definition, "is_group", False):
            _parse_tables.groups.add(definition.id)
        if hasattr(definition, "is_group"):
            _parse_tables.kinds = True
    changed = 0
    for defn in (getattr(model, "definitions", None) or {}).values():
        for face in (getattr(defn, "faces", None) or {}).values():
            lay = getattr(face, "layer", None)
            if isinstance(lay, bool) or not isinstance(lay, int):
                continue
            name = table.get(lay, "")
            face.layer = "" if name in _DEFAULT_TAGS else name
            changed += 1
    return changed


def _keep_vff_annotation_inputs() -> bool:
    """Wrap ``openskp._core._parse_dimensions`` so the VFF annotation
    records can be re-read after the parse: it is the one call that sees
    the raw model bytes together with both resolution tables (vertex
    positions and instance world transforms)."""
    try:
        from openskp import _core
    except Exception:  # noqa: BLE001
        return False
    original = getattr(_core, "_parse_dimensions", None)
    if original is None or getattr(original, "_keeps_vff", False):
        return False

    def _parse_dimensions(model_dat, id2pos, inst_world):
        _parse_tables.vff = (model_dat, id2pos, inst_world)
        return original(model_dat, id2pos, inst_world)

    _parse_dimensions._keeps_vff = True
    _core._parse_dimensions = _parse_dimensions
    return True


from ingetrazo_core import skp_definition_spans as _vff_definition_spans


def _scan_vff_dimensions(model_dat, id2pos, inst_world):
    from ingetrazo_core import skp_annotations
    return skp_annotations(True, model_dat, id2pos, inst_world)


def _scan_vff_texts(model_dat, id2pos, inst_world):
    from ingetrazo_core import skp_annotations
    return skp_annotations(False, model_dat, id2pos, inst_world)


def resolve_vff_annotations(model) -> int:
    from ingetrazo_core import skp_resolve_annotations
    inputs = getattr(_parse_tables, "vff", None)
    _parse_tables.vff = None
    return skp_resolve_annotations(model, *inputs) if inputs else 0


# ---- PBR materials (.skp 2025+) ----------------------------------------
# A 2025+ material.xml carries a <mat:pbrMR> block next to <mat:texture>; the
# maps it names sit in the material's own ZIP folder, beside the base image.
# OpenSKP 1.5 reads neither. Format: docs/skp-pbr-format.md. The reader runs
# inside the parse, where the ZIP is open: openskp calls ``_extract_texture``
# once per material (textured or not), so wrapping it sees every material.

#: ``Material.workflow`` values (the XML's ``workflow`` attribute).
WORKFLOW_CLASSIC, WORKFLOW_PBR = 0, 1
#: ``normalMapStyle`` values (the C API's ``SUMaterialNormalMapStyle``).
NORMAL_STYLE_OPENGL, NORMAL_STYLE_DIRECTX = 0, 1


from ingetrazo_core import skp_read_pbr as read_pbr


def _keep_pbr() -> bool:
    """Wrap ``openskp._core._extract_texture`` so each PBR material's recipe
    is read while the ZIP is open and kept by material name for the
    importer (:func:`pbr_for`). Steps aside once OpenSKP's own ``Material``
    carries ``pbr`` (the importer then reads that)."""
    try:
        from openskp import _core
        from openskp.model import Material
    except Exception:  # noqa: BLE001
        return False
    if "pbr" in getattr(Material, "__dataclass_fields__", {}):
        return False
    extract = getattr(_core, "_extract_texture", None)
    parse = getattr(_core, "full_parse", None)
    if extract is None or parse is None \
            or getattr(extract, "_keeps_pbr", False):
        return False

    def _extract_texture(zf, xml_name, mat_elem, ns):
        tex = extract(zf, xml_name, mat_elem, ns)
        try:
            pbr = read_pbr(zf, xml_name, mat_elem, ns)
        except Exception:  # noqa: BLE001 — never fail the open over PBR
            pbr = None
        if pbr is not None:
            table = getattr(_parse_tables, "pbr", None)
            if table is None:
                table = _parse_tables.pbr = {}
            table[mat_elem.get("name", "unknown")] = pbr
        return tex

    def full_parse(*args, **kwargs):
        _parse_tables.pbr = {}              # this parse's, not the last one's
        return parse(*args, **kwargs)

    _extract_texture._keeps_pbr = True
    full_parse._keeps_layers = getattr(parse, "_keeps_layers", False)
    _core._extract_texture = _extract_texture
    _core.full_parse = full_parse
    return True


def attach_pbr(model) -> int:
    """Give each ``Material`` of ``model`` (parsed last) its ``pbr`` recipe
    from :func:`_keep_pbr`'s table: ``None`` for a classic material. A
    material that already has one (upstream reads PBR) is left alone.
    Returns how many materials got a recipe."""
    table = getattr(_parse_tables, "pbr", None) or {}
    n = 0
    for mat in getattr(model, "materials", None) or ():
        if getattr(mat, "pbr", None) is not None:
            continue
        rec = table.get(getattr(mat, "name", None))
        try:
            mat.pbr = rec
        except AttributeError:  # a slotted dataclass upstream: leave it be
            continue
        n += rec is not None
    return n
