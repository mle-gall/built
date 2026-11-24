# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""«Ahora debería haber una opción de invertir cara» (Marco, 2026-09-10).

La había — en el menú Edición — y no la encontró, porque lo habitual es ponerla en
el menú del BOTÓN DERECHO sobre la cara, que es donde uno la busca. Aquí el
menú contextual ofrecía agrupar, ocultar aristas, cortar, copiar y borrar,
pero no invertir.
"""
from __future__ import annotations

import pytest
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import context_menu_build, context_menu_run


def _V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


@pytest.fixture
def ventana(native_host):
    return native_host.window


def _entries(win, selection):
    win.viewport.scene.select(selection)
    return context_menu_build(win, win.view)


def _choose(win, menu, verb):
    assert verb in menu.ids()
    assert context_menu_run(win, win.view, menu.kind, verb, menu.sel, facts=menu.facts)


def _cara(scene):
    return scene.mesh.add_face([_V(0, 0), _V(2, 0), _V(2, 2), _V(0, 2)])


def test_con_una_cara_seleccionada_el_menu_ofrece_invertir(ventana):
    cara = _cara(ventana.viewport.scene)
    assert "reverse" in _entries(ventana, [cara]).ids()


def test_solo_con_aristas_no_aparece(ventana):
    """Invertir una arista no significa nada: la entrada es de caras."""
    cara = _cara(ventana.viewport.scene)
    arista = ventana.viewport.scene.mesh.edges[0]
    assert cara is not None
    textos = _entries(ventana, [arista]).ids()
    assert "reverse" not in textos
    # «Hide» since 2026-09-18: the one entry hides objects and edges alike.
    assert "hide" in textos, "y las aristas siguen con lo suyo"


def test_invertir_desde_el_menu_da_la_vuelta_a_la_cara(ventana):
    """La entrada tiene que hacer el trabajo, no solo estar."""
    scene = ventana.viewport.scene
    cara = _cara(scene)
    antes = cara.normal()
    _choose(ventana, _entries(ventana, [cara]), "reverse")
    assert QVector3D.dotProduct(cara.normal(), antes) < -0.99
    ventana.viewport.history.undo()
    assert QVector3D.dotProduct(cara.normal(), antes) > 0.99


@pytest.mark.parametrize("vertices", [
    [_V(0, 0), _V(2, 0), _V(2, 2), _V(0, 2)],
    [_V(0, 2), _V(2, 2), _V(2, 0), _V(0, 0)],
    [_V(0, 0), _V(0, 2), _V(0, 2, 3), _V(0, 0, 3)],
    [_V(0, 0), _V(2, 0, 2), _V(2, 2, 2), _V(0, 2)],
])
def test_align_view_from_face_menu(ventana, vertices):
    scene = ventana.viewport.scene
    face = scene.mesh.add_face(vertices)
    camera = ventana.viewport.camera
    camera.set_projection("parallel")
    distance = camera.distance
    _choose(ventana, _entries(ventana, [face]), "alignview")
    assert (camera.target - face.centroid()).length() < 1e-5
    assert QVector3D.dotProduct(camera.forward(), face.normal()) < -0.99999
    assert camera.distance == distance
    assert camera.projection_mode() == "parallel"


def test_align_view_only_for_one_face(ventana):
    scene = ventana.viewport.scene
    first = _cara(scene)
    second = scene.mesh.add_face([_V(3, 0), _V(4, 0), _V(4, 1), _V(3, 1)])
    assert "alignview" not in _entries(ventana, [scene.mesh.edges[0]]).ids()
    assert "alignview" not in _entries(ventana, [first, second]).ids()
