import pytest

from built_core import Matrix4
from built_core import doc_model as doc
from built_core.group import Group


@pytest.mark.parametrize("same", [True, False])
@pytest.mark.parametrize("accept", [True, False])
def test_container_and_transform_savepoints(same, accept):
    outer = Group()
    inner = outer if same else Group()
    for group in (outer, inner):
        group.ext = {"list": [], "key": 0}
        group.xform = Matrix4()
    doc.begin()
    outer.ext["key"] = 1
    outer.ext["list"].append("outer")
    outer.xform.translate(1, 0, 0)
    doc.begin()
    inner.ext["key"] = 2
    inner.ext["list"].append("inner")
    inner.xform.translate(2, 0, 0)
    if accept:
        doc.commit()
    else:
        doc.rollback()
    txn = doc.commit()
    assert outer.ext["key"] == (2 if same and accept else 1)
    assert outer.ext["list"] == (["outer", "inner"] if same and accept else ["outer"])
    assert outer.xform.data()[12] == (3 if same and accept else 1)
    if not same:
        assert inner.ext == ({"list": ["inner"], "key": 2} if accept else {"list": [], "key": 0})
        assert inner.xform.data()[12] == (2 if accept else 0)
    txn.undo()
    for group in (outer, inner):
        assert group.ext == {"list": [], "key": 0}
        assert group.xform.data()[12] == 0


def test_plain_container_alias_is_flushed_before_nested_mark():
    group = Group()
    group.ext = {}
    alias = group.ext["table"] = {}
    doc.begin()
    alias["outer"] = 1
    doc.begin()
    alias["inner"] = 2
    doc.rollback()
    assert group.ext == {"table": {"outer": 1}}
    txn = doc.commit()
    txn.undo()
    assert group.ext == {"table": {}}
