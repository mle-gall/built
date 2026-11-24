# SPDX-License-Identifier: GPL-3.0-or-later
"""Walkthrough bindings for the viewport camera, preferences, timers and painter."""
import os

from tools.overlay_painter import draw_overlay
from PySide6.QtCore import QPointF, Qt, QTimer
from ingetrazo_core import Vec3, ToolMachine

from core.i18n import tr
from core.units import fmt_len
from tools.rust_tool_base import _Host, _mods, _tuple

#: The classic default eye height, 5' 6".
DEFAULT_EYE_HEIGHT = 1.68
_SETTINGS_KEY = "walk/eye_height"

#: Kill-switch: set INGETRAZO_NO_WALK_REACH=1 to send the wall feelers
#: unbounded, as before (every placement down the street baked).
_NO_WALK_REACH = os.environ.get("INGETRAZO_NO_WALK_REACH", "") == "1"


def eye_height() -> float:
    """The eye height the three tools share, remembered across sessions."""
    try:
        from core.settings import settings as QSettings
        v = float(QSettings().value(_SETTINGS_KEY, DEFAULT_EYE_HEIGHT))
        return v if 0.05 <= v <= 100.0 else DEFAULT_EYE_HEIGHT
    except Exception:  # noqa: BLE001 — no settings store (tests, scripts)
        return DEFAULT_EYE_HEIGHT


def set_eye_height(value: float) -> None:
    try:
        from core.settings import settings as QSettings
        QSettings().setValue(_SETTINGS_KEY, float(value))
    except Exception:  # noqa: BLE001
        pass


#: First Person's mouse-look sensitivity, 1..100 (Preferences ▸ General).
#: 25 = 0.05° of head turn per pixel of mouse travel.
DEFAULT_LOOK_SENSITIVITY = 25
_LOOK_KEY = "walk/look_sensitivity"


def look_sensitivity() -> int:
    """The mouse-look sensitivity, 1..100, remembered across sessions."""
    try:
        from core.settings import settings as QSettings
        v = int(float(QSettings().value(_LOOK_KEY, DEFAULT_LOOK_SENSITIVITY)))
        return v if 1 <= v <= 100 else DEFAULT_LOOK_SENSITIVITY
    except Exception:  # noqa: BLE001 — no settings store (tests, scripts)
        return DEFAULT_LOOK_SENSITIVITY


def look_deg_per_px(sensitivity: int | None = None) -> float:
    """Degrees of head turn per pixel for a sensitivity (the stored one
    when ``None``): 1 → 0.002°, 25 → 0.05°, 100 → 0.2°."""
    from tools import walkthrough
    s = walkthrough.look_sensitivity() if sensitivity is None else sensitivity
    return s / 500.0


def _blocked(viewport, eye, delta, h, clearance, knee):
    """Compatibility entry point for the Rust walker's wall query."""
    return ToolMachine.walk_blocked(_Host(viewport, tool=EyeHost()), _tuple(eye),
                                    _tuple(delta), h, clearance, knee)


class EyeHost:
    _READ_DURING = ()
    _viewport = None
    _timer = None

    def on_activate(self, viewport):
        self._viewport = viewport
        super().on_activate(viewport)

    def on_deactivate(self, viewport):
        super().on_deactivate(viewport)
        self._viewport = None

    def on_value(self, viewport, value):
        if not isinstance(value, (int, float)):
            return False
        accepted = super().on_value(viewport, value)
        if accepted:
            viewport.update()
        return accepted

    def on_release(self, viewport):
        self._handoff = None
        self._send(viewport, {"type": "release"})
        if self._handoff:
            # Activation can synchronously deactivate this machine.
            window = getattr(viewport, "window", lambda: None)()
            activate = getattr(window, "_activate_tool", None)
            if activate:
                activate(self._handoff)
        viewport.update()

    def drag_plane(self, viewport):
        return Vec3(viewport.camera.target), -viewport.camera.forward()

    def value_label(self):
        if self._viewport is not None:
            return fmt_len(self._viewport.camera.eye().z()), None

    def host_set_camera(self, viewport, args):
        from core.agent_inspection import set_camera
        set_camera(viewport.camera, **args)

    def host_eye_height(self, viewport, _):
        from tools import walkthrough
        return walkthrough.eye_height()

    def host_ray_distance(self, viewport, args):
        ray = getattr(viewport, "ray_distance", None)
        if ray is None:
            return None
        at, direction, reach = args
        if reach is None or _NO_WALK_REACH:
            return ray(Vec3(*at), Vec3(*direction))
        return ray(Vec3(*at), Vec3(*direction), reach)

    def host_timer(self, viewport, active):
        if not active:
            if self._timer is not None:
                self._timer.stop()
            return
        if self._timer is None:
            self._timer = QTimer(getattr(viewport, "view", viewport))
            self._timer.setInterval(self.TICK_MS)
            self._timer.timeout.connect(self._tick)
        if not self._timer.isActive():
            self._timer.start()

    def _tick(self):
        from views import input_state
        vp = self._viewport
        if vp is None:
            return
        reply = self._send(vp, {"type": "camera_tick", "dt": self.TICK_MS / 1000,
                                "mods": _mods(input_state.modifiers(vp))})
        if reply["redraw"]:
            if hasattr(vp, "measurementChanged") and hasattr(vp, "_measurement_text"):
                vp.measurementChanged.emit(vp._measurement_text())
            vp.update()


