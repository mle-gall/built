# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Create group over a selection that is not plain loose geometry.

Reported as "I press Create group and it doesn't create": every group Marco
made put another group in the drawing, so his next rubber-band selection
included one, and the action refused — into the status bar, for five
seconds, while he was looking at the model. An empty selection returned in
total silence. Both now answer.
"""
from __future__ import annotations

import pytest
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import context_menu_build

from core.group import Group
from core.mesh import Mesh


def V(x, y, z=0.0):
    return QVector3D(x, y, z)


@pytest.fixture
def win(native_host):
    return native_host.window


def _quad(mesh, x0=0.0, y0=0.0):
    return mesh.add_face([V(x0, y0), V(x0 + 2, y0), V(x0 + 2, y0 + 2),
                          V(x0, y0 + 2)])


def _new_groups(scene, before):
    """Groups added since *before* — a fresh document already ships the
    startup template's scale figure, which is not what these tests measure."""
    known = {id(g) for g in before}
    return [g for g in scene.groups if id(g) not in known]


def test_empty_selection_says_so_instead_of_going_silent(win):
    scene = win.viewport.scene
    scene.select([])
    win.statusBar().clearMessage()
    before = len(scene.groups)
    win._on_make_group()
    assert len(scene.groups) == before
    assert win.statusBar().currentMessage(), "the silent no-op is the bug"


def test_plain_loose_geometry_still_groups(win):
    scene = win.viewport.scene
    f = _quad(scene.mesh)
    scene.select([f])
    before = len(scene.groups)
    win._on_make_group()
    assert len(scene.groups) == before + 1


def test_agrupar_un_grupo_lo_ANIDA(win):
    """Lo que antes era un diálogo pidiendo perdón. Un grupo puede contener
    grupos desde el 2026-09-11, así que agrupar hace lo que dice."""
    scene = win.viewport.scene
    g1 = Group(Mesh(), name="Banca")
    _quad(g1.mesh, 10.0)
    g2 = Group(Mesh(), name="Pérgola")
    _quad(g2.mesh, 14.0)
    scene.groups += [g1, g2]
    scene.select([g1, g2])
    previos = list(scene.groups)
    win._on_make_group()
    assert g1 not in scene.groups and g2 not in scene.groups
    nuevo = _new_groups(scene, previos)
    assert len(nuevo) == 1
    padre = nuevo[0]
    assert padre.children == [g1, g2], "los grupos son HIJOS, no se fundieron"
    assert len(g1.mesh.faces) == 1 and len(g2.mesh.faces) == 1
    assert padre.xform is not None, "un contenedor es siempre instancia"


def test_lo_suelto_de_la_selección_va_a_la_malla_del_contenedor(win):
    """Como es lo habitual: dentro del grupo nuevo encuentras las caras sueltas Y
    el grupo, cada uno como lo que era."""
    scene = win.viewport.scene
    f = _quad(scene.mesh)
    g = Group(Mesh(), name="Banca")
    _quad(g.mesh, 10.0)
    scene.groups.append(g)
    scene.select([f, g])
    previos = list(scene.groups)
    sueltas_antes = len(scene.mesh.faces)
    win._on_make_group()
    nuevo = _new_groups(scene, previos)
    assert len(nuevo) == 1
    padre = nuevo[0]
    assert padre.children == [g]
    assert len(padre.mesh.faces) == 1, "la cara suelta es la malla del padre"
    assert len(scene.mesh.faces) == sueltas_antes - 1
    assert len(g.mesh.faces) == 1, "la banca sigue entera"


def test_anidar_se_deshace_en_UN_paso(win):
    scene = win.viewport.scene
    f = _quad(scene.mesh)
    g = Group(Mesh(), name="Banca")
    _quad(g.mesh, 10.0)
    scene.groups.append(g)
    scene.select([f, g])
    antes = [x.name for x in scene.groups]
    sueltas = len(scene.mesh.faces)
    win._on_make_group()
    assert win.viewport.history.undo() is True
    assert [x.name for x in scene.groups] == antes
    assert len(scene.mesh.faces) == sueltas
    assert g.children == [], "y la banca no se queda adoptada"


def test_un_grupo_con_hijos_se_anida_sin_perderlos(win):
    """Anidar un contenedor dentro de otro: el árbol crece, no se aplana.
    (Junto a otro grupo: uno solo ya no se agrupa consigo mismo, #35.)"""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    scene = win.viewport.scene
    hondo = Mesh()
    _quad(hondo, 0.0)
    hijo = Group(hondo, name="Tabla")
    hijo.xform = QMatrix4x4()
    padre = Group(Mesh(), name="Banca")
    _quad(padre.mesh, 20.0)
    padre.adopt([hijo])
    otro = Group(Mesh(), name="Pérgola")
    _quad(otro.mesh, 40.0)
    scene.groups += [padre, otro]
    scene.select([padre, otro])
    previos = list(scene.groups)
    win._on_make_group()
    nuevo = _new_groups(scene, previos)
    assert len(nuevo) == 1
    abuelo = nuevo[0]
    assert abuelo.children == [padre, otro]
    assert padre.children == [hijo], "el nieto sigue ahí"
    assert len(hijo.mesh.faces) == 1


def test_el_menu_del_boton_derecho_ofrece_crear_grupo_con_solo_grupos(win):
    """«Seleccioné cuatro grupos y solo me sale crear componente y unir
    grupos» (Marco, 2026-09-11). La entrada estaba condicionada a que
    hubiera geometría suelta, porque agrupar grupos no existía."""
    scene = win.viewport.scene
    for i in range(4):
        g = Group(Mesh(), name="G%d" % i)
        _quad(g.mesh, 10.0 + 4 * i)
        scene.groups.append(g)
    scene.select(list(scene.groups))

    verbs = context_menu_build(win).ids()
    assert "makegroup" in verbs
    assert "merge" in verbs


def test_un_grupo_solo_no_se_agrupa_consigo_mismo(win):
    """@pacaeiro, issue #35: «RMB click on the group and select > create
    group (shouldn't be possible); result, a group inside of a group that
    is equal to itself». Uno solo, sin nada más: no hay con qué agruparlo
    — y se dice, en vez de fabricar la muñeca rusa. Con otro grupo, o con
    geometría suelta, sigue agrupando (los tests de arriba)."""
    scene = win.viewport.scene
    g = Group(Mesh(), name="Banca")
    _quad(g.mesh, 10.0)
    scene.groups.append(g)
    scene.select([g])
    win.statusBar().clearMessage()
    previos = list(scene.groups)
    win._on_make_group()
    assert scene.groups == previos and not g.children
    assert win.statusBar().currentMessage()


def test_el_menu_no_ofrece_agrupar_a_un_grupo_solo(win):
    """La entrada del clic derecho sigue la misma regla: uno solo no la
    tiene; dos grupos, o un grupo con geometría suelta, sí."""
    scene = win.viewport.scene
    g1 = Group(Mesh(), name="Banca")
    _quad(g1.mesh, 10.0)
    g2 = Group(Mesh(), name="Pérgola")
    _quad(g2.mesh, 14.0)
    scene.groups += [g1, g2]

    def entradas(sel):
        scene.select(sel)
        return context_menu_build(win).ids()
    assert "makegroup" not in entradas([g1])
    assert "makegroup" in entradas([g1, g2])
    f = _quad(scene.mesh)
    assert "makegroup" in entradas([g1, f])
