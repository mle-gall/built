# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the annotation tool gestures."""

from ingetrazo_core import register_tool_adapter
from tools.rust_tool_base import RustTool
from ingetrazo_core import Vec3
from tools.rust_tool_base import _point_property, _tuple


from tools.base import AxisMagnet


@register_tool_adapter
class _AnnotationTool(AxisMagnet, RustTool):
    work_plane = None
    _ctx = _vp = None


@register_tool_adapter
class TextTool(_AnnotationTool):
    kind = "text"
    anchor = start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    def host_text_prompt(self, vp, args):
        from core.i18n import tr
        from core.protocol.commands import run
        from ingetrazo_render import chrome_forms
        self.work_plane = None
        def place(text):
            if text and text.strip():
                run(vp.history, "add_text_label", **{**args, "text": text.strip()})
            vp.update()
        chrome_forms.ask_text(vp.window(), tr("Text"), tr("Label text:"), args["text"], done=place, multiline=True)


@register_tool_adapter
class DimensionTool(_AnnotationTool):
    kind = "dimension"
    a = start_point = _point_property("a")
    b = _point_property("b")
    hover_point = _point_property("hover_point")

    _free = property(fset=lambda self, value: self._machine.set("free", (_tuple(value[0]), value[1]) if value else None))
