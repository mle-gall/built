# SPDX-License-Identifier: GPL-3.0-or-later
"""The viewport right-click menu for the chrome (docs/context-menu.md).

Which rows a selection gets, their labels, keys and checks, the header, and
what runs each verb come from the one verb source,
``rust/ingetrazo_render/src/verbs.rs`` (through ``views.verbs``).
``rust/ingetrazo_render/src/ui/context_menu.rs`` draws the menu and reports
``ctx:<id>``. This module reads the document into facts, keeps what the menu
was opened over, and runs the dispatch Rust names:

- ``slot``: the window's Edit-menu slot (``_on_reverse_faces``…), the one
  the classic QMenu, the menu bar and the inspector call;
- ``tree``: the project tree's own verb on the item row ``i:<uid>``, which
  goes through ``core.protocol``;
- ``core``: a core command on the selected placements;
- ``glue``: one of the few actions below that need a field, a dialog or the
  selection itself.

Some Qt-host callers still use ``MainWindow.show_viewport_context_menu``
(a QMenu).
"""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Optional

from views import verbs as _verbs
from ingetrazo_render import viewport_tag_menu

KINDS, READ_ONLY, EDITING, MAX_ROWS = _verbs.KINDS, _verbs.READ_ONLY, _verbs.EDITING, _verbs.MAX_ROWS


@dataclass
class Verb:
    id: str
    label: str
    key: str = ""
    enabled: bool = True
    #: -1: no check; 0 / 1: a check mark, off / on.
    checked: int = -1


@dataclass
class Row:
    verbs: list
    #: A segmented row's family name ("Intersect"); "" for a plain row.
    label: str = ""

    def text(self) -> str:
        """The row as docs/context-menu.md writes it: "Hide",
        "Explode | Make component…", "Intersect: Model | Selection | Context"."""
        verbs = " | ".join(v.label for v in self.verbs)
        return f"{self.label}: {verbs}" if self.label else verbs


@dataclass
class Menu:
    kind: str
    title: str
    subtitle: str
    rows: list = field(default_factory=list)
    #: Where it was opened (logical points, the view's coordinates).
    at: tuple = (0.0, 0.0)
    #: What it acts on: the selection and the locked things under the pointer.
    sel: list = field(default_factory=list)
    locked_group: object = None
    locked_image: object = None
    facts: dict = field(default_factory=dict)

    def verbs(self):
        return [v for r in self.rows for v in r.verbs]

    def find(self, vid):
        return next((v for v in self.verbs() if v.id == vid), None)

    def labels(self) -> list:
        """Each row as text (tests, docs): "Hide", "Intersect: Model | …"."""
        return [r.text() for r in self.rows]

    def ids(self) -> list:
        return [v.id for v in self.verbs()]


def kind_of(scene, sel) -> str:
    """The menu kind for the selection ``sel``."""
    return _verbs.kind_of(scene, sel)


def build(win, view=None, sel=None, *, locked_image=None, locked_group=None) -> Menu:
    """The menu for the current selection (the right-click has already
    selected what it hit, see :func:`open_at`)."""
    scene = win.viewport.scene
    sel = list(scene.selection if sel is None else sel)
    facts = _verbs.facts(scene, sel, win, view, locked_group=locked_group, locked_image=locked_image)
    kind, title, subtitle, rows = _verbs.verbs_menu(json.dumps(facts))
    return Menu(kind, title, subtitle,
                [Row([Verb(*v) for v in verbs], label) for label, verbs in rows],
                sel=sel, locked_group=locked_group, locked_image=locked_image, facts=facts)


