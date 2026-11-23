# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Watch the files a window's documents link (core/links.py) and reload
them when they change: saved from another tab, another window, or edited
elsewhere. Automatic, as Maxence asked (2026-11-07): the linked part cannot
be edited inside a host, so a reload never overwrites anything of the
host's; a note in the status bar says it happened."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import QFileSystemWatcher, QObject, QTimer

from core.i18n import tr


class LinkWatcher(QObject):
    #: Saves arrive as several writes (and an atomic rename): wait for the
    #: file to settle before reading it.
    SETTLE_MS = 400

    def __init__(self, window) -> None:
        super().__init__(window)
        self._window = window
        self._fs = QFileSystemWatcher(self)
        self._fs.fileChanged.connect(self._changed)
        self._pending: set = set()
        self._timer = QTimer(self, singleShot=True, interval=self.SETTLE_MS,
                             timeout=self._flush)

    def _hosts(self):
        """``(document, scene, host path)`` for every open document."""
        w = self._window
        docs = w.documents() if hasattr(w, "documents") else []
        for doc in docs:
            vp = getattr(doc, "viewport", None)
            scene = getattr(vp, "scene", None)
            if scene is not None:
                yield doc, scene, getattr(doc, "path", None)

    def refresh(self) -> None:
        """Watch exactly the sources the open documents link."""
        from core import links
        want = set()
        for _doc, scene, _host in self._hosts():
            want.update(p for p in links.sources(scene)
                        if not links.is_packed(scene, p))
        have = set(self._fs.files())
        gone = [p for p in have if p not in want]
        if gone:
            self._fs.removePaths(gone)
        new = [p for p in want if p not in have and Path(p).is_file()]
        if new:
            self._fs.addPaths(new)

    def _changed(self, path: str) -> None:
        self._pending.add(path)
        self._timer.start()

    def _flush(self) -> None:
        from core import links
        paths, self._pending = self._pending, set()
        reloaded, failed = [], []
        # On removal or failed reload the scene version need not advance.
        for doc, scene, _ in self._hosts():
            view = getattr(doc, "stand_in", None)
            if view is not None and paths.intersection(links.sources(scene)):
                view._ui_rules_cache = view._ui_tree_sig = view._ui_insp_sig = None
                view.update()
        for path in paths:
            if not Path(path).is_file():
                continue                # mid-save or removed: next change
            for doc, scene, host in self._hosts():
                if not links.linked_groups(scene, path) \
                        or links.is_packed(scene, path):
                    continue
                try:
                    links.reload(scene, path, host)
                except links.LinkError as exc:
                    failed.append(str(exc))
                    continue
                vp = getattr(doc, "viewport", None)
                if vp is not None:
                    notify = getattr(vp, "notify_scene_changed", None)
                    if callable(notify):
                        notify()
                    vp.update()
            reloaded.append(Path(path).name)
        # An atomic save replaces the file: the watch on the old one ends
        # with it, so every path is watched again.
        self.refresh()
        bar = getattr(self._window, "statusBar", None)
        if callable(bar) and (reloaded or failed):
            text = (tr("Reloaded {names}", names=", ".join(sorted(set(
                reloaded)))) if reloaded else "")
            if failed:
                text = (text + "  ·  " if text else "") + failed[0]
            bar().showMessage(text, 5000)
        browser = getattr(self._window, "project_browser", None)
        if browser is not None and hasattr(browser, "links_panel"):
            browser.links_panel.refresh()
        # The Layers rows follow through their usual check: updated in
        # place when the reload kept the same tags and folders, rebuilt only
        # when it added or dropped some. A forced rebuild here tore the
        # whole tree down on every save of a source (hundreds of linked
        # folders in a composite).
        panel = getattr(self._window, "layers_dock", None)
        changed = getattr(getattr(panel, "panel", None), "on_scene_changed", None)
        if callable(changed):
            changed()
