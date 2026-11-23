# SPDX-License-Identifier: GPL-3.0-or-later
"""Host bindings for the update worker, its activity notice and release sheet."""
from concurrent.futures import ThreadPoolExecutor
import sys

from PySide6.QtCore import QSettings, QTimer
from PySide6.QtWidgets import QApplication

from core import desktop_open, update_check
from core.i18n import tr
from core.version import __version__
from views import chrome_dialogs, chrome_sheet
from views.report_sheets import NativeSheet

_RETRY_MS = 5000


class ReleaseSheet(chrome_sheet.Provider):
    id = "update"

    def __init__(self, model):
        self.model = model

    def build(self, ctx):
        return NativeSheet(self.id, spec=self.model.sheet(tr))

    def act(self, ctx, verb, args):
        action = self.model.action(args[0]) if verb == "verb" and args else None
        if action:
            effect, value, close = action
            if effect == "skip":
                update_check.skip_version(QSettings(), value)
            elif effect == "open":
                desktop_open.open_url(value)
            if close:
                chrome_sheet.close(ctx.win)


def start_check(window, quiet=False, fetch=None, current=__version__):
    from ingetrazo_render import ReleaseReview
    settings = QSettings()
    installed = getattr(sys, "frozen", False) or (chrome_sheet.in_shell() and hasattr(sys, "_MEIPASS"))
    if quiet and not update_check.should_check_at_start(settings, frozen=installed):
        return
    pool = ThreadPoolExecutor(max_workers=1)
    result = pool.submit(fetch or update_check.fetch_latest)
    pool.shutdown(wait=False)
    window._release_check = result

    def present(model):
        if window._release_check is not result:
            return
        busy = chrome_dialogs.modal(window) or QApplication.activeModalWidget() is not None
        mode = model.presentation(busy)
        if mode == "wait":
            QTimer.singleShot(_RETRY_MS, window, lambda: present(model))
        elif mode == "review":
            chrome_sheet.open(window, ReleaseSheet(model))
        elif mode != "silent":
            title, text, tone, label = model.notice(tr)
            def review():
                if window._release_check is result and not update_check.is_skipped(settings, model.version):
                    chrome_sheet.open(window, ReleaseSheet(model))
            chrome_dialogs.notify(window, title, text, tone=tone, action_label=label,
                                  action=review if label else None)

    def receive():
        if window._release_check is not result:
            return
        if not result.done():
            QTimer.singleShot(100, window, receive)
            return
        try:
            release = result.result()
        except update_check.UpdateCheckError:
            present(ReleaseReview(current, "", "", "", "", quiet, False, False))
            return
        update_check.mark_checked(settings)
        present(ReleaseReview(current, release.version, release.date, release.notes, release.url,
                              quiet, update_check.is_newer(release.version, current),
                              update_check.is_skipped(settings, release.version)))

    QTimer.singleShot(0, window, receive)
