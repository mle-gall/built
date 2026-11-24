# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Working out a big model's visible faces for the SVG export shows a bar
with Cancel, and Cancel stops it.

Seen on a 1 035-object plaza: exporting the view as SVG, or only
previewing «Faces in colour», froze the window for minutes, with nothing
to say it was working and no way to give up but killing the program.
"""
from __future__ import annotations

import os

import numpy as np
import pytest

pytest.importorskip("manifold3d")

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtWidgets import QApplication              # noqa: E402

from core import progress as P                         # noqa: E402
from core import vista_caras as VC                      # noqa: E402
from core.vista_caras import FaceFill                   # noqa: E402
from tests.test_view_faces_cost import _scene           # noqa: E402
import ingetrazo_render as wgpu_deliveries
from views import chrome_dialogs
from tests.test_export_view_svg import _export_sheet
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def _faces(n=300):
    return [FaceFill(**vars(f)) for f in _scene(np.random.default_rng(5), n)]


# ---- The visible faces say how far they have come --------------------------

def test_the_visible_faces_tell_how_far_they_have_come(monkeypatch):
    from core import _native
    monkeypatch.setattr(_native, "native", None)      # the Python pass's own
    monkeypatch.setattr(P, "REPORT_EVERY", 0.0)
    told = []
    plain = VC._visible_parts(_faces())
    with P.reporting(told.append):
        reported = VC._visible_parts(_faces())
    assert P._report is None
    assert len(set(told)) > 100
    assert told == sorted(told)
    assert 0.0 <= told[0] and told[-1] < 1.0
    # Reporting changes nothing of what comes out.
    assert ([(f.depth, [r.tolist() for r in f.rings]) for f in reported]
            == [(f.depth, [r.tolist() for r in f.rings]) for f in plain])


def test_a_report_that_raises_stops_the_work_there(monkeypatch):
    from core import _native
    monkeypatch.setattr(_native, "native", None)      # counts _take_away
    monkeypatch.setattr(P, "REPORT_EVERY", 0.0)
    done = []
    real = VC._take_away

    def counted(visible, covers):
        done.append(1)
        return real(visible, covers)

    monkeypatch.setattr(VC, "_take_away", counted)
    VC._visible_parts(_faces())
    whole = len(done)
    done.clear()

    class Enough(Exception):
        pass

    def stop(at):
        if at > 0.25:
            raise Enough()

    with pytest.raises(Enough):
        with P.reporting(stop):
            VC._visible_parts(_faces())
    assert P._report is None
    assert 0 < len(done) < 0.5 * whole


def test_every_stretch_of_the_work_is_heard(monkeypatch):
    """Gathering the faces, filing them, cutting them down and their
    shadows: a stretch that said nothing was a window Windows marked as
    not responding before the bar came (14 s of a plaza's preview)."""
    from core.scene import Scene
    from tests.test_export_view_svg import _box, _camera, _sunny
    scene = Scene()
    from core.group import Group
    from core.mesh import Mesh
    for k in range(40):
        g = Group(Mesh(), name=f"c{k}")
        _box(g.mesh, x0=1.5 * (k % 8), y0=1.5 * (k // 8))
        scene.groups.append(g)
    scene.version += 1
    cam = _camera(scene)
    heard = []
    real_tick = P.tick

    def where(done=None):
        import sys
        heard.append(sys._getframe(1).f_code.co_name)
        real_tick(done)

    monkeypatch.setattr(P, "tick", where)
    _sunny(scene)
    with P.reporting(lambda done: None):
        VC.view_faces(scene, cam, shadows=True)
    # Gathering, cutting down (the Python pass or the Rust engine's
    # batches), the shadows — and the grid, when the Python pass runs.
    assert {"view_faces", "_cast_shadows"} <= set(heard)
    assert any(name.startswith("_visible_parts") for name in heard)
    from core import _native
    if _native.native is None:
        assert "__init__" in heard
    # And the hidden lines drawn over them (8.7 s of the plaza's export
    # after the faces, with nothing said).
    from core.hlr import hlr_drawing
    heard.clear()
    with P.reporting(lambda done: None):
        hlr_drawing(scene, cam)
    # The walk is heard; the classification runs in Rust in one call
    # (ingetrazo_core sheets::drawing).
    assert "eat_mesh" in heard


# ---- The bar ----------------------------------------------------------------


def _bars(monkeypatch, cancel_after=None, original=chrome_dialogs.Job):
    made = []

    class ObservedJob(original):
        def __init__(self, *a, **kw):
            super().__init__(*a, **kw)
            self.values = []
            made.append(self)

        def update(self, fraction, phase=None):
            self.values.append(fraction)
            super().update(fraction, phase)
            if cancel_after is not None and len(self.values) > cancel_after:
                self.cancel()

    monkeypatch.setattr(chrome_dialogs, "Job", ObservedJob)
    return made


class _JobDelay:
    """The Rust delay before a job shows, as an attribute monkeypatch restores."""

    @property
    def seconds(self):
        old = wgpu_deliveries.set_preview_job_delay(0.0)
        wgpu_deliveries.set_preview_job_delay(old)
        return old

    @seconds.setter
    def seconds(self, value):
        wgpu_deliveries.set_preview_job_delay(value)


def _no_delay(patch):
    """Show the job at once."""
    patch.setattr(_JobDelay(), "seconds", 0.0)


def test_a_small_model_never_sees_the_bar(monkeypatch):
    from types import SimpleNamespace

    win = SimpleNamespace(viewport=SimpleNamespace(history=object()))
    made = _bars(monkeypatch)
    with wgpu_deliveries.svg_progress(win, win.viewport.history, "SVG"):
        VC._visible_parts(_faces(20))
    assert made == []


def test_cancel_on_the_bar_raises_and_puts_everything_back(monkeypatch):
    from types import SimpleNamespace

    win = SimpleNamespace(viewport=SimpleNamespace(history=object()))
    made = _bars(monkeypatch, cancel_after=3)
    monkeypatch.setattr(P, "REPORT_EVERY", 0.0)
    _no_delay(monkeypatch)
    with pytest.raises(InterruptedError):
        with wgpu_deliveries.svg_progress(win, win.viewport.history, "SVG"):
            for n in range(10):
                P.tick(n / 10)
    assert len(made) == 1
    assert len(made[0].values) == 4 and made[0].values == sorted(made[0].values)
    assert made[0].cancelled and not win._chrome_jobs
    assert P._report is None and QApplication.overrideCursor() is None


def _window_with_faces(host):
    from tests.test_export_view_svg import _two_layer_scene
    win = host.window
    vp = win.viewport
    host.resize(800, 600, 1.)
    vp.camera.set_aspect(800, 600)
    _two_layer_scene(vp.scene)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    vp.camera.fit_to(lo, hi, 1.2)
    vp.camera.perspective = False
    return win


def test_cancel_stops_the_export_and_writes_nothing(native_host, tmp_path, monkeypatch):
    win = _window_with_faces(native_host)
    monkeypatch.setattr(chrome_dialogs, "in_shell", lambda: True)
    with monkeypatch.context() as observe:
        made = _bars(observe, cancel_after=0)
        observe.setattr(P, "REPORT_EVERY", 0.0)
        _no_delay(observe)
        _export_sheet(win, tmp_path / "vista.svg")
        assert not (tmp_path / "vista.svg").exists()
        assert len(made) == 1 and made[0].cancelled and not win._chrome_jobs
        assert win.statusBar().currentMessage() == "SVG export cancelled"
    # The same document can export immediately after cancellation.
    _export_sheet(win, tmp_path / "retry.svg")
    assert (tmp_path / "retry.svg").is_file()


def test_the_bar_left_alone_writes_the_svg_as_before(native_host, tmp_path, monkeypatch):
    win = _window_with_faces(native_host)
    _export_sheet(win, tmp_path / "antes.svg")
    made = _bars(monkeypatch)
    monkeypatch.setattr(P, "REPORT_EVERY", 0.0)
    _no_delay(monkeypatch)
    monkeypatch.setattr(chrome_dialogs, "in_shell", lambda: True)
    _export_sheet(win, tmp_path / "con-barra.svg")
    assert len(made) == 1 and made[0].values
    before = (tmp_path / "antes.svg").read_text(encoding="utf-8")
    after = (tmp_path / "con-barra.svg").read_text(encoding="utf-8")
    assert "<path" in before and before == after


def test_cancel_on_a_slow_preview_turns_the_preview_off(monkeypatch):
    import time
    from types import SimpleNamespace
    from core import deliveries
    from core.camera import OrbitCamera
    from core.history import History
    from core.scene import Scene
    from ingetrazo_render import chrome_sheet
    from PySide6.QtCore import QTimer

    scene = Scene()
    win = SimpleNamespace(viewport=SimpleNamespace(scene=scene, history=History(scene), camera=OrbitCamera()))
    ticks = []
    monkeypatch.setattr(QTimer, "singleShot", lambda delay, fn: ticks.append(fn))
    clock, built = [0.], []

    def render(*args, **kwargs):
        built.append(True)
        clock[0] += 1.
        P.tick(.25)
        pytest.fail("The preview writer continued after cancellation")
    monkeypatch.setattr(P, "REPORT_EVERY", 0)
    monkeypatch.setattr(deliveries, "preview_svg", render)
    monkeypatch.setattr(QApplication, "processEvents", lambda: win._chrome_jobs[0].cancel())
    form = wgpu_deliveries.delivery_export_form(win, one_off=True)
    form.set("format", "svg")
    provider = chrome_sheet.current(win)
    ctx = chrome_sheet.Ctx(win, None)
    svg = provider.build(ctx).forms()[1]
    svg.receive("svg_preview", "1")
    provider.build(ctx)
    assert len(ticks) == 1
    monkeypatch.setattr(time, "monotonic", lambda: clock[0])
    ticks.pop()()
    assert built == [True] and not win._chrome_jobs
    assert not svg.preview_enabled and provider.build(ctx).wire()["detail"] is None
    assert QApplication.overrideCursor() is None and not ticks
    chrome_sheet.close(win)


def test_a_preview_does_not_start_inside_another(tmp_path, monkeypatch):
    from tests.test_export_view_svg import _svg

    model = _svg()
    first = model.preview_ticket("first")
    assert model.begin_preview(first)
    assert model.preview_ticket("edit while rendering") is None
    assert not model.begin_preview(first) and not model.finish_preview(first, "")
    next_ticket = model.preview_ticket("edit while rendering")
    assert model.begin_preview(next_ticket) and model.finish_preview(next_ticket, "")


