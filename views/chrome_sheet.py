# SPDX-License-Identifier: GPL-3.0-or-later
"""The large sheet: one surface for every place where you browse, pick
or set something bigger than a menu — the Library, the start page,
Settings; later the one-off Export, the Extension Manager, About.

``rust/ingetrazo_render/src/ui/large_sheet.rs`` draws it in the Metal
frame (the Qt app and the native ``built`` shell
alike): a sheet hung under the top bar over the dimmed, blurred model, with
a header (title, tabs, verbs, menus, search, close), a side list, a content
pane, an optional detail pane and an optional footer. The pattern and its
rules are in ``docs/ui-patterns.md``.

A surface is a **provider**: it builds the sheet from its own state each
time something changed, and answers what the sheet reports::

    class Fruit(chrome_sheet.Provider):
        id = "fruit"

        def build(self, ctx):
            s = chrome_sheet.Sheet(self.id, "Fruit")
            s.search("Search fruit")
            s.side_item("all", "All", right="3", on=True)
            c = s.content("All fruit")
            c.tiles([chrome_sheet.item("apple", "Apple", hex="#c33")])
            d = s.detail()
            d.heading("Apple")
            d.buttons([chrome_sheet.verb("eat", "Eat it", primary=True)], align="fill")
            return s

        def act(self, ctx, verb, args):
            if verb == "verb" and args[0] == "eat":
                ...

    chrome_sheet.open(win, Fruit())

What comes back (``act(ctx, verb, args)``; ``close`` is handled here):

====================  ===================================================
``search``            [text]: the search field (typing anywhere fills it)
``side`` / ``fold``   [id]: a side row, its chevron (Tab walks the rows)
``tab``               [id]: a header tab
``verb``              [id]: a header verb, footer link or buttons entry
``focus`` / ``open``  [item]: clicked or arrowed to / double-click, Enter
``menu``              [item, entry]: a right-click menu entry
``pick``              [item]: a row of the detail pane
``headmenu``          [menu, option]: a header menu (sort…)
====================  ===================================================

A ``fields`` block holds a :class:`views.chrome_forms.Form`: its edits
apply through the form's ``on_change`` (Settings applies at once), its
inline buttons through ``on_button``; nothing else to wire.

Input: while a sheet is up it owns the pointer under the top bar and every
key but the window's ⌘ shortcuts (⌘K, ⌘O, ⌘W, ⌘Q…). Esc closes the
innermost thing (a menu, a field, the search text), then the sheet; the
model's view gets the keyboard back.

The host can install a provider offscreen with :func:`open`. Interactive
hosts release an undrawn sheet and report the missing frame.
"""
from __future__ import annotations

import json
import sys

SEP = "\x1f"
#: Asked for and not drawn this long after: close the sheet.
WATCHDOG_MS = 2000


# ---- Building a sheet ------------------------------------------------------------
def item(id, label, *, sub="", right="", icon="", hex="", badge="", dot="", dot_text="", depth=0,
         on=False, dim=False, fixed=False, tip="", menu=(), kind="", fold=None) -> dict:
    """A tile, a list row or a side row. ``icon``: a texture key sent with
    :meth:`Sheet.icon` (or any key the chrome holds); ``hex``: a colour
    when there is no picture; ``dot``: ok | warn | error | accent, with
    ``dot_text`` after it; ``menu``: [(entry id, label)] for a right click
    ("-" is a rule); ``fixed``: drawn but never focused."""
    d = {"id": str(id), "label": str(label)}
    for k, v in (("sub", sub), ("right", right), ("icon", icon), ("hex", hex), ("badge", badge),
                 ("dot", dot), ("dot_text", dot_text), ("tip", tip), ("kind", kind)):
        if v:
            d[k] = str(v)
    if depth:
        d["depth"] = int(depth)
    if on:
        d["on"] = True
    if dim:
        d["dim"] = True
    if fixed:
        d["fixed"] = True
    if menu:
        d["menu"] = [[str(a), str(b)] for a, b in menu]
    if fold is not None:
        d["fold"] = 1 if fold else 0
    return d


def verb(id, label, *, key="", primary=False, quiet=False, enabled=True, tip="") -> dict:
    """A button: ``primary`` is the accent one, ``quiet`` a text button."""
    return {"id": str(id), "label": str(label), "key": key, "role": 1 if primary else 3 if quiet else 0,
            "enabled": bool(enabled), "tip": tip}


