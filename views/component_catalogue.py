"""Bind the Library catalogue to the cached index, thumbnail and OBJ services."""
import json
from concurrent.futures import ThreadPoolExecutor

from core.input_types import NATIVE
from core.timers import timer

from core import library
from core.i18n import current_language, tr
from ingetrazo_render import ComponentCatalogue
from ingetrazo_render import chrome_sheet

# Two Library windows can request the same archive. Serialise index and
# archive writes. The thumbnail service has its own coalescing worker pool.
_files = ThreadPoolExecutor(max_workers=1, thread_name_prefix="component-files")


class CatalogueSession:
    def __init__(self, ctx):
        self.ctx, self.document = ctx, ctx.win._doc
        self.model = ComponentCatalogue()
        self.timer = timer(ctx.win if NATIVE else None, self.poll, 80, repeat=True)
        self.entry = None
        self.artwork = []
        self.index(False)

    def index(self, refresh):
        self.job = _files.submit(library.index, refresh)
        self.timer.start()

    def close(self):
        self.timer.stop()
        if self.job is not None:
            self.job.cancel()
            self.job = None
        self.timer = None

    def build(self, base, query):
        sheet = chrome_sheet.Spec(json.loads(self.model.sheet(
            json.dumps(base.wire()), current_language(), query, tr)))
        for icon in self.artwork:
            sheet.icon(*icon)
        self.artwork.clear()
        return sheet

    def act(self, verb, args):
        key = args[0] if args else ""
        if verb == "visible":
            library.prefetch_thumbnails(self.model.visible(args))
            self.timer.start()
        elif verb == "focus":
            self.model.select(key)
            return key.startswith("online:")
        elif verb == "side":
            self.model.select_category(key)
        elif verb == "verb" and key == "refresh_catalogue" and self.job is None:
            self.index(True)
        elif (verb == "open" and key.startswith("online:")) or (verb == "verb" and key == "insert_online"):
            if self.job is None and (entry := self.model.insert(key if verb == "open" else None)):
                self.entry = json.loads(entry)
                self.job = _files.submit(library.model_file, self.entry)
                self.timer.start()
        else:
            return False
        return True

    def poll(self):
        completed = self.job is not None and self.job.done()
        if completed:
            result = self.job.result() if self.job.exception() is None else None
            self.job = None
            if self.entry is None:
                self.model.load(json.dumps(result or []))
            else:
                entry, self.entry = self.entry, None
                self.model.download_finished(result is not None)
                if result is not None and self.ctx.win._doc is self.document:
                    chrome_sheet.close(self.ctx.win)
                    self.ctx.win.insert_library_component(entry, result)
                    return
        self.artwork.extend(self.model.previews(str(library.library_cache())))
        if completed or self.artwork:
            self.ctx.refresh()
        if self.job is None and not library.thumbnails_pending():
            self.timer.stop()
