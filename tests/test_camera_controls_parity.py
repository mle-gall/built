"""Camera controls recorded from the Python class on 9388c25dd."""
import gzip
import json
import random
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "camera_controls.json.gz"


def samples():
    from core.camera import OrbitCamera, frame_rect
    from ingetrazo_core import Vec3

    rng = random.Random(4367)
    for index in range(96):
        camera = OrbitCamera()
        camera.set_view(("top", "front", "iso", "missing")[index % 4])
        camera.set_aspect(rng.randrange(-2, 1920), rng.randrange(-2, 1080))
        camera.set_projection(("parallel", "two_point", "perspective")[index % 3])
        sensor = ([36, 24], [23.6, 15.6], ["17.3", 13], [0, 100], None)[index % 5]
        camera.apply_lens(dict(sensor=sensor, shift_x=rng.uniform(-2, 2),
                               shift_y=rng.uniform(-2, 2), roll_deg=rng.uniform(-720, 720),
                               clip_near=index % 3, clip_far=index % 4, frame_aspect=index % 4))
        lens = camera.lens_dict()
        optics = [camera.picture_height_mm(), camera.focal_mm, camera.fov_for_focal(50)]
        rect = frame_rect(1279, 767, camera.frame_aspect)
        def point():
            return Vec3(*(rng.uniform(-200, 200) for _ in range(3)))
        target, eye, pivot = point(), point(), point()
        placed = camera.place(eye, target if index % 8 else eye)
        camera.orbit_about(pivot, 32, -17, 671)
        camera.pan(-23, 11, 767, (None, -1, 32.)[index % 3])
        camera.zoom(2)
        camera.zoom_to(-3, pivot, 0.8)
        camera.move_eye(Vec3(5, -3, 1))
        camera.toggle_projection()
        camera.toggle_two_point()
        camera.reset_lens()
        yield [lens, optics, rect, placed, camera.target.toTuple(),
               camera.distance, camera.yaw, camera.pitch, camera.lens_dict(),
               camera.projection_mode(), camera.sees_from_below(0)]


def encoded():
    def exact(value):
        if isinstance(value, float):
            return value.hex()
        if isinstance(value, (tuple, list)):
            return [exact(x) for x in value]
        if isinstance(value, dict):
            return {k: exact(v) for k, v in value.items()}
        return value
    return [exact(row) for row in samples()]


def test_camera_controls_match_python():
    assert encoded() == json.loads(gzip.decompress(GOLDEN.read_bytes()))
