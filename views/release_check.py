# SPDX-License-Identifier: GPL-3.0-or-later
"""Host bindings for the update worker, its activity notice and release sheet."""
import json
from concurrent.futures import Future
import threading
import sys

from core.settings import settings as QSettings
from core.timers import later

from core import desktop_open, update_check
from core.i18n import tr
from core.version import __version__
from ingetrazo_render import chrome_sheet
from ingetrazo_render import chrome_dialogs

_RETRY_MS = 5000


class ReleaseSheet(chrome_sheet.Provider):
    id = "update"

    def __init__(self, model):
        self.model = model

    def build(self, ctx):
        return chrome_sheet.Spec(json.loads(self.model.sheet(tr)))

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
    result = Future()

    def worker():
        try:
            result.set_result((fetch or update_check.fetch_latest)())
        except Exception as exc:
            result.set_exception(exc)

    threading.Thread(target=worker, daemon=True).start()
    window._release_check = result

    def present(model):
        if window._release_check is not result:
            return
        busy = chrome_dialogs.modal(window) or chrome_dialogs.async_host(window) is None
        mode = model.presentation(busy)
        if mode == "wait":
            later(window, _RETRY_MS, lambda: present(model))
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
            later(window, 100, receive)
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

    later(window, 0, receive)
