"""Host bindings for the Rust About, Extensions and Cut list view models."""
from __future__ import annotations

import csv
import io
import json

from views import chrome_sheet


class NativeSheet(chrome_sheet.Sheet):
    """Use a Spec already assembled by the renderer crate."""
    def __init__(self, sid, snapshot=None, *, spec=None):
        from ingetrazo_render import surface_spec
        super().__init__(sid)
        self.spec = json.loads(spec if spec is not None else surface_spec(sid, json.dumps(snapshot)))
        self.focused = self.spec.get("focus")

    def wire(self, search_text=None, search_force=False):
        if search_force and "search" in self.spec:
            self.spec["search"].update(text=search_text or "", force=True)
        return self.spec


def _copy(text):
    if chrome_sheet.in_shell():
        from app_shell import native
        return native.pasteboard_write([("public.utf8-plain-text", text.encode())])
    from PySide6.QtWidgets import QApplication
    QApplication.clipboard().setText(text)
    return True


def about_review(version, gl_line="", host=None, renderer=None):
    import os
    import platform
    import sys
    from ingetrazo_render import AboutReview, build_commit
    from core._native import engine
    from core.i18n import current_language

    system = platform.system()
    pretty = ""
    if system == "Linux":
        try:
            pretty = platform.freedesktop_os_release().get("PRETTY_NAME", "")
        except OSError:
            pass
    size = None
    qt = pyside = ""
    if host is not None:
        size = [host.width, host.height, host.scale]
    elif not chrome_sheet.in_shell():
        import PySide6
        from PySide6.QtCore import qVersion
        from PySide6.QtGui import QGuiApplication
        qt, pyside = qVersion(), PySide6.__version__
        if screen := QGuiApplication.primaryScreen():
            size = [screen.size().width(), screen.size().height(), screen.devicePixelRatio()]
    return AboutReview(json.dumps(dict(
        version=version, flatpak=bool(os.environ.get("FLATPAK_ID")), appimage=bool(os.environ.get("APPIMAGE")),
        packaged=bool(getattr(sys, "frozen", False) or (chrome_sheet.in_shell() and hasattr(sys, "_MEIPASS"))),
        system=system, release=platform.release(), machine=platform.machine(), pretty=pretty,
        os_version=platform.mac_ver()[0] if system == "Darwin" else platform.version(),
        python=platform.python_version(), qt=qt, pyside=pyside, gl=gl_line,
        renderer=getattr(renderer, "backend_label", "wgpu"), size=size, native=host is not None, language=current_language(), engine=engine(), commit=build_commit)))


class About(chrome_sheet.Provider):
    id = "about"

    def __init__(self):
        self.review = None

    def build(self, ctx):
        from core.version import __version__
        from core.i18n import tr
        if self.review is None:
            self.review = about_review(__version__, host=ctx.win._host if chrome_sheet.in_shell() else None,
                                       renderer=getattr(getattr(ctx, "view", None), "renderer", None))
        return NativeSheet(self.id, spec=self.review.sheet(tr))

    def signature(self, ctx):
        return self.review.copied() if self.review else False

    def act(self, ctx, verb, args):
        if verb != "verb" or not args or self.review is None:
            return
        action = self.review.action(args[0])
        if action is None:
            return
        kind, value = action
        if kind == "copy":
            from core.timers import later
            if not _copy(value):
                return
            self.review.acknowledge()
            later(ctx.win, 1510, lambda: chrome_sheet.refresh(ctx.win)
                              if chrome_sheet.current(ctx.win) is self else None)
        elif kind == "open":
            import webbrowser
            webbrowser.open(value)


