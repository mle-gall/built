# SPDX-License-Identifier: GPL-3.0-or-later
"""Building the project tree of a model with many linked placements must not
write the whole document back once per placement.

A plain dict stored into a tracked table while something else still holds it
stays a *suspect* (``built_core.doc_model``): it is re-read at boundaries.
``link_of`` reads a placement's node, and asked for every suspect to be
converted and patched again first, once per placement. With a project tree's
folder table as the suspect, opening a model with thousands of linked parts
sat at "Preparing graphics buffer" for minutes. A per-node read now writes
back that node's suspects only, and a suspect is sent again only when it
differs from what was sent; the write-through semantics stay as pinned in
``test_doc_write_through.py``."""
from __future__ import annotations

from pathlib import Path

import pytest


@pytest.fixture(scope="module")
def host(tmp_path_factory):
    from tests.composite_fixture import build
    return Path(build(tmp_path_factory.mktemp("open-flush"), 600, scale=0.2)["host"])


def _tree(scene):
    import views.wgpu_ui as wu

    class Renderer:
        def __getattr__(self, name):
            return lambda *a: None

    class View:
        renderer = Renderer()

        def update(self):
            pass

    class Viewport:
        def __init__(self, sc):
            from built_core.history import History
            self.scene, self.history = sc, History(sc)

        def flash_status(self, *a):
            pass

        def update(self):
            pass

        def notify_scene_changed(self):
            pass

    class Win:
        def __init__(self, sc):
            self.viewport = Viewport(sc)

    wu._tree(View(), Win(scene))


def test_a_shared_suspect_is_not_sent_once_per_linked_placement(host):
    import built_core
    import built_core.doc_model as dm
    from built_core.scene import Scene
    from formats import igz
    scene = Scene()
    igz.load_into(scene, host)
    placements = sum(1 for g in built_core.purge_iter_groups(scene.groups))
    assert placements > 400                          # many linked parts
    big = {f"folder:{i}": {"name": f"F{i}", "recipe": {"tag": f"T{i}", "folder": []}}
           for i in range(500)}
    scene.plugin_data["probe"] = big                  # stored into a tracked table
    keep = big                                        # and held: it stays a suspect
    assert any(entry[3] is keep for entry in dm._SUSPECT.values())
    dm._flush_stats()
    _tree(scene)
    stats = dm._flush_stats()
    # Each suspect is sent once at most (the probe and the tree's own tables,
    # unchanged afterwards), however many placements the tree read; the
    # per-placement reads look at their own node only.
    assert stats["sent"] <= 20, stats
    assert stats["full"] < placements // 10, stats
    # The document has the value, and an edit made through the held dict
    # still reaches it at the next boundary.
    assert built_core.doc_attr_value(scene._root, "plugin_data")["probe"]["folder:7"]["name"] == "F7"
    keep["folder:7"]["name"] = "renamed"
    dm.flush()
    assert built_core.doc_attr_value(scene._root, "plugin_data")["probe"]["folder:7"]["name"] == "renamed"
    assert dm._flush_stats()["sent"] == 1


def test_an_unchanged_suspect_is_sent_once():
    import built_core
    import built_core.doc_model as dm
    from built_core.scene import Scene
    scene = Scene()
    held = {"a": [1, 2, {"b": 3}]}
    scene.plugin_data["x"] = held
    dm._flush_stats()
    for _ in range(50):
        dm.flush()
    assert dm._flush_stats()["sent"] == 1
    # 1 and 1.0 are different document values: the change is sent.
    held["a"][2]["b"] = 3.0
    dm.flush()
    assert dm._flush_stats()["sent"] == 1
    assert type(built_core.doc_attr_value(scene._root, "plugin_data")["x"]["a"][2]["b"]) is float
    # Put back in another place of the table, it is sent again there.
    scene.plugin_data["y"] = held
    dm.flush()
    assert built_core.doc_attr_value(scene._root, "plugin_data")["y"] == {"a": [1, 2, {"b": 3.0}]}
    del held
