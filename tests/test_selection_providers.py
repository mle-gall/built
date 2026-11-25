# SPDX-License-Identifier: GPL-3.0-or-later
"""The chrome's selection providers key on the selection's native revision
(docs/render-glue-port.md, round 4), and the INI preference store sees a
change made by another handle or process without re-reading on every
read."""
from pathlib import Path
from types import SimpleNamespace

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

SLAB = Path(__file__).resolve().parent / "fixtures" / "shell" / "slab-and-cube.igz"


@pytest.fixture
def host(tmp_path):
    from built_render import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    yield h
    h.close()


class Bar:
    def __init__(self):
        self.sent = []

    def ui_set_selection(self, summary, verbs):
        self.sent.append(summary)


def test_the_selection_bar_follows_a_change_in_the_middle(host):
    """The bar's signature was (count, first, last): swapping a middle
    member left it as it was. The revision moves with any change."""
    from views import wgpu_ui
    assert host.open(SLAB)
    scene = host.viewport.scene
    edges = list(scene.mesh.edges)
    faces = list(scene.mesh.faces)
    assert len(edges) >= 3 and faces
    view = SimpleNamespace(scene=scene, renderer=Bar(), last=None)
    scene.selection = [edges[0], edges[1], edges[2]]
    wgpu_ui._selection(view, host.window, "select")
    wgpu_ui._selection(view, host.window, "select")
    assert view.renderer.sent == ["3 items"]
    scene.selection = [edges[0], faces[0], edges[2]]
    # Same count, same first and last: the old signature saw no change.
    members = list(scene.selection)
    assert len(members) == 3 and members[0] is edges[0] and members[-1] is edges[2]
    wgpu_ui._selection(view, host.window, "select")
    assert len(view.renderer.sent) == 2
    # Unchanged, nothing more is sent; an empty selection clears the bar.
    wgpu_ui._selection(view, host.window, "select")
    assert len(view.renderer.sent) == 2
    scene.selection = []
    wgpu_ui._selection(view, host.window, "select")
    assert view.renderer.sent[-1] == ""


def test_the_summary_counts_groups_and_items_from_the_snapshot(host):
    from core.group import Group
    from views import wgpu_ui
    assert host.open(SLAB)
    scene = host.viewport.scene
    group = next((g for g in scene.groups if isinstance(g, Group)), None)
    edge = next(iter(scene.mesh.edges))
    scene.selection = [edge] + ([group] if group is not None else [])
    snapshot = scene.selection.snapshot()
    groups = sum(1 for e in scene.selection if isinstance(e, Group))
    assert snapshot[1] == groups
    expected = " · ".join(p for p in (f"{groups} group" if groups == 1 else "",
                                      "1 item") if p)
    assert wgpu_ui._summary(snapshot) == expected


def test_preferences_written_elsewhere_are_read_fresh(tmp_path):
    """The parsed INI file is kept while its stamp (modification time and
    length) holds; another handle's write, or another process rewriting
    the file, is seen on the next read."""
    import os
    import time
    from built_core import NativeSettings
    a, b = NativeSettings(tmp_path), NativeSettings(tmp_path)
    a.setValue("tabs/hover_picture", "1")
    assert b.value("tabs/hover_picture") == "1"
    a.setValue("tabs/hover_picture", "0")
    assert b.value("tabs/hover_picture") == "0"
    ini = tmp_path / "Built" / "Built.ini"
    text = ini.read_text()
    assert 'hover_picture="0"' in text
    # Another process: same length, a later modification time.
    ini.write_text(text.replace('hover_picture="0"', 'hover_picture="1"'))
    later = time.time() + 2
    os.utime(ini, (later, later))
    assert b.value("tabs/hover_picture") == "1"
    ini.unlink()
    assert b.value("tabs/hover_picture") is None
