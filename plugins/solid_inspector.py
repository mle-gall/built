# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Solid Inspector plugin — find out WHY a solid is not watertight, and
repair what can be repaired without guessing.

IngeTrazo's engine guarantees hermetic solids and its BIM layer only
reports a volume when the object is closed. When Model Info (or the BIM
tray) says "(not watertight)", this plugin answers the next question:
*where is the hole?*

One row per solid candidate (the geometry you are editing, plus every
group). For each: face/edge counts, verdict, a breakdown of the exact
problems, and the volume when the object is closed. **Highlight** selects
the offending edges right in the viewport (orange, like any selection),
so you can see the hole instead of hunting for it; **Next problem** walks
them one at a time and zooms the view onto each.

Diagnosis (per edge, counting non-interior incident faces):

- ``0`` faces  → a **stray edge**: loose drawing, not part of any surface.
- ``1`` face   → an **open border**: the rim of a hole in the skin.
- ``>2`` faces → **overconnected**: an internal wall / T-junction the
  boundary should not have.

And per face:

- **internal**: a second face over the same corners as another, or a
  partition whose every edge is overconnected — a wall inside the solid
  that is not marked as one (:func:`core.orient.orient_outward` marks the
  ones the engine builds itself).
- **reversed**: on a closed object, a face whose front looks inward — the
  faces the engine's own orientation pass would turn.

Watertight = every edge has exactly 2. The volume comes from
:func:`core.bim.face_set_volume` — the same computation behind the BIM
quantities, so this dialog and your takeoff can never disagree.