class Pane:
    """The content or detail pane: blocks, top to bottom."""

    def __init__(self, title="", sub="", empty="", width=0.0):
        self.title, self.sub, self.empty, self.width = title, sub, empty, float(width)
        self.blocks: list = []
        self.forms: list = []

    def _add(self, kind, **kw):
        b = {"kind": kind}
        b.update({k: v for k, v in kw.items() if v not in (None, "", [], 0, 0.0)})
        self.blocks.append(b)
        return self

    def caption(self, text, right=""):
        return self._add("caption", text=text, right=right)

    def text(self, text, tone=""):
        """A paragraph; ``tone``: dim (default), faint, warn, error, ok."""
        return self._add("text", text=text, tone=tone)

    def separator(self):
        return self._add("separator")

    def heading(self, text, *, sub="", right="", star=False, icon="", hex=""):
        """The detail's name line: a swatch or picture, the name (★ when
        ``star``), quiet words on the right, a line under it."""
        return self._add("heading", text=text, sub=sub, right=right, tone="accent" if star else "",
                         icon=icon, hex=hex)

    def picture(self, icon="", hex="", size=0):
        return self._add("picture", icon=icon, hex=hex, size=float(size))

    def kv(self, rows):
        """(label, value) or (label, value, tone) rows."""
        return self._add("kv", rows=[[str(r[0]), str(r[1]), str(r[2]) if len(r) > 2 else ""] for r in rows])

    def buttons(self, buttons, align="right"):
        """``verb()`` dicts; align right (default), left, or fill (stacked)."""
        return self._add("buttons", buttons=list(buttons), align=align)

    def tiles(self, items, size=96, id=""):
        return self._add("tiles", items=list(items), size=float(size), id=id)

    def rows(self, items, id="", thumbs=False):
        return self._add("rows", items=list(items), id=id, tone="thumbs" if thumbs else "")

    def fields(self, form, id=""):
        """A :class:`views.chrome_forms.Form`'s fields, drawn as in a form
        (label column, checks, choices, sliders, colours, paths, buttons).
        Field ids must be unique in the sheet."""
        rows = form.wire()[4]
        self.forms.append(form)
        return self._add("fields", fields=[list(r) for r in rows], id=id or f"f{len(self.forms)}")

    def wire(self) -> dict:
        return {"title": self.title, "sub": self.sub, "empty": self.empty, "width": self.width,
                "blocks": self.blocks}


class Sheet:
    """What one sheet shows, built fresh by its provider."""

    def __init__(self, sid, title="", *, closable=True, width=0.0):
        self.id = str(sid)
        self.title = title
        self.closable = closable
        self.width = float(width)
        self.tabs: list = []
        self.verbs: list = []
        self.menus: list = []
        self.side: list = []
        self.side_width = 0.0
        self._search = None
        self._content = Pane()
        self._detail = None
        self._footer = None
        self.focused = None
        self.force_focus = False
        self.icons: dict = {}

    # header
    def tab(self, id, label, count="", on=False, enabled=True):
        self.tabs.append({"id": str(id), "label": label, "count": str(count or ""), "on": bool(on),
                          "enabled": bool(enabled)})

    def verb(self, id, label, **kw):
        self.verbs.append(verb(id, label, **kw))

    def search(self, placeholder, key="⌘F"):
        self._search = {"placeholder": placeholder, "key": key, "text": "", "force": False}

    def menu(self, id, label, options, on="", left=False):
        """A header menu: [(option id, label)], the checked one ``on``;
        ``left``: among the verbs (a way to make something), else beside
        the search (a way to look: sort)."""
        self.menus.append({"id": str(id), "label": label, "on": str(on), "left": bool(left),
                           "options": [verb(o, lab) for o, lab in options]})

    # side
    def side_caption(self, label):
        self.side.append({"id": "", "label": label, "kind": "caption"})

    def side_item(self, id, label, **kw):
        self.side.append(item(id, label, **kw))

    # panes
    def content(self, title="", sub="", empty="") -> Pane:
        self._content = Pane(title, sub, empty)
        return self._content

    def detail(self, width=340, title="", empty="") -> Pane:
        self._detail = Pane(title, "", empty, width)
        return self._detail

    def footer(self, links=(), hint=""):
        self._footer = {"links": [verb(i, lab) for i, lab in links], "hint": hint}

    def focus(self, item_id, force=False):
        """The focused item: taken when the sheet opens, or now with ``force``."""
        self.focused = item_id
        self.force_focus = force

    def icon(self, key, w, h, rgba) -> str:
        """A picture this sheet needs (sent once per renderer)."""
        self.icons[key] = (key, int(w), int(h), rgba)
        return key

    def forms(self) -> list:
        return self._content.forms + (self._detail.forms if self._detail is not None else [])

    def wire(self, search_text=None, search_force=False) -> dict:
        out = {"id": self.id, "title": self.title, "closable": self.closable, "width": self.width,
               "tabs": self.tabs, "verbs": self.verbs, "menus": self.menus, "side": self.side,
               "side_width": self.side_width, "content": self._content.wire(),
               "detail": self._detail.wire() if self._detail is not None else None,
               "footer": self._footer, "focus": self.focused, "force_focus": self.force_focus}
        if self._search is not None:
            s = dict(self._search)
            if search_text is not None:
                s["text"], s["force"] = search_text, search_force
            out["search"] = s
        return out


