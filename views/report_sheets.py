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


def about_review(version, gl_line="", host=None):
    import os
    import platform
    import sys
    import PySide6
    from PySide6.QtCore import qVersion
    from PySide6.QtGui import QGuiApplication
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
    if host is not None:
        size = [host.width, host.height, host.scale]
    elif screen := QGuiApplication.primaryScreen():
        size = [screen.size().width(), screen.size().height(), screen.devicePixelRatio()]
    return AboutReview(json.dumps(dict(
        version=version, flatpak=bool(os.environ.get("FLATPAK_ID")), appimage=bool(os.environ.get("APPIMAGE")),
        packaged=bool(getattr(sys, "frozen", False) or (chrome_sheet.in_shell() and hasattr(sys, "_MEIPASS"))),
        system=system, release=platform.release(), machine=platform.machine(), pretty=pretty,
        os_version=platform.mac_ver()[0] if system == "Darwin" else platform.version(),
        python=platform.python_version(), qt=qVersion(), pyside=PySide6.__version__, gl=gl_line,
        size=size, native=host is not None, language=current_language(), engine=engine(), commit=build_commit)))


class About(chrome_sheet.Provider):
    id = "about"

    def __init__(self):
        self.review = None

    def build(self, ctx):
        from core.version import __version__
        from core.i18n import tr
        if self.review is None:
            self.review = about_review(__version__, host=ctx.win._host if chrome_sheet.in_shell() else None)
        return NativeSheet(self.id, spec=self.review.sheet(tr))

    def signature(self, ctx):
        return self.review.copied() if self.review else False

    def act(self, ctx, verb, args):
        if verb != "verb" or not args or self.review is None:
            return
        if args[0] == "copy":
            from PySide6.QtCore import QTimer
            if not _copy(self.review.info):
                return
            self.review.acknowledge()
            QTimer.singleShot(1510, lambda: chrome_sheet.refresh(ctx.win)
                              if chrome_sheet.current(ctx.win) is self else None)
        elif url := self.review.link(args[0]):
            import webbrowser
            webbrowser.open(url)


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
        self.catalog = em.load_cached_catalog()
        self.bundled = em.bundled_extensions()
        self.tab, self.focus, self.query = "Installed", "", ""
        self.notice = ""
        self.refreshing = None

    def signature(self, ctx):
        return self.refreshing is not None and self.refreshing.done()

    def build(self, ctx):
        from core import extension_manager as em
        from core.extensions import plugin_dirs, _candidates
        from core.i18n import current_language
        from core.version import __version__
        if self.refreshing is not None and self.refreshing.done():
            try:
                self.catalog = self.refreshing.result()
                self.notice = "Catalog refreshed"
            except Exception as exc:
                self.notice = str(exc)
            self.refreshing = None
        catalog = {e.id: e for e in self.bundled}
        if self.catalog:
            catalog.update((e.id, e) for e in self.catalog.extensions)
        self.catalog_ids = set(catalog)
        managed = em.extension_startup_state()
        hands = {em.hand_installed_ident(p): p for p in em.hand_installed_plugins()}
        shipped = {stem for stem, _, _ in _candidates(plugin_dirs()[0])}
        overrides = em.disabled_overrides()
        loaded = getattr(ctx.win, "_running_extensions", set())
        failures = getattr(ctx.win, "_extension_load_errors", {})
        safe = em.safe_mode()
        lang = current_language()
        self.packages = {}
        rows = []
        for ident in sorted(set(catalog) | managed.keys() | hands.keys() | shipped):
            ext = catalog.get(ident)
            info = em.installed_info(ident) if ident in managed else {}
            info = info or {}
            included = ident in shipped
            installed = included or ident in hands or ident in managed
            enabled = ident not in overrides and (included or ident not in managed or em.is_extension_enabled(ident))
            version = info.get("version")
            if included:
                version = __version__
            elif not installed and ext:
                version = ext.version
            name = em.display_name(ext, lang) if ext else ident.replace("_", " ").title()
            rows.append(dict(
                id=ident, name=name, installed=installed, enabled=enabled, available=bool(ext),
                included=included, manual=ident in hands, bundled=bool(ext and ext.bundled),
                source=info.get("download") or (ext.repository if ext else ""),
                version=version or "Unspecified", running=ident in loaded,
                failure=failures.get(ident, {}).get("error", ""),
                compatible=not ext or em.compatible_with_current(ext),
                summary=em.display_summary(ext, lang) if ext else "",
                update=bool(ext and info and ext.sha256 != info.get("sha256") and not included),
            ))
            self.packages[ident] = (ext, hands.get(ident), included, bool(info), enabled)
        sheet = NativeSheet(self.id, dict(tab=self.tab, query=self.query, focus=self.focus, safe=safe,
                                          notice=self.notice, packages=rows))
        self.focus = sheet.focused or ""
        return sheet

    def confirm(self, win, title, text):
        from views.chrome_dialogs import ask
        return ask(win, title, text, [("cancel", "Cancel"), ("ok", title)], cancel="cancel",
                   default="cancel") == "ok"

    def act(self, ctx, verb, args):
        from core import extension_manager as em
        if verb == "tab":
            self.tab = args[0]
        elif verb in ("focus", "open"):
            self.focus = args[0]
        elif verb == "search":
            self.query = args[0]
        elif verb == "verb":
            action = args[0]
            try:
                if action == "refresh" and self.refreshing is None:
                    from concurrent.futures import ThreadPoolExecutor
                    pool = ThreadPoolExecutor(max_workers=1)
                    self.refreshing = pool.submit(em.refresh_catalog)
                    pool.shutdown(wait=False)
                    self.notice = "Refreshing catalog…"
                elif action == "safe":
                    em.set_safe_mode(not em.safe_mode())
                    self.notice = "Restart to apply safe mode changes"
                elif action == "reset":
                    if self.confirm(ctx.win, "Reset overrides", "Enable all extensions and leave safe mode on next launch?"):
                        em.reset_overrides()
                        self.notice = "Restart to apply changes"
                elif self.focus in self.packages:
                    self.package_action(ctx.win, action)
            except (OSError, em.ExtensionManagerError) as exc:
                self.notice = str(exc)

    def package_action(self, win, action):
        from core import extension_manager as em
        ident = self.focus
        ext, path, included, managed, enabled = self.packages[ident]
        if action == "toggle":
            if managed and not included:
                em.set_extension_enabled(ident, not enabled)
                em.set_override(ident, False)
            else:
                em.set_override(ident, enabled)
        elif action == "remove" and not included:
            if not self.confirm(win, "Remove", f"Remove {ident} from installed extensions?"):
                return
            if path:
                em.remove_hand_installed(path)
            else:
                em.uninstall_extension(ident)
        elif action == "install" and ext:
            reviewed = ext.bundled or (self.catalog and em.is_reviewed(ext, self.catalog.reviewed))
            if not reviewed and not self.confirm(win, "Install unreviewed extension",
                                                f"{ident} will run Python code on your computer. Continue?"):
                return
            def confirm_copies(paths):
                names = ", ".join(p.name for p in paths)
                return self.confirm(win, "Replace copies", f"Set aside {names} and install {ident}?")
            em.install_extension(ext, confirm_manual_copies=confirm_copies, catalog_ids=self.catalog_ids)
        else:
            return
        self.notice = "Restart to apply changes"


def show(win, sid):
    provider = {"about": About, "extensions": Extensions, "cutlist": CutList}[sid]()
    return chrome_sheet.open(win, provider)