# ---- running a verb --------------------------------------------------------------------
def run(win, view, kind, vid, sel, *, locked_group=None, locked_image=None, facts=None) -> bool:
    """Run verb ``vid`` of a ``kind`` selection ``sel`` as the verb source
    says (``verbs_dispatch``). False when it names nothing to run."""
    how, name, arg = _verbs.verbs_dispatch(kind, vid)
    if how == "slot":
        from core.mesh import Face
        fn = getattr(win, name, None)
        if fn is None:
            return False
        values = {"": (), "sel0": (sel[0],) if sel else None, "none": (None,),
                  "face0": next(((e,) for e in sel if isinstance(e, Face)), None),
                  "locked_group": (locked_group,), "locked_image": (locked_image,)}
        args = values.get(arg, (arg,))
        if args is None:
            return False
        fn(*args)
    elif how == "tree":
        if not sel:
            return False
        _tree(win, view, name, sel[0])
    elif how == "core":
        from views import wgpu_ui
        wgpu_ui._core(win.viewport, name, placements=_uids(sel))
        win.viewport.update()
    elif how == "glue":
        _GLUE[name](win, view, sel, arg, dict(facts or {}))
    else:
        return False
    return True


def _tree(win, view, verb, g, extra=None):
    """Run the project tree's own verb on the item row of ``g``."""
    from views import wgpu_ui
    key = f"{verb}{wgpu_ui.SEP}i:{g.uid}"
    if extra is not None:
        key += f"{wgpu_ui.SEP}{extra}"
    wgpu_ui._tree_action(win, view, key)


def _uids(sel):
    return [g.uid for g in sel if getattr(g, "uid", None)]


def _groups(sel):
    from core.group import Group
    return [e for e in sel if isinstance(e, Group)]


def _intersect(win, view, sel, mode, facts):
    from core import intersect
    win._on_intersect_faces({"model": intersect.WITH_MODEL, "selection": intersect.WITH_SELECTION,
                             "context": intersect.WITH_CONTEXT}[mode])


def _isolate(win, view, sel, arg, facts):
    from views import wgpu_ui
    wgpu_ui._isolate(win, _uids(_groups(sel)))


def _file_to(win, view, sel, arg, facts):
    from views import wgpu_ui
    wgpu_ui._file_to(win, len(_groups(sel)))


def _lock(win, view, sel, arg, facts):
    from ingetrazo_render import context_lock
    context_lock(win.viewport, _groups(sel), not facts.get("all_locked"))


def _keep(win, view, sel, arg, facts):
    win.keep_selected_components(not facts.get("kept"))


def _contents(win, view, sel, arg, facts):
    from views import wgpu_ui
    scene = win.viewport.scene
    wgpu_ui._select(win, [c for p in sel for c in getattr(p, "children", None) or ()
                          if scene.entity_selectable(c)] or list(sel))


def _host(sel):
    from core import links
    part = next(iter(_groups(sel)), None)
    if part is None:
        return None
    return part if links.is_linked(part) else links.linked_ancestor(part)


def _link(win, view, sel, verb, facts):
    from views import wgpu_ui
    target = _host(sel)
    if target is not None:
        wgpu_ui._link_action(win, view, target, verb)


def _select_link(win, view, sel, arg, facts):
    from views import wgpu_ui
    host = _host(sel)
    if host is not None:
        wgpu_ui._select(win, [host])


def _delete_guide(win, view, sel, arg, facts):
    from core.protocol import commands
    guides = list(win.viewport.scene.guides)
    commands.erase_stroke(win.viewport.history,
                          guides=[guides.index(g) for g in sel if g in guides])
    win.viewport.update()


def _show_all(win, view, sel, arg, facts):
    from views import wgpu_ui
    wgpu_ui._show_all(win.viewport)


def _section_name(win, view, sel, arg, facts):
    win.prompt_section_name(sel[0], automatic=False)


def _reveal(win, view, sel, arg, facts):
    from views import wgpu_ui
    wgpu_ui._tree_action(win, view, "revealselection" + wgpu_ui.SEP)


_GLUE = {"selectiontag": lambda win, view, sel, arg, facts: viewport_tag_menu(win, sel),
         "intersect": _intersect, "isolate": _isolate, "fileto": _file_to, "lock": _lock,
         "keep": _keep, "contents": _contents, "link": _link, "selectlink": _select_link,
         "deleteguide": _delete_guide, "showall": _show_all, "secname": _section_name,
         "reveal": _reveal,
         "rename": lambda win, view, sel, arg, facts: ask_rename(win, view, sel[0]),
         "edittext": lambda win, view, sel, arg, facts: edit_text(win, view, sel[0])}


