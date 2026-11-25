# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #602 (ArchXQ IT): File ▸ Open of another document kept the
previous document's section planes. The writer leaves a block out when it
holds the default (no planes, nothing hidden…), so the loader must reset
what the file does not bring — otherwise it leaks from the document that
was open before. One fixture that sets every such field, one plain file,
and the scene must end exactly as if the plain file were the only one
ever opened.

Issue #765 is the same leak for dimensions: ``load_into()`` appended the
file's dimensions without clearing the previous ones, so they doubled on
every open/save cycle. The rich fixture carries dimensions too."""
from __future__ import annotations

import os
from pathlib import Path
import sys

import pytest
from built_core import Vec3 as QVector3D

from core.dimension import Dimension
from core.layers import Layer
from core.scene import Scene
from built_core.section import SectionPlane
from formats import igz
from scripts.isolated_launch import run
from tests.shell_isolation import restore_shell_patches  # noqa: F401

#: Runtime-only or identity fields: not document state, or compared apart.
_SKIP = {"mesh", "version", "view_version", "selection"}


def _square(scene, z=0.0):
    scene.mesh.add_face([QVector3D(0, 0, z), QVector3D(1, 0, z),
                         QVector3D(1, 1, z), QVector3D(0, 1, z)])


def _rich(tmp_path):
    s = Scene()
    s.animation.enabled = False
    s.animation.transition = 7.5
    s.animation.pause = 3.0
    _square(s)
    s.section_planes = [
        SectionPlane(QVector3D(0, 0, 1), QVector3D(0, 0, 1), "Plan", "P",
                     active=True),
        SectionPlane(QVector3D(0.5, 0, 0), QVector3D(1, 0, 0), "A", "A")]
    s.show_section_planes = False
    s.show_section_cuts = False
    s.show_hidden_objects = True
    s.show_hidden_geometry = True
    s.show_axes = False
    s.back_face_color = (0.1, 0.2, 0.3)
    s.custom_scales = [33.0]
    s.layers.append(Layer("Walls"))
    s.dimension_style = dict(s.dimension_style, extra_key=1)
    s.dimensions.append(Dimension(QVector3D(0, 0, 0), QVector3D(1, 0, 0),
                                  QVector3D(0, -1, 0)))
    s.dimensions.append(Dimension(QVector3D(0, 0, 0), QVector3D(0, 1, 0),
                                  QVector3D(-1, 0, 0)))
    path = tmp_path / "rich.igz"
    igz.save_scene(s, path)
    return path


def _plain(tmp_path):
    s = Scene()
    _square(s, 2.0)
    path = tmp_path / "plain.igz"
    igz.save_scene(s, path)
    return path


def _state(scene):
    out = {}
    for k, v in vars(scene).items():
        if k.startswith("_") or k in _SKIP:
            continue
        if k == "layers":
            v = [(ly.name, ly.visible) for ly in v]
        elif k == "animation":
            v = v.to_dict()
        out[k] = repr(v)
    return out


def test_opening_a_plain_file_drops_the_previous_section_planes(tmp_path):
    rich, plain = _rich(tmp_path), _plain(tmp_path)
    scene = Scene()
    igz.load_into(scene, rich)
    assert len(scene.section_planes) == 2      # the fixture does load them
    igz.load_into(scene, plain)
    assert scene.section_planes == []
    assert scene.show_section_planes is True
    assert scene.show_section_cuts is True


def _dimension_points(scene):
    return [((d.a.x(), d.a.y(), d.a.z()), (d.b.x(), d.b.y(), d.b.z()),
             (d.offset.x(), d.offset.y(), d.offset.z()))
            for d in scene.dimensions]


def test_opening_a_plain_file_drops_the_previous_dimensions(tmp_path):
    rich, plain = _rich(tmp_path), _plain(tmp_path)
    scene = Scene()
    igz.load_into(scene, rich)
    assert len(scene.dimensions) == 2      # the fixture does load them
    igz.load_into(scene, plain)
    assert scene.dimensions == []


def test_reopening_the_same_file_does_not_double_its_dimensions(tmp_path):
    rich = _rich(tmp_path)
    scene = Scene()
    for _ in range(3):
        igz.load_into(scene, rich)
        assert len(scene.dimensions) == 2


def test_a_file_with_dimensions_brings_exactly_its_own(tmp_path):
    rich, plain = _rich(tmp_path), _plain(tmp_path)
    scene = Scene()
    igz.load_into(scene, plain)
    igz.load_into(scene, rich)
    assert _dimension_points(scene) == [
        ((0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, -1.0, 0.0)),
        ((0.0, 0.0, 0.0), (0.0, 1.0, 0.0), (-1.0, 0.0, 0.0)),
    ]


def test_nothing_of_the_previous_document_survives_an_open(tmp_path):
    rich, plain = _rich(tmp_path), _plain(tmp_path)
    fresh = Scene()
    igz.load_into(fresh, plain)
    reused = Scene()
    igz.load_into(reused, rich)
    assert reused.animation.to_dict() == {"enabled": False, "transition": 7.5, "pause": 3.0}
    igz.load_into(reused, plain)
    a, b = _state(fresh), _state(reused)
    leaks = {k: (a.get(k), b.get(k)) for k in set(a) | set(b)
             if a.get(k) != b.get(k)}
    assert leaks == {}


def test_a_file_with_planes_still_brings_its_own(tmp_path):
    """The reset must not eat what the file does carry."""
    rich, plain = _rich(tmp_path), _plain(tmp_path)
    scene = Scene()
    igz.load_into(scene, plain)
    igz.load_into(scene, rich)
    assert [p.name for p in scene.section_planes] == ["Plan", "A"]
    assert scene.section_planes[0].active
    assert scene.show_section_planes is False
    assert scene.show_section_cuts is False
    assert scene.show_hidden_objects is True
    assert scene.show_hidden_geometry is True
    assert scene.show_axes is False
    assert scene.back_face_color == (0.1, 0.2, 0.3)
    assert scene.custom_scales == [33.0]
    assert [ly.name for ly in scene.layers][-1] == "Walls"


def test_new_after_a_document_with_only_planes_clears_them():
    """File ▸ New: ``Scene.clear`` skipped its reset when the document
    held nothing but section planes."""
    s = Scene()
    s.section_planes = [SectionPlane(QVector3D(0, 0, 1),
                                     QVector3D(0, 0, 1))]
    s.show_section_cuts = False
    s.clear()
    assert s.section_planes == []
    assert s.show_section_cuts is True


def test_the_window_opens_another_file_without_the_old_planes(native_host, tmp_path):
    from tests.test_tree_links import _View
    from views import view_commands, wgpu_ui

    view = _View()
    view._ui_exp = {"sections:all"}
    for path, expected, checked in [(_rich(tmp_path), ["Plan", "A"], 0), (_plain(tmp_path), [], 1)]:
        assert native_host.open_path(path)
        win = native_host.window
        assert [plane.name for plane in win.viewport.scene.section_planes] == expected
        view_commands.sync(view, win)
        clusters, menus = view.renderer.calls["ui_set_cluster"]
        states = {item[0]: item[4] for group in list(clusters) + [rows for _, rows in menus]
                  for item in group}
        assert states["section_planes"] == states["section_cuts"] == checked
        wgpu_ui._tree(view, win)
        rows, _ = view.renderer.calls["ui_set_tree"]
        assert [row[0] for row, _ in rows if row[0].startswith("section:")] == [
            f"section:{plane.uid}" for plane in win.viewport.scene.section_planes]


def test_opening_plain_after_sections_renders_the_same_picture(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    if sys.platform != "darwin" or not binary.is_file():
        pytest.skip("requires the macOS release shell")
    rich, plain = _rich(tmp_path), _plain(tmp_path)
    pictures = []
    for prefix in [[], [rich]]:
        profile = tmp_path / f"profile-{len(prefix)}"
        plugin = profile / "data/built/plugins/view_sample.py"
        plugin.parent.mkdir(parents=True)
        plugin.write_text("""def setup(app):
    from pathlib import Path
    def sample():
        view = app.viewport.keys_stand_in
        width, height, pixels = view.render_pixels(320, 240, overlays=True)
        assert (width, height) == (320, 240)
        Path(__file__).with_suffix(".rgba").write_bytes(pixels)
    app.add_menu_action("Sample model view", sample)
