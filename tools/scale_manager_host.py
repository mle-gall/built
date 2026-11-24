# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the scale manager tool gestures."""
from tools.rust_tool_base import RustTool


class ScaleManagerTool(RustTool):
    kind = "scale_manager"
    shortcut = description = None
    _target_group = None

    @property
    def target(self):
        return self._target_group

    @target.setter
    def target(self, group):
        self._target_group = group
        self._machine.set("target", group.uid if group is not None else None)

    def host_scale_editor(self, vp, uid):
        from PySide6.QtCore import QTimer
        from core.i18n import tr
        group, self._target_group = self._target_group, None
        if uid is None:
            vp.flash_status(tr("Select one group or component first"))
        else:
            QTimer.singleShot(0, lambda: self._run(vp, group))

    def on_deactivate(self, vp):
        self._target_group = None
        super().on_deactivate(vp)

    @staticmethod
    def _run(vp, group):
        from views.scale_dialog import ask_scale
        window = vp.window()
        def closed():
            vp.update()
            window._activate_tool("select")
        ask_scale(window, group, done=closed)