# ---- verbs that need a field or a dialog ------------------------------------------
def ask_rename(win, view, g) -> None:
    """Rename… : the chrome's name prompt (views/chrome_forms.py), answered
    into the tree's own rename (``rename`` on the row ``i:<uid>``, core
    ``rename_placement``)."""
    from views import chrome_forms
    chrome_forms.ask_text(win, "Rename", "Name:", text=getattr(g, "name", "") or "",
                          done=lambda text: rename_answered(win, view, g, text))


def rename_answered(win, view, g, text) -> None:
    """The prompt's Rename: the tree's verb on the item's row."""
    text = (text or "").strip()
    if text and g is not None:
        _tree(win, view, "rename", g, text)


def edit_text(win, view, entity) -> None:
    """Edit text… : what a double-click on the text or dimension does (the
    Select tool's own handler), at the point the menu was opened on."""
    from core.input_types import QPointF, Qt
    from ingetrazo_core import Vec3 as QVector3D
    from tools.base import ToolContext
    from tools.rust_tools import SelectTool
    vp = win.viewport
    px = getattr(vp, "_context_pixel", None)
    if px is None:
        return
    tools = getattr(view, "tools", None) or vp
    SelectTool().on_double_click(ToolContext(
        tools, QVector3D(), QPointF(float(px[0]), float(px[1])), Qt.NoModifier, None))


# ---- opening it ------------------------------------------------------------------------
def open_at(view, x, y) -> Optional[Menu]:
    """The right-click at ``(x, y)``: select what is under the pointer if it
    is not selected yet (``Viewport._context_pick``), build the menu for the
    selection, and hand it to the chrome. Returns the menu."""
    source = view.source_viewport
    if source is None:
        return None
    win = source.window()
    source._context_pixel = (x, y)
    tools = view.tools
    picked = tools._context_pick(x, y)
    # A locked image or group under the pointer, which no click selects:
    # its Unlock lives here (as in the classic menu).
    under = tools.pick_locked_image_plane(x, y)
    locked_image = under if under is not picked else None
    locked_group = tools.pick_locked_group(x, y)
    from ingetrazo_render import context_targets
    sel = context_targets(picked is not None, source.scene.selection)
    menu = build(win, view, sel=sel, locked_image=locked_image, locked_group=locked_group)
    menu.at = (float(x), float(y))
    view._ctx_menu = menu
    send(view, menu)
    return menu


def send(view, menu) -> None:
    renderer = getattr(view, "renderer", None)
    if renderer is None or not hasattr(renderer, "ui_set_context_menu"):
        return
    rows = [(r.label, [(v.id, v.label, v.key, bool(v.enabled), int(v.checked)) for v in r.verbs])
            for r in menu.rows]
    renderer.ui_set_context_menu(float(menu.at[0]), float(menu.at[1]), menu.title, menu.subtitle, rows)
    view.update()


def action(win, view, vid) -> bool:
    """``ctx:<id>`` from the chrome: run that verb of the open menu."""
    menu = getattr(view, "_ctx_menu", None) if view is not None else None
    view._ctx_menu = None
    verb = menu.find(vid) if menu is not None else None
    if verb is None or not verb.enabled:
        return False
    if menu.kind == "texture_position":
        tool = menu.facts["tool"]
        return tool is view.tools.tool and tool.menu_action(view.tools, vid)
    if not run(win, view, menu.kind, vid, menu.sel, locked_group=menu.locked_group,
               locked_image=menu.locked_image, facts=menu.facts):
        return False
    try:
        view.tools.sync()
    except Exception:  # noqa: BLE001
        pass
    win.viewport.update()
    return True


def chrome_menu_available(view) -> bool:
    """Whether this view's chrome draws the right-click menu (the native
    shell, or the Qt app on Metal)."""
    renderer = getattr(view, "renderer", None)
    return (getattr(view, "_ui_attached", False)
            and hasattr(renderer, "ui_set_context_menu") and hasattr(renderer, "ui_over_chrome"))
