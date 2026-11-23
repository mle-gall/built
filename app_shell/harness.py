"""Scripted runs only (``built --script``, ``menu ctx_demo:<kind>``): the
right-click menu of each kind on screen, for captures and visual checks.

It selects a thing of that kind in the open model (tests/fixtures/shell/
make_kinds.py makes one of each) and opens the chrome's menu where that thing
is drawn, through the same ``views.context_menu`` the right-click uses.
Never part of a user's path."""
from __future__ import annotations


def _pick(scene, kind):
    from core import links
    groups = {g.name: g for g in scene.groups}
    faces = list(scene.mesh.faces)
    edges = list(scene.mesh.edges)
    link = next((g for g in scene.groups if links.is_linked(g)), None)
    table = {
        "faces": faces[:1], "edges": edges[:1], "geometry": faces[:1] + edges[:1],
        "group": [groups.get("Box A")], "component": [groups.get("Door")],
        "groups": [groups.get("Box A"), groups.get("Box B")],
        "mixed": [groups.get("Box A")] + faces[:1],
        "link": [link], "linked_part": [(link.children or [None])[0]] if link is not None else [],
        "annotation": list(scene.text_labels[:1]), "dimension": list(scene.dimensions[:1]),
        "guide": list(scene.guides[:1]), "empty": [],
    }
    return [e for e in table.get(kind, []) if e is not None]


def context_menu_for(win, kind) -> bool:
    from views import context_menu
    vp = win.viewport
    scene = vp.scene
    sel = _pick(scene, kind)
    scene.select(sel)
    view = win.view
    point = None
    if sel:
        lo, hi = scene.selection_bounds()
        if lo is not None:
            point = (lo + hi) * 0.5
        elif hasattr(sel[0], "position"):
            point = sel[0].position()
        elif hasattr(sel[0], "point"):
            point = sel[0].point
    px = view.tools._world_to_pixel(point) if point is not None else None
    x, y = (float(px[0]), float(px[1])) if px is not None else (view.width() * 0.45, view.height() * 0.4)
    vp._context_pixel = (x, y)
    menu = context_menu.build(win, view)
    menu.at = (x, y)
    view._ctx_menu = menu
    context_menu.send(view, menu)
    return True
