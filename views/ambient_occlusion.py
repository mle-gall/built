# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Resolve style and diagnostic settings for the wgpu ambient-occlusion pass."""
from __future__ import annotations

import os
from typing import Optional


def _env_float(name: str, default: float) -> float:
    try:
        return float(os.environ.get(name, default))
    except ValueError:
        return default


def settings_from_env() -> Optional[dict]:
    """The settings forced from the environment, or None when it is off."""
    if os.environ.get("INGETRAZO_AO", "0") in ("", "0"):
        return None
    return {"radius": max(_env_float("INGETRAZO_AO_RADIUS", 0.5), 1e-4),
            "intensity": _env_float("INGETRAZO_AO_INTENSITY", 1.0),
            "samples": int(max(4, min(64, _env_float("INGETRAZO_AO_SAMPLES",
                                                      16))))}


def settings_for(style) -> Optional[dict]:
    """What this frame draws with: the style's switch and values (Styles
    panel), unless ``INGETRAZO_AO`` is set — 1 forces it on with the
    environment's values, 0 forces it off (tests, benchmarks)."""
    forced = os.environ.get("INGETRAZO_AO")
    if forced is not None and forced != "":
        return settings_from_env()
    if style is None or not getattr(style, "ambient_occlusion", False):
        return None
    return {"radius": max(float(style.ao_distance), 1e-4),
            "intensity": float(style.ao_intensity),
            "samples": int(max(4, min(64, _env_float("INGETRAZO_AO_SAMPLES",
                                                      16))))}
