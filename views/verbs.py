# SPDX-License-Identifier: GPL-3.0-or-later
"""Glue to the one verb source, ``rust/ingetrazo_render/src/verbs.rs``: the
selection's kind, the verbs that apply, their labels and keys, the menu's
rows, the inspector's chips and More, the linked-file rows' verbs and what
runs each verb all live there (architecture review 7, council B5).

Python only reads the document into the facts Rust decides from
(:func:`facts`); ``views/context_menu.py`` runs the dispatch Rust names.
"""
from __future__ import annotations

import json

from ingetrazo_render import (text_editable, verbs_constants, verbs_dispatch, verbs_file, verbs_inspector,  # noqa: F401
                              verbs_kind, verbs_menu, verbs_name_editable, verbs_offered,
                              verbs_read_only)

KINDS, READ_ONLY, EDITING, MAX_ROWS = (tuple(c) if isinstance(c, list) else c for c in verbs_constants())


def _type(e) -> str:
    return type(e).__name__


def facts(scene, sel, win=None, view=None, *, locked_group=None, locked_image=None) -> dict:
    """What Rust needs to know about the selection ``sel``: counts and
    flags read from the document, and the header's names and measures.
    With ``win``, also what the window can do now (Repeat, Hide, Paste…)."""
    from core import links
    from core.group import Group
    from core.mesh import Edge, Face
    sel = list(sel)
    groups = [e for e in sel if isinstance(e, Group)]
    faces = [e for e in sel if isinstance(e, Face)]
    edges = [e for e in sel if isinstance(e, Edge)]
    linked_part = any(links.linked_ancestor(g) is not None and not links.is_linked(g) for g in groups)
    f = dict(total=len(sel), groups=len(groups), faces=len(faces), edges=len(edges),
             components=sum(1 for g in groups if g.is_component()),
             linked_part=linked_part,
             all_links=bool(groups) and all(links.is_linked(g) for g in groups),
             all_annotations=bool(sel) and all(_type(e) in ("Dimension", "TextLabel") for e in sel),
             all_guides=bool(sel) and all(_guide(e) for e in sel),
             all_sections=bool(sel) and all(_type(e) == "SectionPlane" for e in sel),
             all_images=bool(sel) and all(_type(e) == "ImagePlane" for e in sel),
             all_paths=bool(sel) and all(_type(e) == "GeoPath" for e in sel),
             all_locked=bool(groups) and all(getattr(g, "locked", False) for g in groups))
    if win is None:
        return f
    vp = win.viewport
    one = sel[0] if len(sel) == 1 else None
    last = getattr(win, "_last_command", None)
    if last is not None:
        f["repeat"] = last[0]
        f["repeat_value"] = _repeat_value(win, last[0])
    if locked_group is not None and locked_group not in sel:
        f["locked_group"] = getattr(locked_group, "name", "") or ("Component" if locked_group.is_component() else "Group")
    if locked_image is not None and locked_image not in sel:
        f["locked_image"] = getattr(locked_image, "name", "") or "image"
    geometry = bool(sel) and len(faces) + len(edges) == len(sel)
    from ingetrazo_render import has_tag_actions
    f.update(taggable=has_tag_actions(scene, sel),
             hideable=bool(win._hideable(sel)),
             makes_faces=bool(geometry and win._makes_faces(sel)),
             textured_face=len(faces) == 1 == len(sel) and win._single_textured_face() is not None,
             bridgeable=bool(geometry and win._bridgeable(sel)),
             context=_context_path(scene))
    loose = [e for e in edges if e in scene.mesh.edges]
    f.update(loose_edges=len(loose), curve=any(getattr(e, "curve", None) is not None for e in loose))
    if isinstance(one, Group):
        editable = text_editable(scene, one)
        f.update(name=getattr(one, "name", "") or "", filed=_filed_path(scene, one),
                 text3d=editable is not None, edited_letters=editable is False,
                 billboard=bool(getattr(one, "billboard", False)))
        if one.is_component():
            from core.kept_components import kept_template
            from core.purge import iter_groups
            f.update(instances=sum(1 for o in iter_groups(scene.groups or ()) if o.mesh is one.mesh),
                     replaceable=bool(win._replacement_choices(sel)),
                     kept=kept_template(scene, one.mesh) is not None)
        rec = links.link_of(one)
        if rec:
            from pathlib import Path
            f.update(file=Path(rec.get("path", "")).name, include_nested=bool(rec.get("include_nested", True)))
    if linked_part:
        part = groups[0]
        host = links.linked_ancestor(part)
        f.update(name=getattr(part, "name", "") or "", host=host is not None)
        if host is not None:
            from pathlib import Path
            f["host_file"] = Path((links.link_of(host) or {}).get("path", "")).name
    if len(groups) > 1 and len(groups) == len(sel):
        from core.solids import is_solid
        f["solids"] = sum(1 for g in groups if is_solid(g))
    if len(faces) == 1:
        f["area"] = _measure("fmt_area", faces[0].area)
    if len(edges) == 1:
        f["length"] = _measure("fmt_len", lambda: (edges[0].b - edges[0].a).length())
    if one is not None and _type(one) in ("Dimension", "TextLabel"):
        f["dimension"] = _type(one) == "Dimension"
        try:
            f["text"] = (one.text or one.label()) if f["dimension"] else \
                ((getattr(one, "text", "") or "").strip().splitlines() or [""])[0][:40]
        except Exception:  # noqa: BLE001
            f["text"] = ""
    if one is not None and _guide(one):
        f["guide_point"] = getattr(one, "direction", None) is None or _type(one) == "GuidePoint"
    if one is not None and _type(one) in ("SectionPlane", "ImagePlane"):
        f["name"] = getattr(one, "name", "") or ""
        f["section_active"] = bool(getattr(one, "active", False))
        f["image_locked"] = bool(getattr(one, "locked", False))
    f["path_polygon"] = any(len(getattr(e, "points", ()) or ()) >= 3 for e in sel if _type(e) == "GeoPath")
    if not sel:
        available = getattr(win, "_clipboard_available", None)
        if available is None:
            from formats.clip import available
        f.update(paste=bool(getattr(vp, "clipboard", None) or available()),
                 isolated=vp.__dict__.get("_isolate_base") is not None or bool(getattr(vp, "_previous_views", None)),
                 guides=bool(list(getattr(scene, "guides", ()) or ())))
    elif view is not None:
        from ingetrazo_render import shortcut_text
        f["reveal_key"] = shortcut_text("Ctrl+Shift+J")[1]
    return f


