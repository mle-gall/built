# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Scene transitions — the camera glides from one scene to the next.

Pure maths, no Qt widgets: what the camera looks like at any instant
between two scenes, and along a whole run through every scene (glide,
pause, glide…). The viewport's player (``views/scene_player.py``) asks it
for the pose of the moment and paints it; an export of the run as a video
can ask it for the pose of every frame the same way.

Only the camera moves. The rest of what a scene recalls — layers, style,
section, shadows, hidden objects — changes once, when the camera arrives:
it is the destination's, and changing it on every frame would rebuild the
model's caches sixty times a second.

The glide, per term of the orbit camera:

- the target travels in a straight line;
- the distance changes in proportion (log scale), so coming in from far
  away does not crawl at first and then rush at the end;
- the yaw turns the short way round, never the long one;
- pitch and field of view go from one value to the other;
- it starts and stops gently (``ease``).

A parallel view stays parallel only while both ends are: a glide from or
to perspective runs in perspective, and the projection snaps where the
parallel end is — at the arrival, or at once on leaving. Parallel framing
is the perspective's at the target plane, so the snap barely shows there.
"""
from __future__ import annotations

import math
from dataclasses import dataclass

from ingetrazo_core import (Pose, SceneRun as Run, between, ease, glide, turn,
                            video_at, video_frame_count, video_seconds)

VIDEO_FPS = (24, 25, 30, 60)

#: The defaults: two seconds to glide, one to stay on each scene in a run.
DEFAULT_TRANSITION = 2.0
DEFAULT_PAUSE = 1.0
#: The longest either may be (seconds) — a typo of 2000 is not a setting.
MAX_SECONDS = 60.0


@dataclass
class SceneAnimation:
    """A document's scene-transition settings (saved in the .igz)."""

    #: Whether showing a scene glides there (off: it jumps, as before).
    enabled: bool = True
    #: Seconds the camera takes from one scene to the next.
    transition: float = DEFAULT_TRANSITION
    #: Seconds a run through every scene stays on each.
    pause: float = DEFAULT_PAUSE

    def __post_init__(self) -> None:
        self.enabled = bool(self.enabled)
        self.transition = _seconds(self.transition, DEFAULT_TRANSITION)
        self.pause = _seconds(self.pause, DEFAULT_PAUSE)

    def glide_seconds(self) -> float:
        """How long showing a scene takes: 0 = a jump."""
        return self.transition if self.enabled else 0.0

    def to_dict(self) -> dict:
        """Only what differs from the defaults (terse files)."""
        out: dict = {}
        if not self.enabled:
            out["enabled"] = False
        if self.transition != DEFAULT_TRANSITION:
            out["transition"] = self.transition
        if self.pause != DEFAULT_PAUSE:
            out["pause"] = self.pause
        return out

    @classmethod
    def from_dict(cls, raw) -> "SceneAnimation":
        raw = raw if isinstance(raw, dict) else {}
        return cls(enabled=raw.get("enabled", True),
                   transition=raw.get("transition", DEFAULT_TRANSITION),
                   pause=raw.get("pause", DEFAULT_PAUSE))


def _seconds(value, default: float) -> float:
    try:
        value = float(value)
    except (TypeError, ValueError):
        return default
    if not math.isfinite(value):
        return default
    return max(0.0, min(MAX_SECONDS, value))
