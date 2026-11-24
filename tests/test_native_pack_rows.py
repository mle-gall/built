# SPDX-License-Identifier: GPL-3.0-or-later
"""The pack's placement rows from the native walk (``doc_render_rows``)
match the editor proxies' walk they replace: flags, the paint each row
resolves to, mesh, billboard and uid (architecture review 12)."""
from types import SimpleNamespace

import pytest
from ingetrazo_core import Matrix4, Vec3

from core.group import Group
from core.layers import LINK_UNTAGGED_SUFFIX, Layer
from core.mesh import Mesh
from core.scene import Scene
from views import wgpu_scene
from views.document_interaction import _paint_of


def _mesh(x=0.0):
    m = Mesh()
    m.add_face([Vec3(x, 0, 0), Vec3(x + 1, 0, 0), Vec3(x, 1, 0)])
    return m


def _placed(mesh, name, *, tag=None, material=None, hidden=False, billboard=False, children=()):
    g = Group(mesh, name=name)
    g.xform = Matrix4()
    g.xform.translate(Vec3(len(name), 0, 0))
    if tag:
        g.layer = tag
    if material:
        g.material = material
    g.hidden = hidden
    if billboard:
        g.billboard = billboard
    if children:
        g.adopt(list(children))
    return g


def _scene():
    shared = _mesh()
    leaf = lambda name, **kw: _placed(shared, name, **kw)
    scene = Scene(groups=[
        _placed(_mesh(1), "painted", material={"color": (0.8, 0.2, 0.2)}, children=[
            leaf("plain"), leaf("own", material={"color": (0.1, 0.9, 0.1)}),
            leaf("tagged", tag="walls", children=[leaf("deep")])]),
        _placed(_mesh(2), "on-hidden-tag", tag="off", children=[leaf("inside", tag="walls")]),
        _placed(_mesh(3), "locked", tag="lock", children=[leaf("forced", tag="walls")]),
        _placed(_mesh(4), "hidden", hidden=True, children=[leaf("under-hide")]),
        _placed(_mesh(5), "linked", tag="walls", children=[leaf("untagged", tag="Src" + LINK_UNTAGGED_SUFFIX)]),
        leaf("sprite", billboard="mesh"),
        _placed(Mesh(), "empty"),
    ])
    for name, visible, locked in (("walls", True, False), ("off", False, False), ("lock", True, True)):
        layer = Layer(name, visible=visible, locked=locked)
        scene.layers.append(layer)
    # The children carry tag keys: point them at the new tags.
    keys = {ly.name: ly.key for ly in scene.layers}
    for g in scene.groups:
        stack = [g]
        while stack:
            h = stack.pop()
            if h.layer in keys:
                h.layer = keys[h.layer]
            stack.extend(h.children or ())
    return scene


@pytest.mark.parametrize("by_layer", [False, True])
def test_native_rows_match_the_proxy_walk(by_layer):
    scene = _scene()
    scene.display_style.color_by_layer = by_layer
    state = wgpu_scene._placement_state(scene)
    vp = SimpleNamespace(scene=scene)
    native = list(wgpu_scene._native_records(scene, scene.groups))
    proxied = list(wgpu_scene._proxy_records(scene, scene.groups, SimpleNamespace(scene=scene), state))
    assert [r[0] for r in native] == [r[0] for r in proxied]
    for (g, world, rep, flags, (mesh, billboard, uid)), (_g, world2, proxy, flags2) in zip(native, proxied):
        assert flags == flags2, g.name
        assert (world is None) == (world2 is None) and (world is None or (world == world2).all()), g.name
        assert _paint_of(vp, rep) == _paint_of(vp, proxy), g.name
        assert state(rep)[1] == state(proxy)[1], g.name
        assert mesh is g.mesh and billboard == g.billboard and uid == g.uid, g.name
    assert sum(1 for r in native if r[3] & 1) >= 4         # hidden tag, Hide, below them
    assert len({id(r[2]) for r in native}) < len(native)  # paint stand-ins are shared


def test_an_open_group_keeps_the_proxy_walk():
    scene = _scene()
    scene.begin_group_edit(scene.groups[0])
    state = wgpu_scene._placement_state(scene)
    rows = list(wgpu_scene._placement_records(scene, scene.groups, SimpleNamespace(scene=scene), state))
    assert all(r[4] is None for r in rows)