class CutList(chrome_sheet.Provider):
    id = "cutlist"
    match_material = True
    selected = ""

    def signature(self, ctx):
        from views.inspector_bridge import _assembly
        scene = ctx.win.viewport.scene
        return id(scene), scene.content_version, id(_assembly(scene))

    def build(self, ctx):
        from core.parts import part_rows
        from core.units import fmt_len_fine
        from views.inspector_bridge import _assembly
        scene = ctx.win.viewport.scene
        if getattr(self, "built_for", None) != self.signature(ctx):
            self.selected = ""
        self.assembly = _assembly(scene)
        self.built_for = self.signature(ctx)
        parts = [dict(uid=str(r["part"].uid), name=r["name"] or "Part", material=r["material"],
                      size=r["size"], dimensions=list(map(fmt_len_fine, r["size"])))
                 for r in part_rows(self.assembly)]
        sheet = NativeSheet(self.id, dict(assembly=getattr(self.assembly, "name", "") or "Parts",
                            revision=scene.content_version, parts=parts, selected=self.selected,
                            match_material=self.match_material))
        self.members = sheet.spec.pop("members")
        blocks = sheet.spec["content"]["blocks"]
        self.table = blocks[0]["cells"] if blocks else []
        return sheet

    def closed(self, ctx):
        if ctx.replaced or not self.selected or self.built_for != self.signature(ctx):
            return
        from core.links import linked_ancestor
        if linked_ancestor(self.assembly):
            return
        uids = self.members[int(self.selected) - 1]
        vp = ctx.win.viewport
        if vp.scene.edit_group is not self.assembly:
            vp.begin_group_edit(self.assembly)
        if vp.scene.edit_group is self.assembly:
            vp.scene.select([p for p in self.assembly.children if str(p.uid) in uids])
            vp.scene.bump_view()
            ctx.view.update()

    def act(self, ctx, verb, args):
        if verb == "pick" and args[0].isdigit() and 1 <= int(args[0]) <= len(self.members):
            self.selected = args[0]
        if verb != "verb":
            return
        if args[0] == "group":
            self.match_material = not self.match_material
            self.selected = ""
        elif args[0] == "copy":
            buffer = io.StringIO(newline="")
            csv.writer(buffer, dialect="excel-tab").writerows(self.table)
            _copy(buffer.getvalue())
        elif args[0] == "csv":
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getSaveFileName(ctx.win, "Export cut list", "Cut list.csv", "CSV (*.csv)")
            if path:
                try:
                    with open(path, "w", newline="", encoding="utf-8") as stream:
                        csv.writer(stream).writerows(self.table)
                except OSError as exc:
                    ctx.win.viewport.flash_status(str(exc))


