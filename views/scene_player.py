# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The scene player: glides the viewport's camera to a scene, or runs
through every scene in turn (core/scene_animation.py has the maths).

- ``glide_to``: showing a scene. The camera slides there; on arrival the
  scene's layers, style, section… come in (the Scenes panel's
  ``finish_view``).
- ``play``: the run. From the scene after the one shown, one after another
  in the order of the tabs, waiting on each, round and round until
  stopped.

Anything the user does to the view stops it where it is: a click or the
wheel in the viewport, Esc, a camera moved by anything else (a standard
view, Zoom Extents, a tool), another project brought to the front. A
glide stopped short still brings its scene's layers and style: that
scene is the one marked; the camera stays where the user took it.

Under Metal the GL viewport is hidden and the window's Metal view
(``Document.stand_in``, a ``WgpuView`` whose ``source_viewport`` is the
viewport) draws its camera: each frame repaints it too, and its clicks,
wheel and Esc stop the player like the viewport's.
"""
from __future__ import annotations

from PySide6.QtCore import QEvent, QObject, Qt, QTimer, Signal

from views.scene_playback import Playback

FRAME_MS = 16


class ScenePlayer(Playback, QObject):
    """Classic window timer and input filter for the shared camera timeline."""

    activeChanged = Signal(bool)

    def __init__(self, window, clock=None):
        QObject.__init__(self, window)
        Playback.__init__(self, window, clock)
        self._timer = QTimer(self)
        self._timer.setInterval(FRAME_MS)
        self._timer.timeout.connect(self.tick)

    def _set_running(self, on):
        for widget in (self._viewport, self._display):
            if widget is not None:
                if on:
                    widget.installEventFilter(self)
                else:
                    widget.removeEventFilter(self)
        Playback._set_running(self, on)
        self.activeChanged.emit(on)

    def display_of(self, viewport):
        """The Metal view that draws ``viewport``'s camera, or None (GL)."""
        doc = getattr(self._window, "_doc", None)
        stand_in = getattr(doc, "stand_in", None) if doc is not None else None
        if (stand_in is not None
                and getattr(stand_in, "source_viewport", None) is viewport):
            return stand_in
        return None

    def shows(self, viewport) -> bool:
        """Whether ``viewport``'s camera is on screen: the GL viewport
        itself, or the Metal view drawing it."""
        if viewport.isVisible():
            return True
        display = self.display_of(viewport)
        return display is not None and display.isVisible()

    def focus_widget(self, viewport):
        """Where the keyboard goes during a glide: the widget drawn in."""
        display = self.display_of(viewport)
        if display is not None and display.isVisible():
            return display
        return viewport

    def eventFilter(self, obj, ev) -> bool:  # noqa: N802 — Qt override
        t = ev.type()
        if t in (QEvent.MouseButtonPress, QEvent.MouseButtonDblClick,
                 QEvent.Wheel):
            self.stop()
        elif t == QEvent.KeyPress and ev.key() == Qt.Key_Escape:
            self.stop()
            return True          # Esc stopped the run; the tool keeps its own
        return False
