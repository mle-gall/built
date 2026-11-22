# SPDX-License-Identifier: GPL-3.0-or-later
"""A synthetic composite project for the Project tree's linked files
(ui/tree-links, 2026-11-22): a host linking A.igz and B.igz, A linking
C.igz and D.igz. Every file has its own tag folders and tags, a structure
proposed from them (as «Propose structure from tags» does) and its own
placements: A has about ``big`` of them so slowness shows.

    python -m tests.composite_fixture OUTDIR [big]   # writes the four files
"""
from __future__ import annotations

from pathlib import Path


def _file(path, folders, tags, count, links_to=()):
    """One .igz: ``folders`` {name: parent}, ``tags`` {tag: folder}, ``count``
    placements spread over the tags, ``links_to`` placed as linked files."""
    from PySide6.QtGui import QMatrix4x4, QVector3D
    from core import bim_registry, links, structure
    from core.group import Group
    from core.layers import Layer
    from core.mesh import Mesh
    from core.scene import Scene
    from formats import igz
    s = Scene()
    s.layer_folders = list(folders)
    for name, parent in folders.items():
        if parent:
            s.folder_parents[name] = parent
    for tag, folder in tags.items():
        s.layers.append(Layer(tag, folder=folder))
    names = list(tags)
    for i in range(count):
        m = Mesh()
        x, y = float(i % 200), float(i // 200)
        m.add_face([QVector3D(x, y, 0), QVector3D(x + .5, y, 0), QVector3D(x + .5, y + .5, 0),
                    QVector3D(x, y + .5, 0)])
        g = Group(m, f"{path.stem}-{i}")
        g.layer = names[i % len(names)]
        s.groups.append(g)
    for n, src in enumerate(links_to):
        comp = links.make_link(s, src, host_path=path)
        xf = QMatrix4x4()
        xf.translate(0, 300.0 * (n + 1), 0)
        comp.xform = xf
        s.groups.append(comp)
    registry = bim_registry.get(s)
    proposal = structure.bootstrap(s, registry)
    structure.apply_bootstrap(s, proposal).do(s)
    igz.save_scene(s, path)
    return path


def build(folder, big=20000):
    """Write C, D, B, A and the host into ``folder``; returns their paths."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    c = _file(folder / "C.igz", {"Stairs": None}, {"Flights": "Stairs", "Landings": "Stairs"}, 600)
    d = _file(folder / "D.igz", {"Facilities": None, "Rooms": "Facilities"},
              {"Toilets": "Rooms", "Kiosks": "Rooms", "Signs": "Facilities"}, 900)
    b = _file(folder / "B.igz", {"Building": None, "Levels": "Building"},
              {"Slabs": "Levels", "Walls": "Levels", "Site": "Building"}, 3000)
    a = _file(folder / "A.igz", {"Ride": None, "Track": "Ride", "Supports": "Ride"},
              {"Rails": "Track", "Ties": "Track", "Columns": "Supports", "Base plates": "Supports",
               "Fasteners": "Ride"}, big, links_to=(c, d))
    host = _file(folder / "Host.igz", {"Site works": None}, {"Paths": "Site works", "Fences": "Site works"},
                 50, links_to=(a, b))
    return dict(host=host, a=a, b=b, c=c, d=d)


if __name__ == "__main__":
    import sys
    from PySide6.QtWidgets import QApplication
    QApplication.instance() or QApplication([])
    out = build(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 20000)
    print("\n".join(f"{k} {v}" for k, v in out.items()))