class Extensions(chrome_sheet.Provider):
    id = "extensions"

    def __init__(self):
        from core import extension_manager as em
        from ingetrazo_render import ExtensionBrowser
        self.model = ExtensionBrowser()
        self.catalog = em.load_cached_catalog()
        self.bundled = em.bundled_extensions()
        from concurrent.futures import ThreadPoolExecutor
        from core.autosave import _app_data_base
        self.cache = _app_data_base() / "extension-screenshots"
        self.pool = ThreadPoolExecutor(max_workers=3, thread_name_prefix="catalog")
        self.refreshing = None
        self.requests = {}
        self.reload = True

    def closed(self, ctx):
        self.pool.shutdown(wait=False, cancel_futures=True)
        if timer := getattr(self, "_timer", None):
            timer.stop()
            timer.deleteLater()

    def signature(self, ctx):
        import time
        from core import extension_manager as em
        now = int(time.monotonic() * 1000)
        for icon, job in list(self.requests.items()):
            if job.done():
                del self.requests[icon]
                self.model.preview_done(icon, now, job.result() if job.exception() is None else None)
        for job in self.model.jobs(now):
            self.requests[job.icon] = self.pool.submit(job.run, em._get)
        if self.model.pending() or self.refreshing is not None:
            if host := getattr(ctx.win, "_host", None):
                host.wake_after(100)
            elif ctx.view is not None:
                from PySide6.QtCore import QTimer
                if not getattr(self, "_timer", None):
                    self._timer = QTimer(ctx.view)
                    self._timer.setSingleShot(True)
                    self._timer.timeout.connect(ctx.view.update)
                if not self._timer.isActive():
                    self._timer.start(100)
        return self.model.revision, self.refreshing is not None and self.refreshing.done()

    def build(self, ctx):
        from core.i18n import tr
        if self.refreshing is not None and self.refreshing.done():
            try:
                self.catalog = self.refreshing.result()
                self.model.notice = tr("Cached catalog") if self.catalog.from_cache else tr("Catalog refreshed")
            except Exception as exc:
                self.model.notice = tr("Could not refresh catalog: {error}", error=str(exc))
            self.refreshing = None
            self.reload = True
        if self.reload:
            self.snapshot(ctx.win)
            self.reload = False
        sheet = NativeSheet(self.id, spec=self.model.sheet(tr))
        for icon in self.model.icons():
            sheet.icon(*icon)
        return sheet

    def snapshot(self, win):
        from core import extension_manager as em
        from core.extensions import plugin_dirs, _candidates
        from core.i18n import current_language
        from core.version import __version__
        self.entries = {e.id: e for e in self.bundled}
        if self.catalog:
            self.entries.update((e.id, e) for e in self.catalog.extensions)
        states = em.extension_startup_state()
        self.hands = {em.hand_installed_ident(p): p for p in em.hand_installed_plugins()}
        self.model.load(json.dumps(dict(
            catalog={k: vars(e) for k, e in self.entries.items()},
            states=states, managed={k: em.installed_info(k) for k in states},
            hands={k: str(p) for k, p in self.hands.items()},
            shipped={k: str(p) for k, p, _ in _candidates(plugin_dirs()[0])},
            startup=getattr(win, "_extension_startup_state", states),
            fetched=self.catalog.fetched_at if self.catalog else "",
            safe=em.safe_mode(), disabled=list(em.disabled_overrides()),
            reviewed=self.catalog.reviewed if self.catalog else {},
            running=list(getattr(win, "_running_extensions", ())),
            errors=getattr(win, "_extension_load_errors", {}),
            incompatible=[k for k, e in self.entries.items() if not em.compatible_with_current(e)],
            old=[k for k, e in self.entries.items() if em.tested_version_is_old(e)],
            cache=str(self.cache), lang=current_language(), build=__version__)))

    def confirm(self, win, title, text):
        from core.i18n import tr
        from views.chrome_dialogs import ask
        return ask(win, title, text, [("cancel", tr("Cancel")), ("ok", title)],
                   cancel="cancel", default="cancel") == "ok"

    def act(self, ctx, verb, args):
        from core import extension_manager as em
        from core.desktop_open import open_url
        from core.i18n import tr
        from core.paths import user_log_dir
        if verb != "verb":
            if verb == "tab":
                self.reload = True
            if not self.model.receive(verb, args):
                return
            action = "refresh"
        else:
            action = args[0]
        try:
            if action == "refresh":
                if self.refreshing is None:
                    self.refreshing = self.pool.submit(em.refresh_catalog)
                    self.model.notice = tr("Refreshing catalog…")
                return
            if action == "safe":
                em.set_safe_mode(not em.safe_mode())
            elif action == "reset":
                if not self.confirm(ctx.win, tr("Reset overrides"), tr("Enable all extensions and leave safe mode on next launch?")):
                    return
                em.reset_overrides()
            else:
                plan = self.model.command(action, tr)
                if plan is None:
                    return
                op, ident, questions = plan
                if any(not self.confirm(ctx.win, title, msg) for title, msg in questions):
                    return
                if op in ("install", "replace"):
                    def copies(paths):
                        title, msg = self.model.copies(ident, [p.name for p in paths], tr)
                        return self.confirm(ctx.win, title, msg)
                    installer = em.install_extension if op == "install" else em.replace_hand_installed
                    installer(self.entries[ident], confirm_manual_copies=copies, catalog_ids=set(self.entries))
                elif op in ("enable", "disable"):
                    em.set_extension_enabled(ident, op == "enable")
                    em.set_override(ident, False)
                elif op in ("override_on", "override_off"):
                    em.set_override(ident, op == "override_off")
                elif op == "set_aside":
                    em.remove_hand_installed(self.hands[ident])
                elif op == "uninstall":
                    em.uninstall_extension(ident)
                elif op == "repository":
                    open_url(self.entries[ident].repository)
                    return
                elif op == "logs":
                    open_url(user_log_dir())
                    return
                elif op == "folder":
                    folder = self.hands.get(ident, em.user_plugins_dir() / ident)
                    open_url(folder if folder.is_dir() else folder.parent)
                    return
            self.reload = True
            self.model.notice = tr("Restart to apply changes")
        except (OSError, em.ExtensionManagerError) as exc:
            self.model.notice = str(exc)


def show(win, sid):
    provider = {"about": About, "extensions": Extensions, "cutlist": CutList}[sid]()
    return chrome_sheet.open(win, provider)
