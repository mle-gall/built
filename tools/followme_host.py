# SPDX-License-Identifier: GPL-3.0-or-later
"""Mesh picks, renderer objects and localized messages for Follow Me."""

from ingetrazo_core import register_tool_adapter
from ingetrazo_core import Vec3
from core.i18n import tr
from core.mesh import Edge, Face
from core.sweep import manual_path_extend, manual_path_start, order_path_edges

_MESSAGES = {
    "invalid": "Follow Me: the selected edges must form one simple path",
    "intro": "Follow Me: click the profile and drag along the path (it highlights red), click to finish — or select the path first, then click the profile",
    "drag": "Follow Me: drag along the path (it highlights red) — click to finish, Esc to cancel; hold Alt over a face to follow its perimeter",
    "touch": "Follow Me: touch the path edges to extrude along them",
}
_FAILED = "Follow Me: could not sweep along that path (reversal or degenerate segment)"


@register_tool_adapter
class FollowMeHost:
    wireframe_color = (0.85, 0.16, 0.16, 1.0)

    def __init__(self):
        super().__init__()
        self._entities = {}