""")
        script = profile / "open.txt"
        script.write_text("\n".join(["wait 200", *(f"open {p}" for p in [*prefix, plain]),
                                      "wait 250", "menu extension:view_sample:0", "quit", ""]))
        result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                      "--size", "800x600", "--script", script], settings=profile,
                     env=dict(os.environ, BUILT_START="0"), text=True, capture_output=True, timeout=60)
        assert result.returncode == 0 and "[built] error" not in result.stderr, result.stderr
        image = plugin.with_suffix(".rgba").read_bytes()
        assert len(image) == 320 * 240 * 4 and len(set(image[::4])) > 1
        pictures.append(image)
    assert pictures[0] == pictures[1]


def test_a_file_already_doubled_opens_with_each_dimension_once(tmp_path):
    # #765: files saved before the fix hold each dimension many times over
    # (1024 entries for 8 in the report). Opening one heals it.
    s = Scene()
    _square(s, 2.0)
    one = (QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(0, -1, 0))
    two = (QVector3D(0, 0, 0), QVector3D(0, 1, 0), QVector3D(-1, 0, 0))
    for _ in range(8):
        s.dimensions.append(Dimension(*one))
        s.dimensions.append(Dimension(*two))
    path = tmp_path / "doubled.igz"
    igz.save_scene(s, path)
    scene = Scene()
    igz.load_into(scene, path)
    assert sorted(_dimension_points(scene)) == sorted([
        ((0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, -1.0, 0.0)),
        ((0.0, 0.0, 0.0), (0.0, 1.0, 0.0), (-1.0, 0.0, 0.0)),
    ])
    # …and saving it again writes the healed list.
    igz.save_scene(scene, path)
    again = Scene()
    igz.load_into(again, path)
    assert len(again.dimensions) == 2


def test_two_dimensions_that_differ_in_anything_both_stay(tmp_path):
    s = Scene()
    _square(s, 2.0)
    a, b = QVector3D(0, 0, 0), QVector3D(1, 0, 0)
    s.dimensions.append(Dimension(a, b, QVector3D(0, -1, 0)))
    s.dimensions.append(Dimension(a, b, QVector3D(0, -2, 0)))   # farther out
    s.dimensions.append(Dimension(a, b, QVector3D(0, -1, 0), text="A"))
    path = tmp_path / "three.igz"
    igz.save_scene(s, path)
    scene = Scene()
    igz.load_into(scene, path)
    assert len(scene.dimensions) == 3
