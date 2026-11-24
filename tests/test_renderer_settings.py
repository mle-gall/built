"""The GPU check asks the render crate for an adapter that can draw the scene."""
import sys
from types import SimpleNamespace

import pytest
from core import renderer_settings as rs


@pytest.fixture(autouse=True)
def fresh():
    rs.capability.cache_clear()
    yield
    rs.capability.cache_clear()


def test_missing_crate(monkeypatch):
    monkeypatch.setitem(sys.modules, 'ingetrazo_render', None)
    cap = rs.capability()
    assert not cap.available
    assert 'render crate unavailable' in cap.reason


@pytest.mark.parametrize('fails', [False, True])
def test_adapter_probe(monkeypatch, fails):
    def adapter():
        if fails:
            raise ValueError('no Metal adapter')
        return 'Apple M5 Max'
    monkeypatch.setitem(sys.modules, 'ingetrazo_render', SimpleNamespace(gpu_adapter=adapter))
    cap = rs.capability()
    assert cap.available is not fails and cap.python_version
    if fails:
        assert 'no Metal adapter' in cap.reason
        assert rs.status_text().startswith('No supported GPU backend')
    else:
        assert cap.adapter == 'Apple M5 Max' and rs.status_text() == 'wgpu'
