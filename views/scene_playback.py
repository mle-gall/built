"""Bind the Rust camera timeline to a window's clock, viewport and saved scenes."""
import time

from ingetrazo_core import Pose, ScenePlayback


class Playback:
    def __init__(self, window, clock=None):
        self._window = window
        self._clock = clock or time.monotonic
        self.timeline = ScenePlayback()
        self._viewport = self._display = self._arrive = None
        self._views = ()

    def is_active(self):
        return self.timeline.is_active()

    def is_playing(self):
        return self.timeline.is_playing()

    def glide_to(self, view, seconds, arrive):
        self.cancel()
        vp = self._window.viewport
        if not self.timeline.start_glide(Pose.of_camera(vp.camera), Pose.of_view(view), seconds, self._clock()):
            return False
        self._start(vp, [view], arrive)
        return True

    def play(self, views, start, transition, pause, arrive):
        self.cancel()
        views = list(views)
        if not views:
            return False
        start %= len(views)
        order = views[start:] + views[:start]
        vp = self._window.viewport
        self.timeline.start_run(Pose.of_camera(vp.camera), [Pose.of_view(v) for v in order],
                                transition, pause, self._clock())
        self._start(vp, order, arrive)
        self.tick()
        return True

    def _start(self, vp, views, arrive):
        self._viewport, self._views, self._arrive = vp, views, arrive
        display = getattr(getattr(self._window, "_doc", None), "stand_in", None)
        self._display = display if getattr(display, "source_viewport", None) is vp else None
        self._set_running(True)

    def _set_running(self, on):
        if on:
            self._timer.start()
        else:
            self._timer.stop()

    def _end(self):
        self._set_running(False)
        self._viewport = self._display = self._arrive = None
        self._views = ()

    def cancel(self):
        if self.is_active():
            self.timeline.cancel()
            self._end()

    def stop(self):
        if self.is_active():
            pending, views, arrive = self.timeline.stop(), self._views, self._arrive
            self._end()
            if pending is not None and arrive is not None:
                arrive(views[pending])

    def tick(self):
        if not self.is_active():
            return
        vp = self._viewport
        if vp is not self._window.viewport:
            self.cancel()
            return
        camera = vp.camera
        pose, reached = self.timeline.tick(self._clock(), Pose.of_camera(camera))
        if pose is not None:
            pose.put_on(camera)
            vp.update()
            if self._display is not None:
                self._display.update()
        arrive = self._arrive
        view = self._views[reached] if reached is not None else None
        if self.is_playing() and view is not None and not any(view is v for v in vp.scene.saved_views):
            self.cancel()
            return
        if not self.is_active():
            self._end()
        if view is not None and arrive is not None:
            arrive(view)
            self.timeline.accept_camera(Pose.of_camera(camera))


class WindowPlayback(Playback):
    def __init__(self, window, timers):
        super().__init__(window)
        self._timer = timers.timer(self.tick, interval=16, repeat=True)


def edit_animation(win):
    from ingetrazo_render import OperationForm
    from core.i18n import tr
    from core.scene_animation import SceneAnimation
    from views import chrome_forms

    scene = win.viewport.scene
    current = scene.animation

    def apply(values):
        fresh = SceneAnimation(**values)
        if win.viewport.scene is scene and fresh != scene.animation:
            scene.animation = fresh
            if not fresh.enabled:
                win.scene_player.stop()
            scene.version += 1
            win._sync_animation_menu()

    model = OperationForm.animation(current.enabled, current.transition, current.pause)
    return chrome_forms.show(win, chrome_forms.OperationForm(tr("Animation Settings"), model, apply))