# ---- Providers ----------------------------------------------------------------------
class Provider:
    """One surface on the sheet. Override :meth:`build` and :meth:`act`."""
    id = ""
    #: Close the sheet if no frame draws it in time.
    watchdog = True

    def build(self, ctx) -> Sheet:
        raise NotImplementedError

    def act(self, ctx, verb, args) -> None:
        pass

    def signature(self, ctx):
        """A value that changes when what the sheet shows changes outside
        its own actions (the model, a worker's news). Checked each frame."""
        return None

    def closed(self, ctx) -> None:
        """The sheet went: Esc, ×, :func:`close`, or another sheet opened
        over it (``ctx.replaced``)."""

class Ctx:
    """What a provider reads: the window and the view in front."""

    def __init__(self, win, view):
        self.win, self.view = win, view
        self.replaced = False

    def refresh(self) -> None:
        refresh(self.win)


# ---- On the window --------------------------------------------------------------------
class _State:
    __slots__ = ("provider", "dirty", "sent", "sig", "built", "icons_sent", "renderer",
                 "search", "force_search", "focus", "drawn", "asked", "dog", "closing")

    def __init__(self, provider):
        self.provider = provider
        self.dirty = True
        self.sent = None
        self.sig = None
        self.built = None
        self.icons_sent: set = set()
        self.renderer = None
        self.search = None
        self.force_search = False
        self.focus = None
        self.drawn = False
        self.asked = 0.0
        self.dog = None
        self.closing = False


def _view(win):
    doc = getattr(win, "_doc", None)
    return getattr(doc, "stand_in", None) if doc is not None else None


def _state(win):
    return win.__dict__.get("_chrome_sheet") if win is not None else None


def in_shell() -> bool:
    return "built_shell" in sys.builtin_module_names


def shown(win) -> bool:
    st = _state(win)
    return st is not None and not st.closing


def current(win):
    """The provider on the sheet, or None."""
    st = _state(win)
    return st.provider if st is not None and not st.closing else None


def open(win, provider, *, search=None, focus=None):  # noqa: A001
    """Install the sheet model for the next frame, including offscreen hosts."""
    old = _state(win)
    if old is not None and old.provider is provider and not old.closing:
        if search is not None:
            old.search, old.force_search = search, True
        if focus is not None:
            old.focus = focus
        old.dirty = True
        _repaint(win)
        return True
    if old is not None and not old.closing:
        _closed(win, old, replaced=True)
    st = _State(provider)
    st.search = search
    st.force_search = search is not None
    st.focus = focus
    import time
    st.asked = time.monotonic()
    win._chrome_sheet = st
    _arm_watchdog(win, st)
    _repaint(win)
    return True


def close(win) -> None:
    """Take the sheet down (animated); the model's view gets the keys back."""
    st = _state(win)
    if st is None or st.closing:
        return
    _closed(win, st)
    st.closing = True
    st.dirty = True
    _repaint(win)
    view = _view(win)
    if view is not None and not in_shell():
        from PySide6.QtCore import QTimer

        def focus():
            try:
                view.setFocus()
            except RuntimeError:
                pass
        QTimer.singleShot(0, focus)


def _closed(win, st, replaced=False) -> None:
    ctx = Ctx(win, _view(win))
    ctx.replaced = replaced         # another sheet takes the window (it may come back)
    try:
        st.provider.closed(ctx)
    except Exception:  # noqa: BLE001 — a provider's tidy-up never keeps the sheet up
        import traceback
        traceback.print_exc()


def refresh(win) -> None:
    """Build the sheet again before the next frame."""
    st = _state(win)
    if st is not None:
        st.dirty = True
        _repaint(win)


def _repaint(win) -> None:
    view = _view(win)
    if view is not None:
        try:
            view.update()
        except RuntimeError:
            pass


def _arm_watchdog(win, st) -> None:
    if in_shell() or not getattr(st.provider, "watchdog", True):
        return
    from PySide6.QtCore import QTimer
    from PySide6.QtGui import QGuiApplication
    if QGuiApplication.platformName() in ("offscreen", "minimal"):
        return

    def check():
        if _state(win) is not st or st.drawn or st.closing:
            return
        win.__dict__.pop("_chrome_sheet", None)
        _closed(win, st)
        win.viewport.flash_status(f"Built could not draw the {st.provider.id} sheet.")
    QTimer.singleShot(WATCHDOG_MS, check)


