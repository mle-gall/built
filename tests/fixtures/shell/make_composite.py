"""A synthetic composite with nested links, for the native app's tests and
scripted runs (never a real project): ``leaf.built`` (the slab and cube),
``mid.built`` placing two links to it, ``host.built`` placing ``mid`` twice.

    python tests/fixtures/shell/make_composite.py OUT_DIR
"""
from __future__ import annotations

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def make(out: Path) -> dict:
    """Write the three files under ``out``; their paths by name."""
    from built_core import Matrix4 as QMatrix4x4
    from core import links
    from core.scene import Scene
    from formats import igz
    out.mkdir(parents=True, exist_ok=True)
    leaf = out / "leaf.built"
    scene = Scene()
    igz.load_into(scene, HERE / "slab-and-cube.igz")
    igz.save_scene(scene, leaf)

    def host_of(path, source, offsets):
        host = Scene()
        for dx in offsets:
            comp = links.make_link(host, source, host_path=path)
            m = QMatrix4x4()
            m.translate(dx, 0.0, 0.0)
            comp.xform = m
            host.groups.append(comp)
        host.version += 1
        igz.save_scene(host, path)

    mid = out / "mid.built"
    host_of(mid, leaf, (0.0, 6.0))
    top = out / "host.built"
    host_of(top, mid, (0.0, 14.0))
    return {"leaf": leaf, "mid": mid, "host": top}


if __name__ == "__main__":
    sys.path.insert(0, str(HERE.parents[2]))
    import os
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    paths = make(Path(sys.argv[1]))
    print({k: str(v) for k, v in paths.items()})
