"""The Qt startup probe exercises the actual GPU backend before creating a document."""
import pytest
from core import renderer_settings as rs


def test_missing_crate(monkeypatch):
    import sys
    monkeypatch.setitem(sys.modules, 'ingetrazo_render', None)
    cap = rs._probe_local()
    assert not cap.available
    assert 'render crate unavailable' in cap.reason


@pytest.mark.parametrize('fails', [False, True])
def test_adapter_probe(monkeypatch, fails):
    import sys
    from types import SimpleNamespace
    def renderer(*args):
        if fails:
            raise ValueError('no Metal adapter')
        return SimpleNamespace(ui_enable=lambda font: None)
    monkeypatch.setattr('views.wgpu_surface.window_handles', lambda widget: (1, {}))
    monkeypatch.setitem(sys.modules, 'ingetrazo_render', SimpleNamespace(Renderer=renderer))
    cap = rs._probe_local()
    assert cap.available is not fails
    assert cap.python_version and cap.qt_version
    if fails:
        assert 'no Metal adapter' in cap.reason


def test_missing_chrome_rejects_renderer(monkeypatch):
    import sys
    from types import SimpleNamespace
    monkeypatch.setattr('views.wgpu_surface.window_handles', lambda widget: (1, {}))
    monkeypatch.setitem(sys.modules, 'ingetrazo_render', SimpleNamespace(Renderer=lambda *args: object()))
    result = rs._probe_local()
    assert not result.available
    assert 'ui_enable' in result.reason


@pytest.mark.parametrize('platform', ['linux', 'win32'])
def test_probes_the_native_surface_on_each_platform(monkeypatch, platform):
    import sys
    from types import SimpleNamespace
    monkeypatch.setattr(sys, 'platform', platform)
    monkeypatch.setattr('views.wgpu_surface.window_handles', lambda widget: (1, {}))
    monkeypatch.setitem(sys.modules, 'ingetrazo_render', SimpleNamespace(Renderer=lambda *args: SimpleNamespace(ui_enable=lambda font: None)))
    assert rs._probe_local().available



@pytest.mark.parametrize('outcome', ['ok', 'crash', 'timeout', 'malformed'])
def test_subprocess_boundary_and_cache(monkeypatch, outcome):
    from types import SimpleNamespace
    import subprocess
    calls = []
    def run(*args, **kwargs):
        calls.append(args)
        assert kwargs['timeout'] == 30
        if outcome == 'timeout':
            raise subprocess.TimeoutExpired(args[0], 30)
        return SimpleNamespace(returncode=-11 if outcome == 'crash' else 0,
                               stdout='{"available": true}' if outcome == 'ok' else '')
    monkeypatch.setattr(rs.subprocess, 'run', run)
    rs.capability.cache_clear()
    try:
        assert rs.capability().available is (outcome == 'ok')
        assert rs.capability() is rs.capability()
        assert len(calls) == 1
    finally:
        rs.capability.cache_clear()


def test_native_gpu_probe_opens_a_surface():
    import json
    from tests.launcher_binary import run
    result = run("--probe-gpu")
    assert result.returncode == 0, result.stderr
    assert isinstance(json.loads(result.stdout.splitlines()[-1])["available"], bool)


def test_unavailable_gpu_explains_why(monkeypatch):
    monkeypatch.setattr(rs, "capability", lambda: rs.Capability(False, "no adapter"))
    assert "no adapter" in rs.status_text()
