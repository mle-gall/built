"""Persisted draw chunks must open across processes and winding changes."""
from types import SimpleNamespace as Record

from built_core import Vec3
from views.document_interaction import DocumentInteraction as Controller


def test_cache_key_includes_winding_and_paint_independently_of_session_hash():
    loop = [Record(position=Vec3(*p)) for p in [(0, 0, 0), (1, 0, 0), (0, 1, 0)]]
    face = Record(loop=loop, attrs={"color": [0.5, 0.25, 0.75]})
    mesh = Record(faces=[face])
    fp = (3, 3, 1, 2.0, 0, 0, 0)
    key = Controller._chunk_cache_key(mesh, fp)
    assert key == "4cee49175b9a3be1bfc4beb6974343892391b1ff"
    assert Controller._chunk_cache_key(mesh, (*fp[:4], 123, *fp[5:])) == key
    face.loop = loop[::-1]
    assert Controller._chunk_cache_key(mesh, fp) != key
    face.loop = loop
    face.attrs["opacity"] = 0.5
    assert Controller._chunk_cache_key(mesh, fp) != key
