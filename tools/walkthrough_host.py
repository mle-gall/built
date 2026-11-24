# SPDX-License-Identifier: GPL-3.0-or-later
"""Walkthrough bindings for the viewport camera, preferences, timers and painter."""

from ingetrazo_core import register_tool_adapter, register_tool_functions
import os

from tools.overlay_painter import draw_overlay
from core.input_types import QPointF, Qt
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


@register_tool_adapter
class EyeHost:
    _READ_DURING = ()
    _viewport = None
    _timer = None


@register_tool_adapter
class PositionCameraHost(EyeHost):
    DRAG_PX = 6.0


@register_tool_adapter
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

    overlay_kind = "walk"
    draw_overlay = draw_overlay


@register_tool_adapter
class FirstPersonHost(EyeHost):
    TICK_MS = 16
    WALK_SPEED = WalkHost.WALK_SPEED
    RUN_FACTOR = WalkHost.RUN_FACTOR
    CLEARANCE = WalkHost.CLEARANCE
    STEP_UP = WalkHost.STEP_UP
    KNEE = WalkHost.KNEE


register_tool_functions(globals())
