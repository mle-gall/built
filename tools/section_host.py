# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the section tool gestures."""
from tools.rust_tool_base import RustTool
from tools.rust_tool_base import _point_property, _mods


class SectionPlaneTool(RustTool):
    kind = "section"
    shortcut = None
    hover_point = property(lambda self: self._point("hover_point"))
    _normal = _point_property("normal")
    _axis_pick = property(lambda self: self._state("axis"))

    @property
    def wireframe_color(self):  # type: ignore[override]
        """The floating plane wears the inference colours (@pacaeiro, #62):
        red/green/blue square to an axis, magenta on any other plane."""
        from tools.base import PlaneLock
        axis = PlaneLock.plane_color(self._current_normal())
        if axis is not None:
            return axis
        from core.snap import COLOR_REFERENCE
        return (*COLOR_REFERENCE, 1.0)

    def _current_normal(self):
        return self._point("current")

    def _send(self, vp, event):
        from core import axes
        if event["type"] in ("hover", "click", "key"):
            super()._send(vp, {"type": "frame", "axes": axes.frame()})
        return super()._send(vp, event)

    def on_key(self, vp, key, modifiers):
        from PySide6.QtCore import Qt
        arrow = {Qt.Key_Right: "ArrowRight", Qt.Key_Left: "ArrowLeft", Qt.Key_Up: "ArrowUp", Qt.Key_Down: "ArrowDown"}.get(key)
        if not arrow:
            return False
        self._send(vp, {"type": "key", "key": arrow, "mods": _mods(modifiers)})
        vp.update()
        return True

    def on_click(self, ctx):
        from core.i18n import tr
        vp = ctx.viewport
        if not self._send(vp, self._pointer("click", ctx))["committed"]:
            return
        plane = vp.scene.section_planes[-1]
        window = vp.window() if hasattr(vp, "window") else None
        if not (vp.scene.show_section_cuts and vp.scene.show_section_planes):
            vp.scene.show_section_cuts = vp.scene.show_section_planes = True
            if hasattr(window, "_sync_section_menu"):
                window._sync_section_menu()
        if hasattr(window, "prompt_section_name"):
            window.prompt_section_name(plane)
        vp.flash_status(tr("Section plane placed — double-click toggles the cut; Move/Rotate reposition it"), 4000)
        vp.update()
        if hasattr(window, "_activate_tool"):
            window._activate_tool("select")
