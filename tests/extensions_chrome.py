"""Drive the real Extensions provider against a private plugin folder and catalogue."""
from types import SimpleNamespace

from core import extension_manager as manager, extensions
from views import chrome_sheet
from views.report_sheets import Extensions


class Browser:
    def __init__(self, monkeypatch, root, entries=(), *, reviewed=None, bundled=(), window=None):
        def unexpected(url, *_):
            raise AssertionError(f"Unexpected fixture download {url}")

        monkeypatch.setattr(manager, "_get", unexpected)
        monkeypatch.setattr(manager, "user_plugins_dir", lambda: root)
        monkeypatch.setattr(extensions, "plugin_dirs", lambda: [root / ".shipped", root])
        snapshot = manager.CatalogSnapshot(list(entries), reviewed or {}, "2026-10-07", True)
        monkeypatch.setattr(manager, "load_cached_catalog", lambda: snapshot)
        monkeypatch.setattr(manager, "refresh_catalog", lambda: snapshot)
        monkeypatch.setattr(manager, "bundled_extensions", lambda: bundled)
        self.ctx = chrome_sheet.Ctx(window or SimpleNamespace(), None)
        self.provider = Extensions()
        self.provider.cache = root / ".artwork"
        self.read()

    def read(self):
        self.spec = self.provider.build(self.ctx).wire()
        blocks = self.spec["content"]["blocks"]
        self.items = blocks[0]["items"] if blocks else []
        self.ids = [item["id"] for item in self.items]
        self.blocks = (self.spec["detail"] or {}).get("blocks", [])
        self.buttons = {v["id"]: v["label"] for b in self.blocks for v in b.get("buttons", ())}
        self.facts = {label: value for b in self.blocks for label, value, _ in b.get("rows", ())}
        self.text = [b["text"] for b in self.blocks if "text" in b]
        return self

    def act(self, kind, *args):
        self.provider.act(self.ctx, kind, list(args))
        return self.read()

    def confirm(self, monkeypatch, answer="ok"):
        from views import chrome_dialogs
        questions = []

        def ask(win, title, text, buttons, *, cancel, default):
            questions.append((title, text, default))
            return answer
        monkeypatch.setattr(chrome_dialogs, "ask", ask)
        return questions
