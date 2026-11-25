# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Ahsan Mehmood (OpenSKP) — Built plugin contribution.
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Python Console plugin — a live REPL over the open document.

The classic scripting console: inspect the model, script geometry,
prototype the next plugin — against the running application, no restart.

What makes this one a good citizen of Built (and where it differs from
the original contribution, PR #3):

- **Every execution runs as a Command** (:class:`core.history.SnapshotImport`
  via ``viewport.history.execute``): geometry a script creates is undoable
  with Ctrl+Z, marks the document dirty (so closing warns about unsaved
  work), and repaints immediately — the version bump the raw ``exec`` never
  produced. A script that raises is **rolled back whole**, not left half
  applied. Executions that only inspect (``print(len(mesh.faces))``) are
  detected and leave no undo entry behind.
- The scope's ``scene``/``mesh``/``selection``/``groups``/``layers`` are
  re-bound on every run, so they always point at the CURRENT document —
  not the one that happened to be open when the console was first shown.
- One console per window (re-activating raises it), monospace from the
  system (no hardcoded Consolas), output preserves indentation.

Undo covers what the snapshot covers: mesh mutations plus added groups,
layers, saved views, dimensions and text labels. Exotic state a script
touches beyond that (camera, styles, files on disk) is its own business.
"""
from __future__ import annotations

import io
import sys
import traceback
from pathlib import Path

from built_core.history import SnapshotImport
from built_core.i18n import tr
from built_render.shell_files import ShellFileDialog
from tools.base import Tool


class _ConsoleExecution:
    def _refresh_scope(self) -> None:
        """Re-bind the live objects: always the CURRENT document, not the one
        open when the console appeared (documents change under New/Open)."""
        from built_core import Vec3 as QVector3D
        from core import bim
        from core.group import Group
        from built_core.mesh import Edge, Face, Mesh, Vertex
        from core.scene import Scene

        vp = self._viewport
        app = getattr(vp.window(), "_host", None)
        self._scope.update(
            app=app, window=vp.window(), viewport=vp,
            document=getattr(vp.window(), "_doc", None),
            scene=vp.scene, model=vp.scene, mesh=vp.scene.mesh,
            selection=vp.scene.selection, groups=vp.scene.groups,
            layers=vp.scene.layers, bim=bim, console=self,
            QVector3D=QVector3D, Mesh=Mesh, Group=Group, Scene=Scene,
            Vertex=Vertex, Edge=Edge, Face=Face)

    def _run_code(self, code: str, label: str) -> None:
        """Execute ``code`` as one undoable command against the document.

        The SnapshotImport wrapper gives us, for free: rollback when the code
        raises (no half-built geometry), Ctrl+Z when it succeeds, the dirty
        flag, and the version bump the viewport's VBO rebuild keys on. A run
        that changed nothing is popped back off the undo stack (and the redo
        stack it cleared is restored), so inspecting never eats an undo step.
        """
        self._refresh_scope()
        vp = self._viewport
        history = vp.history
        out_buf, err_buf = io.StringIO(), io.StringIO()

        def mutate(_scene) -> None:
            old_out, old_err = sys.stdout, sys.stderr
            sys.stdout, sys.stderr = out_buf, err_buf
            try:
                # Compile-as-expression is probed OUTSIDE the try that runs
                # the code: running exec inside the `except SyntaxError`
                # handler would chain that internal, irrelevant SyntaxError
                # onto every error a multi-line script raises ("During
                # handling of the above exception...") — caught on the first
                # dogfooding screenshot.
                try:
                    compiled = compile(code, label, "eval")
                except SyntaxError:
                    compiled = None
                if compiled is None:
                    exec(compile(code, label, "exec"), self._scope)
                else:
                    result = eval(compiled, self._scope)
                    if result is not None:
                        self._scope["_"] = result
                        print(repr(result))
            except Exception:
                traceback.print_exc(file=err_buf)
                raise            # history rolls the document back whole
            finally:
                sys.stdout, sys.stderr = old_out, old_err

        cmd = SnapshotImport(mutate)
        saved_redo = list(history.redo_stack)
        history.execute(cmd)

        if history.last_error is None:
            unchanged = not cmd.changed
            if (unchanged and history.undo_stack
                    and history.undo_stack[-1] is cmd):
                history.undo_stack.pop()
                history.redo_stack[:] = saved_redo

        vp.notify_scene_changed()

        if out := out_buf.getvalue():
            self._append_text(out.rstrip(), "out")
        if err := err_buf.getvalue():
            self._append_text(err.rstrip(), "error")
            if history.last_error is not None:
                self._append_text(
                    tr("(rolled back — the document is unchanged)"),
                    "muted")


class PythonConsoleTool(Tool):
    """Extensions-menu entry that opens (or raises) the console."""
    name = "Python Console"
    shortcut = "Ctrl+Shift+P"
    menu_order = 10              # after Model Info and Solid Inspector
    description = (
        "Open a Python prompt over the open document, to inspect it "
        "or script it.")
    uses_snap = False

    def on_activate(self, viewport) -> None:
        window = viewport.window()
        from core.plugin_panels import open_panel
        if not open_panel(window, "python_console"):
            viewport.flash_status(tr("Python Console requires the native view."), 3000)

    def on_deactivate(self, viewport) -> None:
        pass


class ConsolePanel(_ConsoleExecution):
    id, title, slot = "console", "Python Console", "utility"

    def __init__(self, app, invalidate):
        self._viewport = app.viewport
        self._scope = {"__name__": "__console__"}
        self.lines = ["Python console · scene, mesh, selection and viewport are in scope."]
        self.history = []

    def _append_text(self, text, role="out"):
        self.lines.extend(text.splitlines())
        self.lines = self.lines[-400:]

    def sections(self):
        from core.plugin_panels import section, item
        return [section("session", "", item("log", "output", lines=self.lines),
                        item("console", "prompt", history=self.history),
                        item("button", "help", "API help"), item("button", "script", "Run script file…"),
                        item("button", "clear", "Clear output"))]

    def event(self, control, event, value):
        if control == "clear":
            self.lines.clear()
        elif control == "help":
            self._refresh_scope()
            self._append_text(", ".join(sorted(k for k in self._scope if not k.startswith("_"))))
        elif control == "script":
            vp = self._viewport
            path, _ = ShellFileDialog.getOpenFileName(vp.window(), "Run script", "", "Python (*.py)")
            if path and vp.window().viewport is vp:
                self._scope["__file__"] = str(Path(path).resolve())
                self._append_text("Running " + path)
                self._run_code(Path(path).read_text(encoding="utf8"), path)
        elif control == "prompt" and event == "console_submit":
            self.history.append(value)
            self.history = self.history[-200:]
            self._append_text(">>> " + value)
            self._run_code(value, "<console>")


create_panel = ConsolePanel
