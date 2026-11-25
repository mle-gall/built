"""Archive and coordinate values captured from the Python document on 1debc0992."""
import json
from pathlib import Path

import pytest

from built_core.scene import Scene
from built_core import Vec3, coordinate_frame, kept_components, project
from built_core.annotation_anchor import Attachment
from built_core.image_plane import ImagePlane


def test_reference_and_annotation_values_match_the_saved_document():
    expected = json.loads((Path(__file__).parent / "fixtures/core_document_values.json").read_text())
    a, b, c = Vec3(2, 3, 5), Vec3(4, 5, 6), Vec3(1, 2, 3)
    image = ImagePlane("a.png", a, b, c, 0.25, "a", 0.2, None, True)
    assert image.to_dict() == expected["archive"]
    assert [list(v.toTuple()) for v in image.corners()] == expected["corners"]
    assert list(image.project(a)) == expected["projection"]
    assert image.normal().toTuple() == pytest.approx(expected["normal"])
    texture = image.texture()
    assert texture["path"] == expected["texture"]["path"]
    for key in ("sw", "sh", "uvw"):
        assert texture[key] == pytest.approx(expected["texture"][key])

    scene = Scene()
    edge = scene.mesh.add_edge(a, b)
    anchor = Attachment.capture(scene, edge, None, (a + b) * 0.5)
    assert json.loads(json.dumps(anchor.archive())) == expected["attachment"]
    assert list(anchor.position().toTuple()) == expected["anchor_point"]
    assert project.path_for(scene) == expected["project_path"]
    assert [list(coordinate_frame.choose_origin(p)) for p in
            [(512, 0, 0), (512.5, -1023.5, 8)]] == expected["origins"]
    assert [kept_components.numbered_name(n, ["A #2", "A #3"])
            for n in ("A", "A #7", "S１")] == expected["component_names"]
