# SPDX-License-Identifier: GPL-3.0-or-later
"""The large sheet's Python side (ingetrazo_render.chrome_sheet, sheet_runtime.rs): a provider's sheet
sent as JSON, actions routed back, fields applied at once, close and focus,
key ownership, and the fallback where the chrome cannot draw. The drawing,
Esc order, arrows and type-to-search are tested in Rust (ui/large_sheet.rs);
the Metal round trip in test_sheet_pattern_metal.py."""
from __future__ import annotations

import json

import pytest
from PySide6.QtCore import QEvent, Qt
from PySide6.QtGui import QKeyEvent
from PySide6.QtWidgets import QApplication

from views import chrome_forms
from ingetrazo_render import chrome_sheet as cs

_app = QApplication.instance() or QApplication([])


class Renderer:
    def __init__(self):
        self.sent = []
        self.icons = []

    def ui_set_large_sheet(self, text):
        cs.check_wire(text)       # what the chrome reads (ui/large_sheet.rs)
        self.sent.append(json.loads(text) if text else None)

    def ui_set_icons(self, icons):
        self.icons += [i[0] for i in icons]


class View:
    def __init__(self):
        self.renderer = Renderer()
        self._ui_attached = True
        self.focused = 0
        self.updates = 0

    def update(self):
        self.updates += 1

    def setFocus(self, *_):
        self.focused += 1


class Win:
    def __init__(self):
        self.view = View()
        self._doc = type("Doc", (), {"stand_in": self.view})()
        self.flashed = []
        self.viewport = type("VP", (), {"flash_status": lambda _s, t: self.flashed.append(t)})()


class Fruit(cs.Provider):
    id = "fruit"

    def __init__(self):
        self.seen = []
        self.snap = True
        self.closed_n = 0

    def build(self, ctx):
        s = cs.Sheet(self.id, "Fruit")
        s.tab("all", "All", count=3, on=True)
        s.search("Search fruit")
        s.side_caption("Kinds")
        s.side_item("pome", "Pome", right="2", on=True)
        c = s.content("Pome", "2")
        c.tiles([cs.item("apple", "Apple", hex="#cc3333", menu=[("eat", "Eat")]), cs.item("pear", "Pear")])
        form = chrome_forms.Form("", [chrome_forms.Field("snap", "check", "Snap", self.snap),
                                      chrome_forms.Field("reset", "button", "Reset")],
                                 on_change=lambda f, fid, v: setattr(self, "snap", v),
                                 on_button=lambda f, fid: self.seen.append(("button", fid)))
        c.fields(form)
        d = s.detail()
        d.heading("Apple", right="Kitchen")
        d.kv([("Colour", "red", "warn")])
        d.buttons([cs.verb("eat", "Eat it", primary=True)], align="fill")
        s.footer([("help", "Help")], "esc back")
        s.icon("fruit:apple", 1, 1, b"\xff\x00\x00\xff")
        return s

    def act(self, ctx, verb, args):
        self.seen.append((verb, *args))

    def closed(self, ctx):
        self.closed_n += 1


@pytest.fixture
def win(monkeypatch):
    w = Win()
    monkeypatch.setattr(cs, "_view", lambda _w: w.view)
    monkeypatch.setattr("views.wgpu_ui._window", lambda v: w)
    return w


def frame(win):
    cs.before_frame(win.view, win)
    return win.view.renderer.sent[-1] if win.view.renderer.sent else None


def test_a_provider_builds_the_whole_sheet_as_json(win):
    p = Fruit()
    assert cs.open(win, p)
    assert cs.shown(win) and cs.current(win) is p
    sh = frame(win)
    assert sh["id"] == "fruit" and sh["title"] == "Fruit" and sh["closable"] is True
    assert sh["tabs"] == [{"id": "all", "label": "All", "count": "3", "on": True, "enabled": True}]
    assert sh["side"][0]["kind"] == "caption" and sh["side"][1]["id"] == "pome"
    kinds = [b["kind"] for b in sh["content"]["blocks"]]
    assert kinds == ["tiles", "fields"]
    tiles = sh["content"]["blocks"][0]
    assert tiles["size"] == 96 and tiles["items"][0]["menu"] == [["eat", "Eat"]]
    fields = sh["content"]["blocks"][1]["fields"]
    assert [f[0] for f in fields] == ["snap", "reset"] and fields[0][4] == 1.0
    assert [b["kind"] for b in sh["detail"]["blocks"]] == ["heading", "kv", "buttons"]
    assert sh["footer"]["links"][0]["id"] == "help"
    assert win.view.renderer.icons == ["fruit:apple"]
    # Nothing changed: nothing sent again; the icon is never sent twice.
    n = len(win.view.renderer.sent)
    frame(win)
    assert len(win.view.renderer.sent) == n
    cs.refresh(win)
    frame(win)
    assert win.view.renderer.icons == ["fruit:apple"]


