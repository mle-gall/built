"""Native staging retains serialized content and the resident arrays."""
import time

import numpy as np

from built_core.scene import Scene
from formats import igz
from built_core import DocumentOpen
from tests.fixtures.shell.make_composite import make
from built_core import open_focus, pack_scene


def test_worker_matches_blocking_nested_open(tmp_path):
    path = make(tmp_path)["host"]
    reference = Scene()
    igz.load_into(reference, path)
    expected = pack_scene(reference, focus=open_focus(reference))
    worker = DocumentOpen(path)
    deadline = time.monotonic() + 30
    while (result := worker.take()) is None:
        assert time.monotonic() < deadline
        time.sleep(0.005)
    scene, packed, _, _ = result
    assert not scene.link_errors
    for field in ("vertices", "indices", "instances", "commands", "bounds"):
        np.testing.assert_array_equal(getattr(expected, field), getattr(packed, field))
    output = tmp_path / "copy.built"
    igz.save_scene(reference, output, backup=False)
    saved = output.read_bytes()
    igz.save_scene(scene, output, backup=False)
    assert output.read_bytes() == saved


def test_cancel_before_publication_discards_finished_result(tmp_path):
    path = make(tmp_path)["host"]
    worker = DocumentOpen(path)
    deadline = time.monotonic() + 30
    while not worker.ready():
        assert time.monotonic() < deadline
        time.sleep(0.005)
    worker.cancel()
    assert worker.take() is None
