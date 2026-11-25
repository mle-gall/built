import pytest

from built_core.scene import Scene
from built_core.group import Group
from formats.igz import load_into, save_scene


def parentage(roots):
    result = []
    pending = [(g, None) for g in roots]
    while pending:
        group, parent = pending.pop()
        result.append((group.uid, parent, group.is_component()))
        pending.extend((child, group.uid) for child in reversed(group.children))
    return result


@pytest.mark.parametrize("extension", ["igz", "built"])
@pytest.mark.parametrize("codec", ["0", "1"])
def test_archive_preserves_deep_placements(tmp_path, monkeypatch, extension, codec):
    monkeypatch.setenv("BUILT_RUST_IO", codec)
    scene = Scene()
    for table in (scene.groups, scene.kept_components):
        root = Group()
        parent = root
        for _ in range(100):
            child = Group()
            parent.adopt([child])
            parent = child
        root.component = True
        table.append(root)
    expected = [parentage(scene.groups), parentage(scene.kept_components)]
    assert all(len(rows) == 101 for rows in expected)
    path = tmp_path / f"deep.{extension}"
    save_scene(scene, path)
    restored = Scene()
    load_into(restored, path)
    assert [parentage(restored.groups), parentage(restored.kept_components)] == expected


@pytest.mark.parametrize("codec", ["0", "1"])
def test_json_decoder_limit_leaves_live_document_intact(tmp_path, monkeypatch, codec):
    monkeypatch.setenv("BUILT_RUST_IO", codec)
    scene = Scene()
    group = Group()
    scene.groups.append(group)
    path = tmp_path / "deep.igz"
    chain = '{"children":[' * 2000 + '{}' + ']}' * 2000
    path.write_text('{"scene":{"groups":[' + chain + ']}}')
    try:
        load_into(scene, path, include_links=False)
    except ValueError as exc:
        assert "nesting" in str(exc)
        assert list(scene.groups) == [group]
    else:
        assert len(parentage(scene.groups)) == 2001
