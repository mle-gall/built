# SPDX-License-Identifier: GPL-3.0-or-later
"""Move the viewport into the edit context requested by a browser row."""
from core import outliner


def open_to(viewport, path) -> bool:
    """Leave the view with exactly ``path`` open — stepping out of the
    levels that differ, then into the rest, one level at a time, as the
    view's own double-click and Esc do. False when a group on the way is
    hidden or locked (it cannot be entered). The tray's «In model» list
    opens to a nested copy the same way."""
    if any(g.hidden or getattr(g, "locked", False) for g in path):
        return False
    scene = viewport.scene
    current = outliner.open_path(scene)
    same = 0
    while (same < min(len(current), len(path))
           and current[same] is path[same]):
        same += 1
    for _ in range(len(current) - same):
        viewport.end_one_group_edit()
    for g in path[same:]:
        viewport.begin_group_edit(g)
    return outliner.open_path(scene) == list(path)