def test_actions_reach_the_provider_and_fields_apply_at_once(win):
    p = Fruit()
    cs.open(win, p)
    frame(win)
    for a in ("search\x1fapp", "focus\x1fapple", "open\x1fapple", "menu\x1fapple\x1feat",
              "side\x1fpome", "verb\x1feat", "headmenu\x1fsort\x1fname"):
        cs.action(win, "fruit\x1f" + a)
    assert p.seen == [("search", "app"), ("focus", "apple"), ("open", "apple"), ("menu", "apple", "eat"),
                      ("side", "pome"), ("verb", "eat"), ("headmenu", "sort", "name")]
    assert cs.search_text(win) == "app"
    cs.action(win, "fruit\x1fv\x1fsnap\x1f0")
    assert p.snap is False                               # applied, no OK
    assert frame(win)["content"]["blocks"][1]["fields"][0][4] == 0.0
    cs.action(win, "fruit\x1fb\x1freset")
    assert ("button", "reset") in p.seen
    # A click for another sheet (one already gone) is dropped.
    cs.action(win, "other\x1fverb\x1feat")
    assert ("verb", "eat") == p.seen[5] and p.seen.count(("verb", "eat")) == 1


def test_close_animates_out_and_gives_the_view_the_keys(win, monkeypatch):
    p = Fruit()
    cs.open(win, p)
    frame(win)
    fired = []
    monkeypatch.setattr("PySide6.QtCore.QTimer.singleShot", staticmethod(lambda ms, fn: (fired.append(ms), fn())))
    cs.action(win, "fruit\x1fclose")                     # Esc or × in the chrome
    assert not cs.shown(win) and cs.current(win) is None
    assert p.closed_n == 1
    assert win.view.focused == 1                         # focus back on the model's view
    assert frame(win) is None                            # "" sent: the chrome animates it out
    assert "_chrome_sheet" not in win.__dict__


def test_a_second_sheet_replaces_the_first(win):
    a, b = Fruit(), Fruit()
    b.id = "veg"
    cs.open(win, a)
    frame(win)
    cs.open(win, b)
    assert a.closed_n == 1 and cs.current(win) is b
    # Opening the same one again keeps it and can preset the search.
    assert cs.open(win, b, search="leek")
    sh = frame(win)
    assert sh["search"]["text"] == "leek" and sh["search"]["force"] is True
    sh = (cs.refresh(win), frame(win))[1]
    assert sh["search"]["force"] is False                # the chrome's own typing wins after


def test_a_broken_provider_closes_its_sheet_not_the_window(win):
    class Broken(cs.Provider):
        id = "broken"

        def build(self, ctx):
            raise RuntimeError("boom")
    cs.open(win, Broken())
    frame(win)
    assert not cs.shown(win)


def key(k, text="", mods=Qt.NoModifier):
    return QKeyEvent(QEvent.KeyPress, k, mods, text)


def test_it_owns_the_keys_but_the_windows_shortcuts(win):
    view = win.view
    assert cs.claims_key(view, key(Qt.Key_Escape)) is None
    cs.open(win, Fruit())
    for k, t in ((Qt.Key_Escape, ""), (Qt.Key_A, "a"), (Qt.Key_Down, ""), (Qt.Key_Tab, ""), (Qt.Key_Return, "")):
        assert cs.claims_key(view, key(k, t)) is True
    for k in (Qt.Key_F, Qt.Key_A, Qt.Key_C, Qt.Key_V):        # ⌘F search, text editing
        assert cs.claims_key(view, key(k, "", Qt.ControlModifier)) is True
    for k in (Qt.Key_K, Qt.Key_O, Qt.Key_W, Qt.Key_Q, Qt.Key_Comma):
        assert cs.claims_key(view, key(k, "", Qt.ControlModifier)) is False


def test_offscreen_open_exposes_the_sheet_model():
    win = Win()
    provider = Fruit()
    assert cs.open(win, provider)
    assert cs.shown(win) and cs.current(win) is provider


def test_missing_frame_reports_failure_and_closes_the_provider(win, monkeypatch):
    shots = []
    monkeypatch.setattr("PySide6.QtCore.QTimer.singleShot", staticmethod(lambda ms, fn: shots.append(fn)))

    monkeypatch.setattr("PySide6.QtGui.QGuiApplication.platformName", staticmethod(lambda: "cocoa"))
    p = Fruit()
    cs.open(win, p)
    shots[-1]()                                          # the watchdog, no frame drawn
    assert p.closed_n == 1 and not cs.shown(win)
    assert win.flashed == ["Built could not draw the fruit sheet."]
    q = Fruit()
    cs.open(win, q)
    frame(win)                                           # drawn: the watchdog does nothing
    shots[-1]()
    assert q.seen == [] and cs.shown(win)