# ---- Frames ------------------------------------------------------------------------
def before_frame(view, win) -> None:
    """Send the sheet when it changed (cheap when it did not)."""
    st = _state(win)
    renderer = getattr(view, "renderer", None)
    if st is None or renderer is None or not hasattr(renderer, "ui_set_large_sheet"):
        return
    if st.renderer is not renderer:
        st.renderer = renderer
        st.icons_sent = set()
        st.sent = None
    if st.closing:
        renderer.ui_set_large_sheet("")
        if win.__dict__.get("_chrome_sheet") is st:
            del win._chrome_sheet
        return
    ctx = Ctx(win, view)
    try:
        sig = st.provider.signature(ctx)
    except Exception:  # noqa: BLE001
        sig = None
    if sig != st.sig:
        st.sig = sig
        st.dirty = True
    st.drawn = True
    if not st.dirty and st.sent is not None:
        return
    st.dirty = False
    try:
        sheet = st.provider.build(ctx)
    except Exception:  # noqa: BLE001 — a broken provider closes its sheet, not the window
        import traceback
        traceback.print_exc()
        close(win)
        return
    if st.focus is not None and sheet.focused is None:
        sheet.focus(st.focus, force=True)
    st.focus = None
    st.built = sheet
    fresh = [i for k, i in sheet.icons.items() if k not in st.icons_sent]
    if fresh:
        renderer.ui_set_icons(fresh)
        st.icons_sent.update(i[0] for i in fresh)
    wire = sheet.wire(st.search if st.force_search else None, st.force_search)
    st.force_search = False
    text = json.dumps(wire, ensure_ascii=False, separators=(",", ":"))
    if text != st.sent:
        try:
            renderer.ui_set_large_sheet(text)
        except ValueError as exc:
            # The chrome could not read it: say why, and try again next frame.
            print(f"[chrome_sheet] {sheet.id}: {exc}", file=sys.stderr, flush=True)
            st.dirty = True
            return
        st.sent = text


def check_wire(text: str) -> None:
    """Raise ValueError when the chrome would refuse ``text`` (tests; the
    schema is ui/large_sheet.rs ``Spec``). Quiet without the native module."""
    if not text:
        return
    try:
        from ingetrazo_render import large_sheet_check
    except ImportError:
        return
    err = large_sheet_check(text)
    if err:
        raise ValueError(f"the chrome cannot read this sheet: {err}")


def _form_of(st, fid):
    sheet = st.built
    if sheet is None:
        return None
    return next((f for f in sheet.forms() if f.field(fid) is not None), None)


def action(win, arg: str) -> None:
    """``lsheet:<sheet id>\\x1f<verb>[\\x1f<arg>…]`` from the chrome."""
    sid, _, rest = arg.partition(SEP)
    st = _state(win)
    if st is None or st.closing or st.built is None or st.built.id != sid:
        return                          # a click for a sheet already gone
    parts = rest.split(SEP)
    verb, args = parts[0], parts[1:]
    if verb == "close":
        close(win)
        return
    if verb in ("v", "b") and args:
        form = _form_of(st, args[0])
        if form is not None:
            if verb == "v":
                form.receive(args[0], args[1] if len(args) > 1 else "")
            else:
                form.press(args[0])
        st.dirty = True
        _repaint(win)
        return
    if verb == "search":
        st.search = args[0] if args else ""
    try:
        st.provider.act(Ctx(win, _view(win)), verb, args)
    except (KeyError, ValueError) as exc:
        vp = getattr(win, "viewport", None)
        if vp is not None and hasattr(vp, "flash_status"):
            vp.flash_status(str(exc).strip("'\""))
    st.dirty = True
    _repaint(win)


def search_text(win) -> str:
    st = _state(win)
    return (st.search or "") if st is not None else ""


# ---- Keys (called from views/wgpu_ui.py) ---------------------------------------
def _window_key(event) -> bool:
    """⌘/⌃ combinations stay the window's (⌘K, ⌘O, ⌘W, ⌘Q…), except the
    ones a text field uses and ⌘F (the sheet's search)."""
    from PySide6.QtCore import Qt
    if not event.modifiers() & (Qt.ControlModifier | Qt.MetaModifier):
        return False
    return event.key() not in (Qt.Key_F, Qt.Key_A, Qt.Key_C, Qt.Key_V, Qt.Key_X, Qt.Key_Z)


def claims_key(view, event):
    """True: the sheet's key; False: the window's; None: no sheet up."""
    from views import wgpu_ui
    win = wgpu_ui._window(view)
    if not shown(win):
        return None
    return not _window_key(event)
