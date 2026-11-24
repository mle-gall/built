# SPDX-License-Identifier: GPL-3.0-or-later
"""A cold start without a file opens nothing by itself.

The start page (views/wgpu_start.py) offers the last document in its
Continue card; nothing reopens it, or any other recent file, behind the
page. Crash recovery is the one thing a cold start brings back unasked,
and only after asking: the untitled auto-save left by a session that died.

``main.main()`` builds its own QApplication, so it runs in a child process,
in a throwaway profile: QSettings, auto-save slots and $HOME (the crash log
lives under it) all in temporary folders.
"""
from __future__ import annotations

import json
import os
from scripts import isolated_launch
import sys
import textwrap
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent

SCRIPT = textwrap.dedent(r"""
    import json, os, sys
    from pathlib import Path
    repo, work, page, recover = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3] == "1", sys.argv[4] == "1"
    sys.path.insert(0, str(repo))
    os.chdir(repo)
    from PySide6.QtCore import QCoreApplication, QSettings, QTimer
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, str(work / "settings"))
    QCoreApplication.setOrganizationName("IngeTrazo")
    QCoreApplication.setApplicationName("IngeTrazo")
    st = QSettings()
    assert st.fileName().startswith(str(work)), st.fileName()
    last, other = work / "last.igz", work / "other.igz"
    st.setValue("home/recent_files", json.dumps([{"path": str(last), "pinned": False},
                                                 {"path": str(other), "pinned": False}]))
    st.setValue("recent_files", [str(last), str(other)])
    st.setValue("home/last_file", str(last))
    st.setValue("General/auto_update_check", "0")
    st.sync()
    from core import autosave
    slots = work / "autosave"
    autosave.autosave_dir = lambda: slots
    from views import wgpu_start
    from views import main_window as mw
    opened, asked = [], []
    real_open = mw.MainWindow.open_path
    def open_path(self, path, *a, **k):
        opened.append(str(path))
        return real_open(self, path, *a, **k)
    mw.MainWindow.open_path = open_path
    def question(parent, title, text, *a, **k):
        asked.append(title)
        return "yes" if recover else "no"
    mw._dialogs.ask = question
    import main as M
    M._gpu_available = lambda interactive: True  # this subprocess tests document recovery offscreen
    def report():
        from PySide6.QtWidgets import QApplication
        win = next(w for w in QApplication.topLevelWidgets() if isinstance(w, mw.MainWindow))
        docs = [dict(path=str(d.path) if d.path else None,
                     names=[getattr(g, "name", "") for g in d.viewport.scene.groups]) for d in win.documents()]
        out = dict(opened=opened, asked=asked, docs=docs,
                   page=wgpu_start.shown(win),
                   slots=sorted(p.name for p in slots.iterdir()))
        (work / "report.json").write_text(json.dumps(out))
        os._exit(0)
    init = mw.MainWindow.__init__
    def init_window(win, *a, **k):
        init(win, *a, **k)
        QTimer.singleShot(int(os.environ.get("COLD_MS", "3000")), report)
    mw.MainWindow.__init__ = init_window
    from PySide6.QtNetwork import QLocalServer, QLocalSocket
    def shared_socket(*args):
        raise AssertionError("An isolated launch accessed the interactive instance socket")
    QLocalSocket.connectToServer = shared_socket
    QLocalServer.removeServer = shared_socket
    sys.argv = ["main.py"]
    M.main()
""")


def cold_start(tmp_path, page=False, recover=False, untitled_slot=False):
    from core.scene import Scene
    from formats import igz
    work = tmp_path / "work"
    (work / "settings").mkdir(parents=True)
    (work / "autosave").mkdir()
    (work / "home").mkdir()
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
    script = tmp_path / "cold.py"
    script.write_text(SCRIPT)
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", HOME=str(work / "home"),
               XDG_DATA_HOME=str(work / "home"), COLD_MS="3000", BUILT_START="1" if page else "0")
    run = isolated_launch.run([sys.executable, str(script), str(REPO), str(work),
                          "1" if page else "0", "1" if recover else "0"],
                         cwd=str(tmp_path), home=work / "home", settings=work / "settings", env=env, capture_output=True, text=True, timeout=120)
    report = work / "report.json"
    assert report.is_file(), run.stdout[-3000:] + run.stderr[-3000:]
    return json.loads(report.read_text())


@pytest.mark.parametrize("page", [False, True], ids=["model", "start-page"])
def test_a_cold_start_without_a_file_opens_nothing(tmp_path, page):
    r = cold_start(tmp_path, page=page)
    assert r["opened"] == []
    assert r["asked"] == []
    assert [d["path"] for d in r["docs"]] == [None]      # the blank untitled only
    assert r["page"] == page


def test_a_cold_start_still_offers_the_crash_recovery(tmp_path):
    r = cold_start(tmp_path, page=True, recover=True, untitled_slot=True)
    assert r["asked"] == ["Recover the auto-saved drawing?"]
    assert r["opened"] == []                             # and opens nothing else
    assert len(r["docs"]) == 1 and "unsaved work" in r["docs"][0]["names"]   # the work is back
    assert "untitled.built" in r["slots"]                # its slot kept until a save


def test_declining_the_recovery_retires_the_slot(tmp_path):
    r = cold_start(tmp_path, page=True, recover=False, untitled_slot=True)
    assert r["asked"] == ["Recover the auto-saved drawing?"]
    assert r["opened"] == []
    assert len(r["docs"]) == 1 and "unsaved work" not in r["docs"][0]["names"]
    assert "untitled.built" not in r["slots"]            # moved to descartados/
