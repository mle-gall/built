# SPDX-License-Identifier: GPL-3.0-or-later
"""Document ownership and orchestration of the Python modelling tools."""
from __future__ import annotations

from typing import Optional

from core.camera import OrbitCamera
from core.history import History
from core.i18n import tr
from core.inference import InferenceEngine
from core.scene import Scene
from tools.base import Tool


def _context_stamp(vp):
    feed = getattr(getattr(vp, "history", None), "feed", None)
    return getattr(vp.scene, "content_version", None), feed.readers.stamp() if feed else 0


def _ctx_changed(vp, before):
    """``History.context_changed`` (stub viewports in tests have no
    history, or one without it)."""
    cc = getattr(getattr(vp, "history", None), "context_changed", None)
    if cc is not None:
        cc(before)


class DocumentTools:
    suspended_tool: Optional[Tool] = None

    def _init_document(self):
        self.camera = OrbitCamera()
        self.scene = Scene()
        self._bind_bake_store()
        from core import units as _units
        _units.bind_scene(self.scene)   # readouts format in the model's units
        self.history = History(self.scene)
        # Snapping inference state (hover / acquired edges and points,
        # the encouraged points, the axis and reference locks, the last
        # snap) gathered in its own collaborator. The delegating
        # properties further down keep the old attribute names reading
        # and writing through to it.
        self.inference = InferenceEngine(self)
        self.active_tool: Optional[Tool] = None
        # Extensions' overlays and snap providers (views.extension_api).
        self._ext_overlays: list = []
        self._ext_snap_providers: list = []
        # Copy/paste clipboard: copied geometry (faces + edges as positions,
        # groups as snapshot copies) plus a reference corner so Paste can
        # place it under the cursor.
        self.clipboard: Optional[dict] = None

    def set_active_tool(self, tool: Optional[Tool]) -> None:
        suspended = self.suspended_tool
        if suspended is not None:
            self.suspended_tool = None
            if tool is suspended:
                self._resume_tool(tool)
                return
            # Any other tool: the parked one ends as a switch ends it.
            suspended.on_deactivate(self)
        if self.active_tool is tool and self.nav_mode is None:
            # Picking the tool you already hold starts it over, the classic
            # behaviour: the first point is released, the locks let go. Issue
            # #34 (@pacaeiro): «If I'm drawing a line and have the first
            # point defined… if I press (L) again, the command should
            # reset». It used to be a no-op.
            if tool is not None:
                tool.on_cancel(self)
            self.inference.clear_references()
            self.axis_lock = None
            self._shift_lock = None
            self.reference_edge = None
            self.reference_mode = None
            self.linear_inference_mode = "all"
            self._refresh_snap()
            return
        # Picking a drawing tool always leaves camera-navigation mode — and
        # any camera drag it was in the middle of.
        self.nav_mode = None
        self._last_pos = None
        self._orbit_pivot = None
        self._pan_mode = False
        self._look_drag = None
        self.unsetCursor()
        if self.active_tool is not None:
            self.active_tool.on_deactivate(self)
        self.active_tool = tool
        self.linear_inference_mode = "all"   # the toggle lasts one operation
        # …and so does the arrow-key axis lock (issue #30). It was the one
        # sticky state this reset forgot, so a lock taken while drawing a
        # line was still on — label and all — after switching to Rectangle,
        # quietly constraining a tool that never asked for it.
        self.axis_lock = None
        self._hover_entity = None  # stale highlight from the previous tool
        self.inference.clear_references()
        if tool is not None:
            tool.on_activate(self)
        self._apply_tool_cursor()
        self.measurementChanged.emit(self._measurement_text())
        self.update()

    def _resume_tool(self, tool: Tool) -> None:
        """Back from a camera mode to the tool it suspended, as it was: the
        first point placed, the rubber band, the locks. Only the camera
        drag ends."""
        self.nav_mode = None
        self._last_pos = None
        self._orbit_pivot = None
        self._pan_mode = False
        self._look_drag = None
        self.active_tool = tool
        resume = getattr(tool, "on_resume", None)
        if callable(resume):
            resume(self)
        self._apply_tool_cursor()
        self._refresh_snap()
        self.measurementChanged.emit(self._measurement_text())
        self.update()

    def begin_group_edit(self, group) -> None:
        """Enter a group for editing (double-click-into-group). A
        component's tools work in the frame of the selected occurrence.

        A locked group (the Browser's padlock), or one inside a locked
        group, does not open — whichever road asks: a double-click, the
        right-click's Edit Group or a part clicked in the Parts tab."""
        from core.outliner import path_to
        path = path_to(self.scene.groups, group) or []
        if any(getattr(g, "locked", False) or self.scene._layer_state(g)[1]
               for g in [*path, group]):
            self.flash_status(tr("Unlock it first"), 3000)
            return
        # A linked document (core/links.py) is edited in its own file: going
        # into it offers to open that file instead.
        from core import links
        linked = links.linked_ancestor(group) or next((p for p in reversed(path) if links.is_linked(p)), None)
        if linked is not None:
            self._offer_link_source(linked)
            return
        parent = self.scene.edit_group
        in_definition = (parent is not None
                         and group in (getattr(parent, "children", None) or ())
                         and bool(getattr(parent, "component", True)))
        if (getattr(group, "xform", None) is not None
                and not getattr(group, "component", True)
                and not in_definition
                and self._mesh_shared_elsewhere(group)):
            # A copied GROUP shares its geometry with the other copies only
            # until it is edited: opening it makes it its own, one undo
            # step, instead of editing every copy like a component
            # (issue #90 — the classic behaviour of groups).
            from core.history import MakeUniqueCommand
            self.history.execute(MakeUniqueCommand(group))
        was_instance = (getattr(group, "xform", None) is not None
                        and (bool(getattr(group, "component", True))
                             or not getattr(group, "children", None)))
        before = _context_stamp(self)
        self.scene.begin_group_edit(group)
        _ctx_changed(self, before)
        self._edit_undo_mark = len(self.history.undo_stack)
        # Per level too: leaving folds only what ran since THIS level opened.
        if self.scene._edit_stack:
            self.scene._edit_stack[-1]["undo_mark"] = self._edit_undo_mark
        self._hover_entity = None
        self._edges_version = -1     # the rest may fade out or leave the VBOs
        share = self.scene._edit_stack[-1].get("share") if self.scene._edit_stack else None
        if was_instance and share is not None:
            copies = sum(1 for g in self.scene.groups if g.mesh is share[1]) + 1
            self.flash_status(tr(
                "Editing component '{name}' — the change reaches its {n} "
                "copies when you leave (Esc); Make Unique first to edit "
                "one copy only", name=group.name, n=copies), 6000)
        else:
            self.flash_status(tr(
                "Editing {path} — Esc or click outside goes up one level",
                path=self.edit_path_text()), 5000)
        self.update()

    def _mesh_shared_elsewhere(self, group) -> bool:
        """Whether another placement — top level or nested at any depth —
        draws ``group``'s mesh. A group exploded out of a component copy
        shares its mesh with the group still inside the other copies
        (issue #97), and the top-level scan alone did not see it."""
        from core.purge import iter_groups
        mesh = group.mesh
        return any(g is not group and g.mesh is mesh for g in iter_groups(self.scene.groups))

    def edit_path_text(self) -> str:
        """Where you are, as a path: ``Plaza ▸ Jardinera ▸ Banca``.

        Nesting without this reads as a broken program: you click outside,
        the program correctly steps up ONE level into a parent that is also
        a container, everything is still faded, and nothing seems to have
        happened («hago clic en ese grupo que me sale atenuado y sigue así»,
        Marco, 2026-09-11 — he was two levels deep and had no way to know).
        """
        from core.outliner import open_path
        return " ▸ ".join(group.name for group in open_path(self.scene))

    def end_one_group_edit(self) -> None:
        """Step out ONE level — Esc, which leaves you inside the
        parent when the group you were editing lived in another group."""
        self._leave_group_edit(todos=False)

    def set_document(self, scene, history) -> tuple:
        """Show another document: ``scene`` with its own undo ``history``
        (for an extension's workspace, ``ExtensionApp.enter_workspace``).
        Returns the ``(scene, history)`` pair it replaces, untouched, to be
        handed back later — the parked document keeps its undo steps and its
        selection.

        A document boundary like New / Open: the id()-keyed chunk caches
        are reset (:meth:`reset_document_caches`), and since the other
        render caches remember the scene VERSION they were built for, not
        which scene it was, the incoming scene is moved past every version
        this viewport has shown before it is drawn."""
        self.end_group_edit()
        old = (self.scene, self.history)
        self.inference.reset()
        self.reset_document_caches()
        seen = max(getattr(self, "_versions_seen", 0), self.scene.version, scene.version)
        scene.version = self._versions_seen = seen + 1
        # A view preference, not document content: it follows the viewport.
        scene.hide_similar_components = bool(
            getattr(self.scene, "hide_similar_components", False))
        self.scene = scene
        self.history = history
        self._bind_bake_store()        # the incoming document's shared bakes
        from core import units as _units
        _units.bind_scene(scene)
        self._edges_version = -1
        self._hover_entity = None
        self._hover_edge = None
        self.last_snap = None
        self.reference_edge = None
        self.reference_mode = None
        self.notify_scene_changed()
        return old

    def end_group_edit(self) -> None:
        """Leave every open group, back to the model. What the menus and the
        save/export paths call."""
        self._leave_group_edit(todos=True)

    def _leave_group_edit(self, todos: bool) -> None:
        if self.scene.edit_group is None:
            return
        # Level by level: a part shares back before the assembly around it
        # takes its matrix back up out of the parts, and each level folds
        # only what ran since it opened.
        while todos and len(self.scene._edit_stack) > 1:
            self._leave_group_edit(todos=False)
        level = self.scene._edit_stack[-1] if self.scene._edit_stack else {}
        share = level.pop("share", None)
        closing_group = self.scene.edit_group
        updated_component = None
        before = _context_stamp(self)
        if todos:
            self.scene.end_group_edit()
        else:
            self.scene.end_one_group_edit()
        if share is None and "session" in level:
            from core.history import OccurrenceCommands
            session = level["session"]
            if (closing_group.is_component()
                    and OccurrenceCommands.group_start(self.history, session) < len(self.history.undo_stack)):
                updated_component = closing_group, closing_group.mesh
            OccurrenceCommands.fold(self.history, session)
        _ctx_changed(self, before)
        self._hover_entity = None
        self._edges_version = -1     # the rest comes back
        if share is not None:
            from core.reference_edit import record_leave
            group, proto = share[:2]
            shared = record_leave(self.history, level, share)
            if shared:
                updated_component = group, proto
        if updated_component is not None:
            group, proto = updated_component
            copies = sum(1 for g, _m in self.scene.placements() if g.mesh is proto)
            self.flash_status(tr(
                "Component '{name}' updated on its {n} copies",
                name=group.name, n=copies), 4000)
        elif share is not None:
            self.flash_status(tr("Left the group"), 2000)
        elif self.scene.edit_group is not None:
            # Stepping up into another container: say so, or it reads as a
            # click that did nothing (everything around is still faded).
            self.flash_status(tr(
                "Up one level — now in {path}",
                path=self.edit_path_text()), 4000)
        else:
            self.flash_status(tr("Left the group"), 2000)
        # Leaving re-shares a component: the trays (Components' «In the
        # model» list, Entity Info) must hear it now, not at the next edit —
        # the list read «no components yet» with the house on screen
        # (Rafael, revision 4, 06:18).
        self.notify_scene_changed()
