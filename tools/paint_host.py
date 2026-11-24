# SPDX-License-Identifier: GPL-3.0-or-later
"""Viewport picks and shared toolbar/tray brush state for the Rust Paint tool."""

from ingetrazo_core import register_tool_adapter, register_tool_functions
from core.i18n import tr
from core.paint_ops import paint_stamp
from core.style import DEFAULT_FACE_COLOR
from ingetrazo_core import Vec3, clicked_back_side

#: How many materials the «Recent» row keeps.
RECENT_MAX = 6


@register_tool_adapter
class PaintHost:
    _READ_DURING = ()
    # Shared current paint colour (RGB, 0..1), set from the toolbar swatch.
    current_color: tuple[float, float, float] = (0.80, 0.45, 0.30)
    # Shared current texture ({"path","sw","sh"}) or None. When set, the click
    # applies a texture instead of a colour — chosen from the toolbar.
    current_texture: dict | None = None
    # Shared current material IDENTITY (core.materials.Material) or None.
    # Set by the tray when the active swatch has a name; painting then
    # stamps attrs["mat"] alongside the colour/texture (and registers the
    # material in the scene on first use). None = anonymous paint, which
    # CLEARS any previous identity — a red face is no longer "Concreto
    # visto", so the per-material takeoff never lies.
    current_material = None
    # Shared translucency (glass): None = opaque paint, which also CLEARS
    # any previous opacity on the painted faces.
    current_opacity: float | None = None
    # Armed by the toolbar's eyedropper button: the NEXT click samples
    # instead of painting, then disarms. The pipette sits
    # beside the material — Alt works for people who know it, the button is
    # how you find it.
    sample_armed: bool = False
    # Plane the current texture's explicit ``uvw`` belongs to, as
    # ``(normal, offset)`` — set when the eyedropper samples a face that
    # carries one. Only faces on that plane inherit the map; see the module
    # docstring. ``None`` = the texture has no map of its own to hand on.
    current_texture_plane: tuple | None = None
    # The «Default» material (no material): the eyedropper picks it
    # up from an unpainted side, the tray offers it, and painting with it
    # REMOVES the paint (@pacaeiro, #47 point 2). While it is set the
    # colour/texture fields above are ignored.
    current_is_default: bool = False
    # The last materials painted with, newest first, as material dicts
    # (``_current_as_material``). The tray loads them from the settings,
    # shows them and saves them back (``recent_materials_changed``).
    recent: list[dict] = []

register_tool_functions(globals())
