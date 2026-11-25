"""The model clipboard on the platform pasteboard: one type written, the
Qt-era type still read, a copy of this process not read back."""
from types import SimpleNamespace

from built_render import shell_services as native
from formats import clip_payload
from built_render import ModelClipboard

MIME = "application/x-built-clipboard"
QT_ERA = "com.trolltech.anymime.application--x-built-clipboard"


def _board(monkeypatch):
    items, reads, writes = {}, [], []

    def write(entries):
        items.clear()
        items.update(entries)
        writes.append(dict(items))
        return True

    def read(kind):
        reads.append(kind)
        return items.get(kind)

    monkeypatch.setattr(native, "available", lambda: True)
    monkeypatch.setattr(native, "pasteboard_write", write)
    monkeypatch.setattr(native, "pasteboard_read", read)
    monkeypatch.setattr(native, "pasteboard_types", lambda: (list(items), len(writes)))
    monkeypatch.setattr(clip_payload, "encode", lambda clip: b"copy of " + clip["name"])
    monkeypatch.setattr(clip_payload, "decode", lambda data: {"name": bytes(data)})
    return items, reads, writes, write


def _window(clipboard=None):
    return SimpleNamespace(viewport=SimpleNamespace(clipboard=clipboard))


def test_a_copy_is_written_once_under_the_model_type_and_not_read_back(monkeypatch):
    items, reads, writes, _ = _board(monkeypatch)
    board = ModelClipboard()
    assert not board.available()
    source = {"name": b"a"}
    win = _window(source)
    board.publish(win, source)
    assert items == {MIME: b"copy of a"} and len(writes) == 1
    assert board.available()
    board.adopt(win)                         # its own copy: nothing to read
    assert win.viewport.clipboard is source and reads == []
    # Another document of the process gets objects of its own, once.
    other = _window()
    board.adopt(other)
    adopted = other.viewport.clipboard
    assert adopted == {"name": b"copy of a"} and other.viewport._clip_adopted_from is source
    board.adopt(other)
    assert other.viewport.clipboard is adopted and reads == [MIME]


def test_a_copy_of_an_older_build_is_read_by_its_qt_era_type(monkeypatch):
    items, reads, _, write = _board(monkeypatch)
    board = ModelClipboard()
    write([(QT_ERA, b"older build")])
    assert board.available()
    win = _window()
    board.adopt(win)
    assert win.viewport.clipboard == {"name": b"older build"} and reads == [QT_ERA]
    # Both on the board: ours wins.
    write([(QT_ERA, b"old"), (MIME, b"new")])
    board.adopt(win)
    assert win.viewport.clipboard == {"name": b"new"}
    write([("public.utf8-plain-text", b"text")])
    assert not board.available()
    board.adopt(win)
    assert win.viewport.clipboard == {"name": b"new"}
