# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the scale manager tool gestures."""

from ingetrazo_core import register_tool_adapter
from tools.rust_tool_base import RustTool


@register_tool_adapter
class ScaleManagerTool(RustTool):
    kind = "scale_manager"
    shortcut = description = None
    _target_group = None

    def host_scale_editor(self, vp, uid):
        from core.timers import later
        from core.i18n import tr
        group, self._target_group = self._target_group, None
        if uid is None:
            vp.flash_status(tr("Select one group or component first"))
        else:
            later(vp, 0, lambda: self._run(vp, group))

    @staticmethod
    def _run(vp, group):
        from views.scale_editor import ask_scale
        window = vp.window()
        def closed():
            vp.update()
            window._activate_tool("select")
        ask_scale(window, group, done=closed)
