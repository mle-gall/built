# SPDX-License-Identifier: GPL-3.0-or-later
"""Draw a saved-scene tour into the selected native video writer."""
from pathlib import Path

from ingetrazo_core import VideoTimeline, video_format, video_writer
from core.i18n import tr


class VideoError(RuntimeError):
    """An export failed before the destination was published."""


def _frame(window, viewport, width, height, overlays):
    display = getattr(getattr(window, "_doc", None), "stand_in", None)
    if getattr(display, "renderer", None) is not None:
        return display.render_pixels(width, height, overlays=overlays, annotations_only=True)
    # The classic GL viewport still supplies an image.
    image = viewport.render_image(width, height, overlays=overlays, annotations_only=True)
    if image is not None and not image.isNull():
        image = image.convertToFormat(image.Format.Format_RGBA8888)
        return image.width(), image.height(), bytes(image.constBits())


def export_video(window, path, width, height, fps, progress=None):
    from core.saved_views import SavedView, shown_state
    from core.scene_animation import Pose, video_frame_count

    viewport = window.viewport
    scene = viewport.scene
    views = list(scene.saved_views)
    if not views:
        raise VideoError(tr("This project has no scenes to play: keep a view "
                            "as a scene first (Scenes panel, «+ Scene»)."))
    window.scene_player.cancel()
    camera = viewport.camera
    before = SavedView.capture("", scene, camera)
    aspect = camera.aspect
    poses = [Pose.of_view(v) for v in views]
    anim = scene.animation
    total = video_frame_count(len(poses), anim.transition, anim.pause, fps)
    timeline = VideoTimeline(poses, anim.transition, anim.pause)
    writer = None
    try:
        writer = video_writer(path, width, height, fps)
        width, height = writer.width, writer.height
        same_shape = abs(width / height - max(viewport.width(), 1) / max(viewport.height(), 1)) < 0.01
        shown = -1
        for i in range(total):
            pose, reached = timeline.at(i / fps)
            if reached != shown:
                shown = reached
                state = shown_state(scene)
                views[reached].apply_state(scene)
                if shown_state(scene) != state:
                    scene.bump_view()
            pose.put_on(camera)
            camera.aspect = width / height
            frame = _frame(window, viewport, width, height, same_shape)
            if frame is None or frame[:2] != (width, height):
                raise VideoError(tr("Could not render the view."))
            writer.write(frame[2])
            if progress is not None and progress(i + 1, total) is False:
                return 0
        writer.close()
        return writer.frames
    except (OSError, ValueError, RuntimeError) as exc:
        raise VideoError(str(exc)) from exc
    finally:
        if writer is not None:
            writer.discard()
        state = shown_state(scene)
        before.apply(scene, camera)
        camera.aspect = aspect
        if shown_state(scene) != state:
            scene.bump_view()
        viewport.update()


def run_export(window):
    from ingetrazo_render import OperationForm
    from core.scene_animation import video_seconds
    from views import chrome_dialogs, chrome_forms
    from views.filedialogs import file_dialogs

    vp = window.viewport
    scene = vp.scene
    if not scene.saved_views:
        chrome_dialogs.information(window, tr("Export Video"), tr(
            "This project has no scenes to play: keep a view as a scene first (Scenes panel, «+ Scene»)."))
        return
    current = window._current_path
    name = current.stem if current else "untitled"
    suffix, format_label = video_format()
    chosen, _ = file_dialogs.getSaveFileName(window, tr("Export Video"), name + suffix, format_label)
    if not chosen:
        return
    path = Path(chosen)
    if path.suffix.lower() != suffix:
        path = path.with_suffix(suffix)
    count = len(scene.saved_views)
    seconds = video_seconds(count, scene.animation.transition, scene.animation.pause)
    model = OperationForm.video(vp.width(), vp.height(), count, seconds,
                                tr("Like the view ({w} × {h})"),
                                tr("{scenes} scenes, {seconds} s ({frames} frames)"))

    def write(values):
        if window.viewport is not vp:
            return
        bar, update = window._import_progress(tr("Rendering the video…"))

        def progress(done, total):
            update(done / total, tr("Rendering the video…"))
            return window.viewport is vp and not bar.wasCanceled()

        try:
            frames = export_video(window, path, **values, progress=progress)
        except VideoError as exc:
            chrome_dialogs.close_failed(bar, exc)
            chrome_dialogs.critical(window, tr("Export Video failed"), str(exc))
            return
        bar.close()
        message = tr("Exported video {w}×{h}, {frames} frames → {name}", w=values["width"],
                     h=values["height"], frames=frames, name=path.name) if frames else tr("Video export cancelled.")
        window.statusBar().showMessage(message, 6000)

    chrome_forms.show(window, chrome_forms.OperationForm(tr("Export Video"), model, write,
        note=format_label + "\n" + tr("The times are those of Camera ▸ Animation ▸ Animation Settings…")))
