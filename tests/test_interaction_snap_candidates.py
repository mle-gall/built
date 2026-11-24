"""Document interaction policies shared by the controller and viewport facades."""
from types import SimpleNamespace

import numpy as np

from views.document_interaction import DocumentInteraction as Interaction


def test_loose_candidates_keep_document_order_until_capped():
    projected = (np.array([3., 1., 2., 4.]), np.zeros(4),
                 np.array([3., 1., 2., 4.]), np.ones(4), np.ones(4, bool))
    view = SimpleNamespace(_ledge_screen=lambda: projected,
                           _pick_index=lambda near: SimpleNamespace(edges=list("abcd")))
    nearby = Interaction._nearby_loose_edges
    assert nearby(view, 0, 0, 4, 3) == list("abc")
    assert nearby(view, 0, 0, 4, 2) == list("bc")