**Repair** (one undo step, also on a group you are not inside) does only
what has a single right answer: erases stray edges and duplicated faces,
closes holes whose rim is flat, marks partitions as interior and turns
reversed faces outward. A non-flat hole or an edge shared by two solids
needs a decision, so it stays listed for you.
"""
from __future__ import annotations

from typing import Optional

from ingetrazo_core import Vec3 as QVector3D
from core.i18n import tr
from tools.base import Tool


# ---------------------------------------------------------------------------
# Diagnosis (pure data, no GUI — see tests/test_solid_inspector_plugin.py)
# ---------------------------------------------------------------------------

from core.solid_analysis import (
    _live, _steps, _step_key, _face_key,
    internal_faces, _copy_faces, reversed_faces, _signed_volume,
    inspect_mesh, is_sound, _problem_text, _edge_clusters,
    problem_items, _item_points, _border_loops, _flat,
    _agree_with_neighbour, _cap, why_not_repairable, repair_item,
    repair_mesh, RepairSolidsCommand, KINDS as _KIND_STYLES,
)

KINDS = _KIND_STYLES

# ---------------------------------------------------------------------------
# Dialog
# ---------------------------------------------------------------------------

def _group_rows(scene) -> list:
    """One row per group mesh to inspect, nested groups included: the
    pieces of an imported model live one level down, inside a container
    whose own mesh is empty. Each row knows the top-level object (what a
    click selects from outside) and the matrix that places it.

    A component's instances share one mesh: it is listed once, under the
    first instance — repairing it repairs them all. Billboards (scale
    figures, 2D cut-out trees/people) are flat by design — listing them as
    "not watertight" would be noise in every single model."""
    rows, seen = [], {id(scene.mesh)}      # the open group: the live row

    def walk(g, top, path, base):
        if getattr(g, "billboard", False):
            return
        m = getattr(g, "xform", None)
        if base is not None:
            m = base if m is None else base * m
        name = path + [g.name]
        mesh = g.mesh
        hollow = (not mesh.faces and not mesh.edges
                  and getattr(g, "children", None))
        if not hollow and id(mesh) not in seen:
            seen.add(id(mesh))
            rows.append({"name": " ▸ ".join(name), "kind": "group",
                         "group": g, "top": top, "matrix": m,
                         "mesh": mesh})
        for child in getattr(g, "children", None) or ():
            walk(child, top, name, m)

    for g in scene.groups:
        walk(g, g, [], None)
    return rows


def _matrix_of(scene, group):
    """The world matrix placing ``group`` (``None`` = already in world
    coordinates), wherever it is nested."""
    if group is None:
        return None
    from core.group import iter_placements
    for top in scene.groups:
        for g, m in iter_placements(top):
            if g is group:
                return m
    return getattr(group, "xform", None)


def _frame(vp, pts) -> None:
    """Fly the camera onto world points ``pts``, with room around them: a
    single short edge framed edge to edge gives no idea of where in the
    model it is."""
    lo = QVector3D(min(p.x() for p in pts), min(p.y() for p in pts),
                   min(p.z() for p in pts))
    hi = QVector3D(max(p.x() for p in pts), max(p.y() for p in pts),
                   max(p.z() for p in pts))
    pad = max((hi - lo).length() * 0.75, 0.05)
    vp.camera.fit_box(lo - QVector3D(pad, pad, pad),
                      hi + QVector3D(pad, pad, pad))


class SolidInspectorPanel:
    id, title, slot = "solids", tr("Solid Inspector"), "utility"
    notify_close = True

    def __init__(self, viewport):
        self._viewport = viewport
        self.visible = False
        self._last_key = None
        self._rows = []
        self.selected = 0
        self._current = None
        self._painted = []
        self.note = ""

    def show(self):
        self.visible = True
        if self._inspected_key() != self._last_key:
            self.refresh()

    on_open = show

    def close(self):
        self.visible = False
        self._current = None
        self._painted = []
        self.note = ""
        self._viewport.update()

    def _inspected_key(self):
        """Geometry/context plus every inspected mesh's mutation token.

        Browser rows can stay valid across geometry edits. Store writes
        can also bypass Mesh.revision entirely.
        Walk objects, never their elements, to detect same-count edits.
        """
        scene = self._viewport.scene

        def mesh_key(mesh):
            store = mesh.__dict__.get("_store")
            return (id(mesh), mesh.revision,
                    (id(store), store.lineage, store.revision)
                    if store is not None else None,
                    len(mesh.vertices), len(mesh.edges), len(mesh.faces))

        rows = [(mesh_key(scene.mesh),)]
        for row in _group_rows(scene):
            matrix = row["matrix"]
            rows.append((mesh_key(row["mesh"]), row["name"],
                         tuple(matrix.data()) if matrix is not None else None))
        return (id(scene), scene.geometry_version, id(scene.edit_group),
                len(scene.groups), tuple(rows))

    def refresh(self) -> None:
        """Inspect everything again. The problem on screen is let go: what
        it pointed at may have changed."""
        if not self.visible:
            self._last_key = None
            return
        scene = self._viewport.scene
        current = self._selected_row()
        current_mesh = current["mesh"] if current else None
        self._rows = []

        # The mesh you are editing right now: the open group's, or the
        # loose one. Problem edges here CAN be highlighted directly.
        if scene.edit_group is not None:
            active_name = tr("Open group: {name}",
                             name=scene.edit_group.name)
        else:
            active_name = tr("Loose geometry")
        self._rows.append({"name": active_name, "kind": "active",
                           "mesh": scene.mesh,
                           "matrix": _matrix_of(scene, scene.edit_group),
                           "report": inspect_mesh(scene.mesh)})

        # Every group, nested ones included (the one being edited is the
        # live row above; skip it here to avoid the duplicate).
        for row in _group_rows(scene):
            row["report"] = inspect_mesh(row["mesh"])
            self._rows.append(row)

        self.selected = next((i for i, row in enumerate(self._rows)
                              if row["mesh"] is current_mesh), 0)
        self._current = None
        self._painted = []
        self.note = ""
        self._last_key = self._inspected_key()

    def _selected_row(self):
        if 0 <= self.selected < len(self._rows):
            return self._rows[self.selected]

    def sections(self):
        from core.plugin_panels import section, item
        from core.units import fmt_volume
        if not self.visible:
            return [section("inspector", self.title, item("button", "inspect", tr("Inspect")))]
        self.show()
        rows = []
        for i, row in enumerate(self._rows):
            rep = row["report"]
            if not rep["faces"] and not rep["edges"]:
                status = tr("(empty)")
            elif is_sound(rep):
                status = "✔ " + tr("watertight")
            else:
                status = "✘ " + tr("faces reversed" if rep["watertight"] else "not watertight")
            volume = fmt_volume(rep["volume"]) if rep["volume"] is not None else "—"
            rows.append(dict(key=str(i), cells=[row["name"], str(rep["faces"]), status,
                                                _problem_text(rep), volume]))
        controls = [item("table", "objects", columns=list(map(tr, ("Object", "Faces", "Status", "Problems", "Volume"))),
                         rows=rows, selection=str(self.selected)),
                    item("log", "details", lines=[self.note])]
        for key, label in (("previous", "Previous problem"), ("next", "Next problem"),
                           ("highlight", "Highlight problems"), ("repair_current", "Repair this problem"),
                           ("repair", "Repair object"), ("repair_all", "Repair all")):
            controls.append(item("button", key, tr(label), enabled=key != "repair_current" or self.can_fix))
        return [section("inspector", self.title, *controls)]

    @property
    def can_fix(self):
        cur = self._current
        return cur is not None and not why_not_repairable(cur["mesh"], cur["item"])

    def event(self, control, event, value):
        if control == "objects" and event == "selection":
            self.selected = int(value)
            self.focus_object()
        elif control == "_panel" and event == "closed":
            self.close()
        elif event == "invoke":
            actions = dict(inspect=self.show, previous=self.previous_problem, next=self.next_problem,
                           highlight=self.highlight, repair_current=self.repair_current,
                           repair=self.repair, repair_all=self.repair_all)
            if control in actions:
                actions[control]()

    def focus_object(self) -> bool:
        """What a click on the list does: an object with problems shows its
        first one — framed, painted, «Repair this problem» ready — and a
        sound one is flown to and selected. False when there is nothing to
        show (an empty row, or the table being rebuilt)."""
        row = self._selected_row()
        vp = self._viewport
        if row is None:
            return False
        if problem_items(row["report"]):
            return self.show_problem(0) is not None
        self._current = None
        self._painted = []
        self._show_label()
        pts = [v.position for v in row["mesh"].vertices if v.edges]
        m = row["matrix"]
        if m is not None:
            pts = [m.map(p) for p in pts]
        scene = vp.scene
        scene.selection.clear()
        if row["kind"] != "active":
            scene.selection.add(row["top"])
        if not pts:
            vp.update()
            return False
        _frame(vp, pts)
        self._show_label(tr("No problems left in this object"))
        vp.update()
        return True

    def highlight(self) -> Optional[list]:
        """Paint EVERY problem of the chosen row at once, each in its
        kind's colour, and frame them all — the whole picture before going
        through them one by one. Returns the problems painted (None when
        there are none).

        The problems are also selected when they are the geometry you are
        editing; on a group you are not inside, the group is selected and
        the colours show its problems through it."""
        row = self._selected_row()
        vp = self._viewport
        if row is None:
            return None
        items = problem_items(row["report"])
        self._current = None
        if not items:
            self._painted = []
            if row["kind"] != "active":
                vp.scene.selection.clear()
                vp.scene.selection.add(row["top"])
            self._show_label(tr("No problems left in this object"))
            vp.flash_status(tr("Nothing to highlight — watertight"))
            vp.update()
            return None
        self._painted = items
        self._painted_matrix = row["matrix"]
        scene = vp.scene
        scene.selection.clear()
        if row["kind"] == "active":
            for item in items:
                scene.selection.update(item["ents"])
        else:
            scene.selection.add(row["top"])
        pts = [p for item in items for p in _item_points(item)]
        m = row["matrix"]
        if m is not None:
            pts = [m.map(p) for p in pts]
        _frame(vp, pts)
        self._show_label()
        vp.flash_status(tr("{n} problems highlighted", n=len(items)))
        vp.update()
        return items

    # ---- One problem at a time ---------------------------------------------
    def next_problem(self) -> Optional[dict]:
        """Show the chosen row's next problem (the first, the first time).
        Returns it — ``{"kind", "ents"}`` — or None when there is none."""
        return self._step(+1)

    def previous_problem(self) -> Optional[dict]:
        return self._step(-1)

    def _step(self, delta: int) -> Optional[dict]:
        row = self._selected_row()
        if row is None:
            return None
        cur = self._current
        if cur is not None and cur["name"] == row["name"]:
            index = cur["index"] + delta
        else:
            index = 0 if delta > 0 else -1
        return self.show_problem(index)

    def show_problem(self, index: int) -> Optional[dict]:
        """Frame problem ``index`` of the chosen row (wrapping round), paint
        it in its kind's colour and select it.

        On a group you are not inside, the group is what gets selected —
        the view still flies to the problem, in the group's placement, and
        the colour shows it through the group."""
        row = self._selected_row()
        vp = self._viewport
        if row is None:
            return None
        items = problem_items(row["report"])
        if not items:
            self._current = None
            self._painted = []
            self._show_label(tr("No problems left in this object"))
            vp.flash_status(tr("Nothing to highlight — watertight"))
            vp.update()
            return None
        index %= len(items)
        item = items[index]
        self._current = {"name": row["name"], "index": index,
                         "count": len(items), "item": item,
                         "mesh": row["mesh"], "matrix": row["matrix"]}
        self._painted = []

        pts = _item_points(item)
        m = row["matrix"]
        if m is not None:
            pts = [m.map(p) for p in pts]
        scene = vp.scene
        scene.selection.clear()
        if row["kind"] == "active":
            scene.selection.update(item["ents"])
        else:
            scene.selection.add(row["top"])
        _frame(vp, pts)
        self._show_label()
        vp.flash_status(tr("Problem {i} of {n}", i=index + 1, n=len(items)))
        vp.update()
        return item

    def _show_label(self, note=""):
        self.note = note
        if self._current:
            cur = self._current
            name = KINDS[cur["item"]["kind"]][0]
            self.note = tr("Problem {i} of {n}: {kind}", i=cur["index"] + 1,
                           n=cur["count"], kind=tr(name))
            reason = why_not_repairable(cur["mesh"], cur["item"])
            if reason:
                self.note += " · " + reason
        elif self._painted:
            from collections import Counter
            counts = Counter(item["kind"] for item in self._painted)
            self.note = "   ".join(f"{tr(KINDS[k][0])} ({n})" for k, n in counts.items())

    def overlay(self, vp):
        from views.wgpu_overlay import Primitive
        if self._current:
            items, matrix = [self._current["item"]], self._current["matrix"]
        elif self._painted:
            items, matrix = self._painted, self._painted_matrix
        else:
            return
        for problem in items:
            color = "#%02x%02x%02x" % tuple(KINDS[problem["kind"]][1])
            for entity in problem["ents"]:
                face = hasattr(entity, "loop")
                vertices = entity.loop if face else (entity.v0, entity.v1)
                points = [vp._world_to_pixel(matrix.map(v.position) if matrix is not None else v.position)
                          for v in vertices]
                if None not in points:
                    yield Primitive("polygon" if face else "polyline", points, color,
                                    3 if face else 5, fill="#6e" + color[1:] if face else None)

    def repair_current(self) -> Optional[dict]:
        """Repair the problem on screen (one undo step) and show the next
        one of the same object. None when there was nothing to repair or
        it has no single right repair (the reason is shown)."""
        cur = self._current
        vp = self._viewport
        if cur is None:
            vp.flash_status(tr("Press Next problem first"))
            return None
        reason = why_not_repairable(cur["mesh"], cur["item"])
        if reason:
            vp.flash_status(reason, 5000)
            return None
        item = cur["item"]
        cmd = RepairSolidsCommand([cur["mesh"]],
                                  fix=lambda mesh: repair_item(mesh, item))
        vp.history.execute(cmd)
        if vp.history.last_error:
            vp.flash_status(vp.history.last_error)
            self.refresh()
            return None
        index = cur["index"]
        self.refresh()
        # The repaired problem left the list: the one now in its place is
        # the next one.
        row = self._selected_row()
        left = problem_items(row["report"]) if row else []
        self.show_problem(min(index, len(left) - 1) if left else 0)
        vp.flash_status(tr("Repaired — {n} problems left in this object",
                           n=len(left)), 4000)
        return cmd.done

    # ---- Repair ------------------------------------------------------------
    def _repair_rows(self, rows) -> Optional[dict]:
        meshes, seen = [], set()
        for row in rows:
            if is_sound(row["report"]) or not row["report"]["edges"]:
                continue
            if id(row["mesh"]) not in seen:   # instances share a prototype
                seen.add(id(row["mesh"]))
                meshes.append(row["mesh"])
        vp = self._viewport
        if not meshes:
            vp.flash_status(tr("Nothing to repair"))
            return None
        cmd = RepairSolidsCommand(meshes)
        vp.history.execute(cmd)
        if vp.history.last_error:
            vp.flash_status(vp.history.last_error)
            self.refresh()
            return None
        d = cmd.done
        vp.flash_status(tr(
            "Repaired: {stray} stray edges, {dup} duplicated faces, "
            "{holes} holes closed, {inner} partitions, {rev} faces turned",
            stray=d["stray"], dup=d["duplicates"], holes=d["holes"],
            inner=d["partitions"], rev=d["reversed"]), 5000)
        vp.update()
        self.refresh()
        return d

    def repair(self) -> Optional[dict]:
        """Repair the chosen row's object (one undo step)."""
        row = self._selected_row()
        return self._repair_rows([row]) if row is not None else None

    def repair_all(self) -> Optional[dict]:
        """Repair every listed object with problems (one undo step)."""
        return self._repair_rows(self._rows)


# ---------------------------------------------------------------------------
# Tool registration (the plugin entry point)
# ---------------------------------------------------------------------------

#: The tests and older code name it by its dialog-era name.
SolidInspectorDialog = SolidInspectorPanel


class SolidInspectorTool(Tool):
    """Extensions-menu entry that brings the inspector's tab forward (or,
    in a window without the side tray, opens it as a window of its own)."""
    name = "Solid Inspector"
    shortcut = None
    description = (
        "Find out why a group is not a solid, show the edges to blame and "
        "repair what has one right answer.")
    uses_snap = False

    def on_activate(self, viewport):
        from core.plugin_panels import open_panel
        open_panel(viewport.window(), "solid_inspector")

    def on_deactivate(self, viewport):
        pass


def create_panel(app, invalidate):
    return SolidInspectorPanel(app.viewport)
