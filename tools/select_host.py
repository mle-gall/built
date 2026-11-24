# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Select's viewport actions and compatibility paths for materialised meshes.
Store-backed meshes grow picks through native incidence. The native viewport
box-selects placements from the renderer's packed prototype arrays.
"""
from __future__ import annotations


from ingetrazo_core import (box_records, box_placements, box_chunk_group, chunk_bounds as _local_bounds, box_from_pack as _box_groups_packed,
                            segment_overlaps_box as _seg_rect_overlap)

from core.dimension import Dimension
from core.textlabel import TextLabel
from core.group import Group
from core.mesh import Edge, Face
from core.guide import is_guide
from core.section import SectionPlane
from core.history import (
    CompoundCommand,
    DeleteDimensionsCommand,
    DeleteGeoPathsCommand,
    DeleteGroupCommand,
    DeleteGuidesCommand,
    DeleteSectionPlanesCommand,
    DeleteTextLabelsCommand,
    SetActiveSectionCommand,
    EditTextLabelCommand,
    EraseSelectionCommand,
)
from georef.geopath import GeoPath



def _seg_rect_mask(ax, ay, bx, by, ok, rect, crossing):
    """The rectangle test of N screen segments: window mode = both
    endpoints inside; crossing mode = an endpoint inside or the segment
    through one of the four borders (``ingetrazo_core.seg_rect_mask``)."""
    import numpy as np
    from ingetrazo_core import seg_rect_mask
    f = lambda a: np.ascontiguousarray(a, dtype=np.float64).reshape(-1)
    n = len(f(ax))
    return seg_rect_mask(f(ax), f(ay), f(bx), f(by),
                         np.ascontiguousarray(np.broadcast_to(ok, (n,)), dtype=bool),
                         tuple(map(float, rect)), bool(crossing))


def _box_loose_fast(viewport, rect, crossing):
    """Vectorized box select over the loose mesh: edges via the cached
    screen projection, faces derived from their boundary edges. Returns
    (edges, faces) or None to fall back to the per-entity walk (stub
    viewports in tests, or a viewport without the caches)."""
    if (getattr(viewport, "_ledge_screen", None) is None
            or getattr(viewport, "_pick_index", None) is None):
        return None
    try:
        proj = viewport._ledge_screen()
        idx = viewport._pick_index()
    except Exception:  # noqa: BLE001 — any stub oddity: python path
        return None
    if proj is None or idx.edge_a is None:
        return None
    import numpy as np
    ax, ay, bx, by, ok = proj
    edges_all = idx.edges
    if len(edges_all) != len(ax):
        return None
    mask = _seg_rect_mask(ax, ay, bx, by, ok, rect, crossing)
    sel = viewport.scene.entity_selectable
    # What the context lets a click take. The index also carries the rest of
    # the model's loose edges (a reference while a group is open); those are
    # drawn faded and must not be caught by the box.
    pick_ok = getattr(idx, "edge_sel", None)
    hit_idx = np.where(mask)[0]
    if pick_ok is not None:
        hit_idx = hit_idx[pick_ok[hit_idx]]
    edges = [edges_all[int(i)] for i in hit_idx
             if sel(edges_all[int(i)])
             and not getattr(edges_all[int(i)], "hidden", False)]
    faces = []
    if crossing:
        seen = set()
        for i in hit_idx:
            for f in edges_all[int(i)].faces:
                if f.key not in seen:
                    seen.add(f.key)
                    if sel(f):
                        faces.append(f)
    else:
        hit_ids = {edges_all[int(i)].key for i in hit_idx}
        face_edges: dict = {}
        for e in edges_all:
            for f in e.faces:
                face_edges.setdefault(f.key, []).append(e)
        for f in viewport.scene.faces:
            es = face_edges.get(f.key)
            if es and all(e.key in hit_ids for e in es) and sel(f):
                faces.append(f)
    return edges, faces


def _box_loose_arrays(viewport, rect, crossing):
    """The loose mesh's box catch from the store's dense arrays
    (``ingetrazo_core.box_mesh_arrays``): element views made only for
    what the box takes. Returns (edges, faces), or None when the loose
    mesh is not store-backed or the view cannot project."""
    camera_px = getattr(viewport, "_camera_px", None)
    scene = viewport.scene
    mesh = getattr(scene, "mesh", None)
    if camera_px is None or mesh is None:
        return None
    from core.store_mesh import StoreMesh
    if mesh.__class__ is not StoreMesh:
        return None
    try:
        mvp, w, h = camera_px()
    except (AttributeError, TypeError):
        return None
    emask, fmask = mesh.__dict__["_store"].box_pick(mvp, w, h, rect, crossing)
    sel = scene.entity_selectable
    seq = mesh.edges
    edges = [e for e in (seq[int(i)] for i in emask) if sel(e)]
    seq = mesh.faces
    faces = [f for f in (seq[int(i)] for i in fmask) if sel(f)]
    return edges, faces


def _box_group_fast(viewport, group, rect, crossing):
    try:
        return box_chunk_group(viewport, group, rect, crossing)
    except (AttributeError, TypeError, ValueError, KeyError, RuntimeError):
        # A scripted viewport may omit its chunk or projection.
        return None


def describe_pick(entity) -> str:
    """A few words for the status bar naming what a click picked."""
    from core.i18n import tr
    if isinstance(entity, Group):
        kind = tr("Component") if entity.is_instance() else tr("Group")
        name = (getattr(entity, "name", "") or "").strip()
        return f"{kind} «{name}»" if name else kind
    for cls, word in ((Edge, "Edge"), (Face, "Face"), (Dimension, "Dimension"),
                      (TextLabel, "Text"), (SectionPlane, "Section Plane")):
        if isinstance(entity, cls):
            return tr(word)
    return tr("Image") if type(entity).__name__ == "ImagePlane" \
        else type(entity).__name__


#: A repeated plain click within this many pixels takes the next candidate
#: (the machine's ``CYCLE_PX``).
CYCLE_PX = 4.0


def cycle_status(viewport, index: int, count: int, entity) -> None:
    flash = getattr(viewport, "flash_status", None)
    if callable(flash):
        from core.i18n import tr
        flash(tr("{index} of {count}: {what} — click again for "
                 "the next").format(index=index + 1, count=count,
                                    what=describe_pick(entity)))


def _bounded(viewport, entity) -> list:
    """A double click's catch: a face with its bounding edges, an edge
    with its faces."""
    mesh = viewport.scene.mesh
    picked = list(expand_pick(viewport, entity))
    if isinstance(entity, Face):
        for f in list(picked):
            if not isinstance(f, Face):
                continue
            for lp in (f.loop, *f.hole_loops):
                n = len(lp)
                for i in range(n):
                    e = mesh.find_edge(lp[i], lp[(i + 1) % n])
                    if e is not None:
                        picked.append(e)
    else:
        for e in list(picked):
            if isinstance(e, Edge):
                picked.extend(e.faces)
    return picked


def apply(viewport, entities, mode: str, grow: str) -> None:
    """Select what the machine picked, grown as it says: ``natural`` (a
    smooth surface, a whole curve), ``bounded`` (double click) or
    ``connected`` (triple click: the whole solid)."""
    picked = []
    for entity in entities:
        if grow != "natural" and isinstance(entity, (Face, Edge)):
            query = getattr(viewport.scene.mesh, "grow_pick", None)
            result = query(entity, grow) if query else None
            if result is not None:
                picked.extend(result)
                continue
        if grow == "connected":
            from core.select_ops import all_connected
            picked.extend(all_connected([entity]))
        elif grow == "bounded":
            picked.extend(_bounded(viewport, entity))
        else:
            picked.extend(expand_pick(viewport, entity))
    viewport.scene.select(picked, mode=mode)


def _toggle_section(viewport, entity) -> None:
    # Double-clicking a section plane toggles the active cut.
    viewport.history.execute(SetActiveSectionCommand(
        None if entity.active else entity))
    viewport.scene.select([entity])


def _edit_text(viewport, entity) -> None:
    # As usual, double-clicking a leader text edits its text.
    from core.i18n import tr
    from views import chrome_forms

    def edit(text):
        if text is not None and text.strip() and text.strip() != entity.text:
            viewport.history.execute(
                EditTextLabelCommand(entity, text.strip()))
        viewport.update()
    chrome_forms.ask_text(viewport.window(), tr("Text"), tr("Label text:"),
                          entity.text, done=edit, multiline=True)


def _edit_dimension(viewport, entity) -> None:
    # Double-clicking the dimension text edits it; "<>" stands for the
    # measured value, and an empty text (or a bare "<>") goes back to the
    # automatic value.
    from core.i18n import tr
    from core.history import EditDimensionTextCommand
    fmt = getattr(viewport, "_format_dim_value", None)
    style = getattr(viewport.scene, "dimension_style", {}) or {}
    measured = (fmt(entity.value(), style) if fmt is not None
                else entity.label())
    current = entity.text if entity.text else measured
    from views import chrome_forms

    def retext(text):
        if text is not None:
            new = text.strip()
            if new in ("", "<>", measured):
                new = None
            if new != entity.text:
                viewport.history.execute(
                    EditDimensionTextCommand(entity, new))
        viewport.update()
    chrome_forms.ask_text(viewport.window(), tr("Dimension"),
                          tr("Dimension text (<> = measured value):"),
                          current, done=retext)


#: The double-click verbs the host still owns (the machine names them).
HOST_VERBS = {"toggle_section": _toggle_section, "edit_text": _edit_text,
              "edit_dimension": _edit_dimension}


def box_select(viewport, rect, crossing: bool, mode: str) -> None:
    """What a released box catches (``rect`` normalised, ``crossing`` when
    dragged right to left), selected the way ``mode`` says."""
    picked = []
    fast = _box_loose_fast(viewport, rect, crossing)
    if fast is None:
        fast = _box_loose_arrays(viewport, rect, crossing)
    if fast is not None:
        picked.extend(fast[0])
        picked.extend(fast[1])
    else:
        picked.extend(box_records(viewport, rect, crossing, loose=True))
    ctx = viewport.scene.edit_group
    candidates = (getattr(viewport.scene, "groups", []) if ctx is None
                  else (getattr(ctx, "children", None) or []))
    if candidates:
        candidates = [g for g in candidates
                      if viewport.scene.entity_selectable(g)]
        packed = _box_groups_packed(viewport, candidates, rect, crossing)
        for group in candidates:
            if packed is not None and id(group) in packed[1]:
                if id(group) in packed[0]:
                    picked.append(group)
                continue
            verdict = _box_group_fast(viewport, group, rect, crossing)
            if verdict is not None:
                if verdict:
                    picked.append(group)
                continue
            from core.group import iter_placements
            if box_placements(viewport, rect, crossing, iter_placements(group)):
                picked.append(group)
    picked.extend(box_records(viewport, rect, crossing, loose=False))
    if mode == "replace" and getattr(viewport, "extension_pick",
                                     None) is not None:
        viewport.clear_extension_pick()
    viewport.scene.select(picked, mode=mode)
    viewport.update()


# ---- Delete: the selection, or what is under the cursor ------------------------

def expand_pick(viewport, entity) -> list:
    """Grow a pick to its natural whole: a curved surface (faces joined by
    soft edges) for a face, or the whole drawn curve (circle/arc) for one of
    its segments — the classic behaviour. Plain entities select alone."""
    if isinstance(entity, Face):
        return viewport.scene.mesh.surface_of(entity)
    if isinstance(entity, Edge) and getattr(entity, "curve", None) is not None:
        return viewport.scene.mesh.curve_edges(entity)
    return [entity]


def erase_entities(viewport, entities) -> bool:
    """Erase ``entities`` (any mix of pickable types) as ONE undoable step.
    True if anything was erased."""
    edges = [e for e in entities if isinstance(e, Edge)]
    faces = [f for f in entities if isinstance(f, Face)]
    groups = [g for g in entities if isinstance(g, Group)]
    dims = [d for d in entities if isinstance(d, Dimension)]
    labels = [t for t in entities if isinstance(t, TextLabel)]
    paths = [p for p in entities if isinstance(p, GeoPath)]
    guides = [g for g in entities if is_guide(g)]
    splanes = [p for p in entities if isinstance(p, SectionPlane)]
    commands = []
    if edges or faces:
        # Erasing an edge between two coplanar faces merges them back
        # into one (as usual); any other erased edge takes its faces.
        commands.append(EraseSelectionCommand(edges, faces))
    commands.extend(DeleteGroupCommand(g) for g in groups)
    if guides:
        commands.append(DeleteGuidesCommand(guides))
    if splanes:
        commands.append(DeleteSectionPlanesCommand(splanes))
    if dims:
        commands.append(DeleteDimensionsCommand(dims))
    if labels:
        commands.append(DeleteTextLabelsCommand(labels))
    if paths:
        commands.append(DeleteGeoPathsCommand(paths))
    if not commands:
        return False
    cmd = commands[0] if len(commands) == 1 else CompoundCommand(commands)
    viewport.history.execute(cmd)
    viewport.update()
    return True


def delete_selection_or_hover(viewport) -> bool:
    """The Delete key: erase the selection; with NOTHING selected, erase
    what is highlighted under the cursor — hover an edge or a face and press
    Supr, no click needed. The hover grows like a click would (a whole
    circle, a whole smooth surface), so what goes is what a click-then-Supr
    would have erased. True if anything was erased."""
    if getattr(viewport, "extension_pick", None) is not None:
        return viewport.delete_extension_pick()      # a render light, say
    selection = list(viewport.scene.selection)
    if selection:
        return erase_entities(viewport, selection)
    hovered = getattr(viewport, "_hover_entity", None)
    if hovered is None:
        return False
    viewport.set_hover(None)        # it is about to be gone
    return erase_entities(viewport, expand_pick(viewport, hovered))
