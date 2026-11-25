# SPDX-License-Identifier: GPL-3.0-or-later
"""A cold start without a file opens nothing by itself.

The start page (ingetrazo_render.wgpu_start) offers the last document in its
Continue card; nothing reopens it, or any other recent file, behind the
page. Crash recovery is the one thing a cold start brings back unasked,
and only after asking: the untitled auto-save left by a session that died.

"""
from __future__ import annotations

import json

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def cold_start(host, tmp_path, monkeypatch, page=False, recover=False, untitled_slot=False):
    from ingetrazo_core import autosave
    from core.scene import Scene
    from ingetrazo_core.settings import settings
    from formats import igz
    from ingetrazo_render import wgpu_start
    from ingetrazo_render import shell_files as files

    work = tmp_path
    slots = work / "autosave"
    slots.mkdir()
    monkeypatch.setattr(autosave, "autosave_dir", lambda: slots)
    monkeypatch.setenv("BUILT_START", "1" if page else "0")
    last, other = work / "last.igz", work / "other.igz"
    st = settings()
    st.setValue("home/recent_files", json.dumps([{"path": str(p), "pinned": False} for p in (last, other)]))
    st.setValue("recent_files", [str(last), str(other)])
    st.setValue("home/last_file", str(last))
    igz.save_scene(Scene(), work / "last.igz")
    igz.save_scene(Scene(), work / "other.igz")
    # The last document has a newer auto-save too: still not reopened.
    igz.save_scene(Scene(), work / "autosave" / "last-00000000.built")
    if untitled_slot:
        from core.group import Group
        from core.mesh import Mesh
        from ingetrazo_core import Vec3 as QVector3D
        scene = Scene()
        mesh = Mesh()
        mesh.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(1, 1, 0)])
        scene.groups.append(Group(mesh, name="unsaved work"))
        igz.save_scene(scene, work / "autosave" / "untitled.built")
    opened, asked = [], []
    real_open = files.open_path

    def open_path(win, path):
        opened.append(path)
        return real_open(win, path)

    def question(win, title, *args, **kwargs):
        asked.append(title)
        return "yes" if recover else "no"

    monkeypatch.setattr(files, "open_path", open_path)
    monkeypatch.setattr("ingetrazo_render.chrome_dialogs.ask", question)
    host.cold_start()
    assert not host.errors
    docs = [dict(path=d.path, names=[g.name for g in d.viewport.scene.groups])
            for d in host.window.documents()]
    return dict(opened=opened, asked=asked, docs=docs, page=wgpu_start.shown(host.window),
                slots=[p.name for p in slots.iterdir()])


@pytest.mark.parametrize("page", [False, True], ids=["model", "start-page"])
def test_a_cold_start_without_a_file_opens_nothing(native_host, tmp_path, monkeypatch, page):
    r = cold_start(native_host, tmp_path, monkeypatch, page=page)
    assert r["opened"] == []
    assert r["asked"] == []
    assert [d["path"] for d in r["docs"]] == [None]      # the blank untitled only
    assert r["page"] == page


def test_a_cold_start_still_offers_the_crash_recovery(native_host, tmp_path, monkeypatch):
    r = cold_start(native_host, tmp_path, monkeypatch, page=True, recover=True, untitled_slot=True)
    assert r["asked"] == ["Recover the auto-saved drawing?"]
    assert r["opened"] == []                             # and opens nothing else
    assert len(r["docs"]) == 1 and "unsaved work" in r["docs"][0]["names"]   # the work is back
    assert "untitled.built" in r["slots"]                # its slot kept until a save


def test_declining_the_recovery_retires_the_slot(native_host, tmp_path, monkeypatch):
    r = cold_start(native_host, tmp_path, monkeypatch, page=True, recover=False, untitled_slot=True)
    assert r["asked"] == ["Recover the auto-saved drawing?"]
    assert r["opened"] == []
    assert len(r["docs"]) == 1 and "unsaved work" not in r["docs"][0]["names"]
    assert "untitled.built" not in r["slots"]            # moved to descartados/
