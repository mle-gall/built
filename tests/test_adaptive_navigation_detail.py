"""Navigation feedback is temporary and absent from exports."""
from built_core import Matrix4 as QMatrix4x4
from views import document_interaction as viewport_module
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_navigation_settles_and_respects_precision_contexts(native_host, monkeypatch):
    now = [10.0]
    monkeypatch.setattr(viewport_module._time_mod, 'monotonic', lambda: now[0])
    native_host.resize(800, 600, 1.0)
    vp = native_host.window.viewport
    vp._inst_pool = (None, [None] * 2000)
    a = QMatrix4x4(); b = QMatrix4x4(); b.translate(1, 0, 0)
    assert vp._navigation_detail(a, 'shaded') is False
    assert vp._navigation_detail(b, 'shaded') is True
    assert vp._nav_detail_timer.isActive()
    now[0] += .2
    assert vp._navigation_detail(b, 'shaded') is True
    now[0] += .2
    assert vp._navigation_detail(b, 'shaded') is False
    assert vp._navigation_detail(a, 'shaded') is True
    vp._export_size = (800, 600)
    assert vp._navigation_detail(b, 'shaded') is False
    vp._export_size = None
    assert vp._navigation_detail(b, 'wireframe') is False
    assert vp._navigation_detail(a, 'hidden_line') is False
    vp.scene.selection.add(object())
    assert vp._navigation_detail(a, 'shaded') is False
    vp.scene.selection.clear()
    vp._inst_pool = (None, [None] * 10)
    assert vp._navigation_detail(b, 'shaded') is False
