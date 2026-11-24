# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the solid tool gestures."""
from tools.rust_tool_base import RustTool


_SOLID_ERRORS = {
    "not a solid": "That is not a solid — a group or component closed all "
                   "round, with nothing nested inside",
    "no overlap": "The two solids do not overlap",
    "empty": "The operation leaves nothing",
    "not manifold": "That solid has a flaw the operation cannot read",
    "two solids": "Pick two solids",
}


class SolidTool(RustTool):
    kind = "solid_union"
    shortcut = None

    def __init__(self):
        super().__init__()
        self._cache = {}
        self._cache_version = None
        self._viewport = None

    @staticmethod
    def _siblings(scene):
        return scene.edit_group.children if scene.edit_group else scene.groups

    @property
    def first(self):
        key = self._state("first")
        if self._viewport and key:
            return next((g for g in self._siblings(self._viewport.scene) if g.uid == key), None)

    dragging = property(lambda self: self._state("first") is not None)
    _badge = property(lambda self: self._state("badge"))
    solid_cursor_key = property(lambda self: "solid-" + self._badge)
    op = property(lambda self: self.kind.removeprefix("solid_"))

    @property
    def qt_cursor(self):
        from views.icons import solid_cursor
        return solid_cursor(self._badge)

    def host_solid_selection(self, vp, _):
        from core.group import Group
        self._viewport = vp
        return [(g.uid, self._is_solid(vp.scene, g)) for g in self._siblings(vp.scene)
                if isinstance(g, Group) and g in vp.scene.selection]

    def _is_solid(self, scene, group):
        from core.solids import is_solid
        if self._cache_version != scene.version:
            self._cache.clear()
            self._cache_version = scene.version
        if group.uid not in self._cache:
            self._cache[group.uid] = is_solid(group)
        return self._cache[group.uid]

    def host_solid_pick(self, vp, px):
        self._viewport = vp
        g = vp.pick_group(*px)
        return (g.uid, self._is_solid(vp.scene, g)) if g is not None else None

    def host_solid_select(self, vp, uid):
        if uid is None:
            vp.scene.selection.clear()
            vp.scene.bump_view()
        else:
            vp.scene.selection = {g for g in self._siblings(vp.scene) if g.uid == uid}
            vp.scene.version += 1

    def host_solid_result(self, vp, _):
        return next((g.uid for g in self._siblings(vp.scene) if g in vp.scene.selection), None)

    def host_solid_error(self, vp, message):
        from core.i18n import tr
        vp.flash_status(tr(_SOLID_ERRORS.get(message, message or "empty")), 5000)

    def host_solid_cutter_hint(self, vp, _):
        from core.i18n import tr
        vp.flash_status(tr("Click the solid that cuts, then the one it cuts"), 4000)

    def on_activate(self, vp):
        if self._send(vp, {"type": "activate"})["committed"]:
            vp.update()
            window = vp.window() if hasattr(vp, "window") else None
            if hasattr(window, "_activate_tool"):
                window._activate_tool("select")

    def on_hover(self, ctx):
        if self._send(ctx.viewport, self._pointer("hover", ctx))["redraw"]:
            apply = getattr(ctx.viewport, "_apply_tool_cursor", None)
            if apply:
                apply()


class OuterShellTool(SolidTool):
    kind = "outer_shell"
    op = "outer_shell"
    shortcut = None


class UnionTool(SolidTool):
    kind = "solid_union"
    op = "union"
    shortcut = None


class SubtractTool(SolidTool):
    kind = "solid_subtract"
    op = "subtract"
    shortcut = None


class TrimTool(SolidTool):
    kind = "solid_trim"
    op = "trim"
    shortcut = None


class IntersectTool(SolidTool):
    kind = "solid_intersect"
    op = "intersect"
    shortcut = None


class SplitTool(SolidTool):
    kind = "solid_split"
    op = "split"
    shortcut = None


SOLID_TOOLS = (
    ("outer_shell", OuterShellTool), ("solid_union", UnionTool),
    ("solid_subtract", SubtractTool), ("solid_trim", TrimTool),
    ("solid_intersect", IntersectTool), ("solid_split", SplitTool),
)
