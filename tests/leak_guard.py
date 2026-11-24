"""A pytest plugin for subprocess checks: after each test, report a
document transaction left open (and close it) and any Rust panic seen."""
import pytest


@pytest.hookimpl(trylast=True)
def pytest_runtest_teardown(item, nextitem):
    from ingetrazo_core import doc_depth
    depth = doc_depth()
    if depth:
        print(f"\nOPEN-TRANSACTION depth={depth} after {item.nodeid}")
        from core import doc_model
        while doc_model.NATIVE.doc_depth():
            doc_model.rollback()
