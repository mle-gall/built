"""Serialized clipboard offers retain a snapshot after the source scene closes."""
from app_shell import native
from formats import clip


def test_native_offer_tracks_external_changes_and_caches_bytes(monkeypatch):
    items = {}
    reads = []
    writes = []
    encodes = []

    def write(entries):
        items.clear()
        items.update(entries)
        writes.append(dict(items))
        return True

    def read(kind):
        reads.append(kind)
        return items[kind]

    def encode(scene):
        encodes.append(scene)
        return b"original"

    monkeypatch.setattr(native, "available", lambda: True)
    monkeypatch.setattr(native, "pasteboard_write", write)
    monkeypatch.setattr(native, "pasteboard_read", read)
    monkeypatch.setattr(native, "pasteboard_types", lambda: (list(items), len(writes)))
    monkeypatch.setattr(clip, "encode", encode)
    monkeypatch.setattr(clip, "decode", bytes)
    monkeypatch.setattr(clip, "_offer", None)
    monkeypatch.setattr(clip, "_change", None)

    source = object()
    clip.publish(source)
    assert clip.current_offer().clip is source
    assert clip.foreign() is None
    clip.flush()
    plain = clip.current_offer()
    assert not isinstance(plain, clip.ClipMime)
    assert bytes(plain.data(clip.MIME)) == b"original"
    assert encodes == [source] and reads == [] and len(writes) == 1

    kind, = items
    write([(kind, b"second")])
    replacement = clip.current_offer()
    assert replacement is not plain and reads == []
    assert clip.foreign() == b"second"
    assert clip.current_offer() is replacement and clip.foreign() == b"second"
    assert reads == [kind]
    write([("text/plain", b"text")])
    assert clip.current_offer() is None and not clip.available()