def _guide(e) -> bool:
    try:
        from core.guide import is_guide
        return bool(is_guide(e))
    except Exception:  # noqa: BLE001
        return False


def _measure(fmt, value) -> str:
    try:
        from core import units
        return getattr(units, fmt)(float(value()))
    except Exception:  # noqa: BLE001
        return ""


def _repeat_value(win, label) -> str:
    """The value the last command runs with, when it has one (Push/Pull's
    distance, which a double-click repeats)."""
    for tool in (getattr(win, "_tools", {}) or {}).values():
        if getattr(tool, "name", None) == label:
            value = getattr(type(tool), "last_distance", None)
            if isinstance(value, (int, float)) and value:
                return _measure("fmt_len", lambda: abs(value))
    return ""


def _context_path(scene) -> str:
    """Where loose geometry lives: the groups open for editing, else Model."""
    from core.outliner import open_path
    names = [getattr(g, "name", "") or ("Component" if g is not None and g.is_component() else "Group")
             for g in open_path(scene)]
    return " › ".join(names) if names else "Model"


def _filed_path(scene, g) -> str:
    """The group's node path in the project tree, or "Unfiled"."""
    from core import bim_registry, structure
    try:
        key = structure.node_of(g)
        if key:
            return " › ".join(structure.path(bim_registry.get(scene), key))
    except Exception:  # noqa: BLE001 — a stale key: no path, not an error
        pass
    return "Unfiled"


def kind_of(scene, sel) -> str:
    """The kind of the selection ``sel`` (docs/context-menu.md)."""
    return verbs_kind(json.dumps(facts(scene, sel)))


def read_only(kind) -> bool:
    return verbs_read_only(kind)


def offers(kind, verb) -> bool:
    """Whether the inspector of ``kind`` offers ``verb``."""
    return verbs_offered(kind, verb)


def name_editable(kind) -> bool:
    """A name field edits in place only on this document's own things."""
    return verbs_name_editable(kind)


def file_verbs(nested: bool, state: str = "resolved") -> tuple:
    """The verb ids of a linked file's tree row."""
    return tuple(v for v, _ in verbs_file(nested, state))


def file_verb_rows(nested: bool, state: str) -> list:
    """(id, label) of a linked file's tree row."""
    return verbs_file(nested, state)
