# SPDX-License-Identifier: GPL-3.0-or-later
"""Saved sort choice when opening a new native Project tree."""
import pytest

from core.settings import settings
from tests.test_project_drawing_filters import Project


@pytest.mark.parametrize("index", [0, 1])
def test_sort_order_is_restored_for_a_new_view(index):
    key = "ui/project_tree_sort"
    store = settings()
    old = store.value(key)
    try:
        Project().action("sort", str(index))
        restored = Project()
        restored.rows()
        assert restored.view.renderer.calls["ui_set_tree_sort"] == (index,)
    finally:
        if old is None:
            store.remove(key)
        else:
            store.setValue(key, old)
