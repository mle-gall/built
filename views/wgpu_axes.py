"""Small axes uniform, independent of global core.axes state across documents."""
from ingetrazo_core import Matrix4 as QMatrix4x4


def axes_state(scene, camera, source=None, origin=(0., 0., 0.)):
    frame = scene.drawing_frame
    visible = (scene.show_axes or frame is not None)
    if source is not None:
        visible = visible and source.plano_style is None and source.style_override is None
    matrix = list((frame if frame is not None else QMatrix4x4()).data())
    matrix[12:15] = [v - o for v, o in zip(matrix[12:15], origin)]
    return (tuple(matrix),
            max(camera.distance * .008, 1e-4), visible)