class PositionCameraHost(EyeHost):
    DRAG_PX = 6.0

    def drag_plane(self, viewport):
        return None

    def value_label(self):
        return fmt_len(self.host_eye_height(None, None)), None

    def place(self, viewport, base, look_at=None):
        self._send(viewport, {"type": "place_camera", "base": _tuple(base), "look": _tuple(look_at)})
        viewport.update()

    def host_set_eye_height(self, viewport, value):
        from tools import walkthrough
        walkthrough.set_eye_height(value)

    def host_eye_placed(self, viewport, height):
        flash = getattr(viewport, "flash_status", None)
        if flash is not None:
            flash(tr("Eye placed {h} above the point — drag to look around, Walk to move",
                     h=fmt_len(height)), 4000)

    def host_handoff(self, viewport, key):
        self._handoff = key


class WalkHost(EyeHost):
    TICK_MS = 33
    FULL_PX = 150.
    CLEARANCE = 0.35
    STEP_UP = 0.5
    KNEE = 0.55
    WALK_SPEED = 1.5
    RUN_FACTOR = 3.
    TURN_DEG_PER_S = 60.
    DEAD_PX = 4.

    def step(self, viewport, dx, dy, dt, modifiers=Qt.NoModifier):
        self._send(viewport, {"type": "walk_step", "delta": (dx, dy), "dt": dt, "mods": _mods(modifiers)})

    @property
    def walking(self):
        return self._state("anchor") is not None

    @property
    def _anchor_px(self):
        p = self._state("anchor")
        return None if p is None else QPointF(*p)

    @_anchor_px.setter
    def _anchor_px(self, p):
        self._machine.set("anchor", None if p is None else (p.x(), p.y()))

    overlay_kind = "walk"
    draw_overlay = draw_overlay


class FirstPersonHost(EyeHost):
    TICK_MS = 16
    WALK_SPEED = WalkHost.WALK_SPEED
    RUN_FACTOR = WalkHost.RUN_FACTOR
    CLEARANCE = WalkHost.CLEARANCE
    STEP_UP = WalkHost.STEP_UP
    KNEE = WalkHost.KNEE

    @property
    def held(self):
        return set(self._state("held") or ())

    @property
    def moving(self):
        return bool(self.held)

    def claims_key(self, key, modifiers):
        return not modifiers & Qt.ControlModifier and self._machine.query("claims_key", int(key))

    def on_key(self, viewport, key, modifiers):
        return self._send(viewport, {"type": "key", "key": str(int(key)), "mods": _mods(modifiers)})["consumed"]

    def on_key_release(self, viewport, key):
        return self._send(viewport, {"type": "key_release", "key": str(int(key))})["consumed"]

    def on_focus_out(self, viewport):
        self.on_cancel(viewport)

    def step(self, viewport, keys, dt, modifiers=Qt.NoModifier):
        self._send(viewport, {"type": "first_person_step", "keys": list(map(int, keys)),
                              "dt": dt, "mods": _mods(modifiers)})

    def on_look(self, viewport, dx, dy):
        self._send(viewport, {"type": "look_delta", "delta": (dx, dy)})
        viewport.update()

    def host_look_rate(self, viewport, _):
        from tools import walkthrough
        return walkthrough.look_deg_per_px()

    def context_menu(self, viewport, pos):
        return True
