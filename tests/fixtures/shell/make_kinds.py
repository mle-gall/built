"""A synthetic model with one of each thing the viewport's right-click menu
tells apart (docs/context-menu.md), for tests and scripted captures (never
a real project):

- loose geometry: the slab and cube of ``slab-and-cube.igz``;
- ``Box A``, ``Box B``: groups;
- ``Door``: a component;
- a placement of ``parts.built`` (a linked file holding the group
  ``Stringer``), next to them;
- a text, a dimension and a guide line.

    python tests/fixtures/shell/make_kinds.py OUT_DIR
"""
from __future__ import annotations

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _box(history, x, y, size=1.0, h=1.0):
    """Six faces of a box at (x, y); returns them."""
    from PySide6.QtGui import QVector3D as V
    from core.history import AddFaceCommand
    mesh = history.scene.mesh
    a, b = (x, y), (x + size, y + size)
    loops = [
        [V(a[0], a[1], 0), V(a[0], b[1], 0), V(b[0], b[1], 0), V(b[0], a[1], 0)],
        [V(a[0], a[1], h), V(b[0], a[1], h), V(b[0], b[1], h), V(a[0], b[1], h)],
        [V(a[0], a[1], 0), V(b[0], a[1], 0), V(b[0], a[1], h), V(a[0], a[1], h)],
        [V(b[0], a[1], 0), V(b[0], b[1], 0), V(b[0], b[1], h), V(b[0], a[1], h)],
        [V(b[0], b[1], 0), V(a[0], b[1], 0), V(a[0], b[1], h), V(b[0], b[1], h)],
        [V(a[0], b[1], 0), V(a[0], a[1], 0), V(a[0], a[1], h), V(a[0], b[1], h)],
    ]
    for loop in loops:
        history.execute(AddFaceCommand(loop))

    def inside(f):
        c = f.centroid()
        return a[0] - 1e-6 <= c.x() <= b[0] + 1e-6 and a[1] - 1e-6 <= c.y() <= b[1] + 1e-6
    # By place, not identity: faces may be new objects after each command.
    return [f for f in history.scene.mesh.faces if inside(f)]


def _group(history, faces, name, component=False):
    from core.history import GroupToComponentCommand, MakeGroupCommand
    scene = history.scene
    edges = {e for f in faces for e in getattr(f, "edges", ())}
    before = list(scene.groups)
    history.execute(MakeGroupCommand(faces, [e for e in edges if e in scene.mesh.edges]))
    g = next(g for g in scene.groups if g not in before)
    g.name = name
    if component:
        history.execute(GroupToComponentCommand(g, name))
    return g


def make(out: Path) -> dict:
    """Write ``parts.built`` and ``kinds.built`` under ``out``; their paths."""
    from PySide6.QtGui import QMatrix4x4, QVector3D as V
    from core import links
    from core.dimension import Dimension
    from core.guide import Guide
    from core.history import AddDimensionCommand, AddGuideCommand, AddTextLabelCommand, History
    from core.scene import Scene
    from core.textlabel import TextLabel
    from formats import igz
    out.mkdir(parents=True, exist_ok=True)
    leaf = out / "parts.built"
    scene = Scene()
    igz.load_into(scene, HERE / "slab-and-cube.igz")
    history = History(scene)
    _group(history, _box(history, 0.5, 4.0, 0.3, 2.5), "Stringer")
    scene.version += 1
    igz.save_scene(scene, leaf)

    path = out / "kinds.built"
    scene = Scene()
    igz.load_into(scene, HERE / "slab-and-cube.igz")
    history = History(scene)
    _group(history, _box(history, 6.0, 0.0), "Box A")
    _group(history, _box(history, 8.0, 0.0), "Box B")
    _group(history, _box(history, 10.0, 0.0, 0.9, 2.1), "Door", component=True)
    link = links.make_link(scene, leaf, host_path=path)
    m = QMatrix4x4()
    m.translate(0.0, 6.0, 0.0)
    link.xform = m
    scene.groups.append(link)
    history.execute(AddTextLabelCommand(TextLabel(V(2, 1.5, 0), V(0.5, 0.5, 1.5), "Slab edge")))
    history.execute(AddDimensionCommand(Dimension(V(0, 0, 0), V(4, 0, 0), V(0, -1, 0))))
    history.execute(AddGuideCommand(Guide(V(0, -2, 0), V(1, 0, 0))))
    scene.version += 1
    igz.save_scene(scene, path)
    return {"parts": leaf, "kinds": path}


if __name__ == "__main__":
    sys.path.insert(0, str(HERE.parents[2]))
    import os
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtGui import QGuiApplication
    app = QGuiApplication.instance() or QGuiApplication([])
    paths = make(Path(sys.argv[1]))
    print({k: str(v) for k, v in paths.items()})
