from concurrent.futures import ThreadPoolExecutor

import pytest

from built_core import doc_model
from built_core.history import Command, History
from built_core.scene import Scene


class Probe(Command):
    def __init__(self):
        self.calls = 0

    def do(self, scene):
        self.calls += 1
        scene.dimension_style["font_size"] = 17

    def undo(self, scene):
        pass


@pytest.mark.parametrize("other_thread,other_doc", [(True, True), (True, False), (False, True)])
def test_conflicting_history_never_enters_do(other_thread, other_doc):
    scene = Scene()
    target = Scene() if other_doc else scene
    hist = History(target)
    probe = Probe()
    doc_model.begin(scene._root._doc_key)
    try:
        if other_thread:
            with ThreadPoolExecutor(1) as pool:
                pool.submit(hist.execute, probe).result()
        else:
            hist.execute(probe)
        assert probe.calls == 0
        assert hist.last_rejected
        assert not hist.undo_stack and hist.revision == 0
        assert doc_model.NATIVE.doc_depth() == 1
    finally:
        doc_model.rollback()
    hist.execute(probe)
    assert probe.calls == 1 and hist.last_txn is not None
