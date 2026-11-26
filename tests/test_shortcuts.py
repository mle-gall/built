# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Atajos compartidos por el menú, la paleta y los controles de vista."""
from collections import defaultdict
from unittest.mock import Mock, patch

from built_core import Input as Qt
from built_render import shortcut_text
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_shortcuts_transfer import ventana  # noqa: F401


def _todos_los_atajos(win):
    por_tecla = defaultdict(list)
    for action in win._registry_actions.values():
        for tecla in action.portable_keys():
            por_tecla[tecla].append(action.text())
    return por_tecla


def _pulsar(win, tecla, modificador=Qt.NoModifier):
    from built_render import ShellAction
    disparos = []
    trigger = ShellAction.trigger
    def observar(action):
        disparos.append(action.text())
        trigger(action)
    with patch.object(ShellAction, "trigger", observar):
        win._host.key(True, tecla, modificador.value)
        win._host.key(False, tecla, modificador.value)
    return disparos


def test_ninguna_accion_pisa_el_atajo_de_otra(ventana):
    chocan = {tecla: nombres
              for tecla, nombres in _todos_los_atajos(ventana).items()
              if len(nombres) > 1}
    assert not chocan, (
        "atajos compartidos — Qt los deja MUERTOS a todos: "
        + "; ".join(f"{t} -> {n}" for t, n in sorted(chocan.items())))


def test_las_teclas_de_camara_son_las_habituales(ventana):
    """O / H / Z / Shift+Z, tal cual la tarjeta de referencia habitual."""
    for tecla, modificador, esperado in (
            (Qt.Key_O, Qt.NoModifier, "Orbit"),
            (Qt.Key_H, Qt.NoModifier, "Pan"),
            (Qt.Key_Z, Qt.NoModifier, "Zoom"),
            (Qt.Key_Z, Qt.ShiftModifier, "Zoom Extents"),
            (Qt.Key_F2, Qt.NoModifier, "Zoom Extents"),   # el de toda la vida
    ):
        assert _pulsar(ventana, tecla, modificador) == [esperado]


def test_las_dos_herramientas_que_cedieron_siguen_a_mano(ventana):
    """Ceder la tecla no puede dejarlas sin ninguna."""
    assert _pulsar(ventana, Qt.Key_H, Qt.ShiftModifier) == ["Protractor"]
    assert _pulsar(ventana, Qt.Key_O, Qt.ShiftModifier) == ["Center Arc"]


def test_redondear_3d_va_con_mayus_f_y_la_f_sigue_siendo_equidistancia(ventana):
    """Redondear 3D no tenía tecla. La F es de Equidistancia en la tarjeta,
    así que Redondear toma Mayús+F, como las otras que llegaron después."""
    assert _pulsar(ventana, Qt.Key_F, Qt.ShiftModifier) == ["Fillet 3D"]
    assert ventana.viewport.active_tool is ventana._tools["fillet"]
    assert _pulsar(ventana, Qt.Key_F) == ["Offset"]
    assert ventana.viewport.active_tool is ventana._tools["offset"]


#: Lo que la tarjeta de referencia habitual SÍ trae atado de fábrica y
#: nosotros respetamos (edición 2026). Lo que falta de esa lista es
#: deliberado y está anotado abajo; lo que sobra son teclas que allá están libres.
TARJETA_HABITUAL = {
    "Space": "Select", "B": "Paint", "E": "Eraser", "L": "Line",
    "R": "Rectangle", "C": "Circle", "A": "Arc", "M": "Move",
    "Q": "Rotate", "S": "Scale", "F": "Offset", "T": "Tape Measure",
    "O": "Orbit", "H": "Pan", "Z": "Zoom", "Shift+Z": "Zoom Extents",
    "G": "Make Component…", "P": "Push / Pull",
}
# Divergencias a propósito, para que nadie las "arregle" sin decidirlo:
#   U       = Empujar/Tirar   (segundo atajo del MISMO comando: la tecla que
#                              Built usó hasta el 2026-09-10)
#   Mayús+P = Perspectiva/paralela (la tarjeta no le da tecla ninguna)
#   K       = Rectángulo rotado    (allá: aristas traseras; el rotado no
#                                   tiene tecla allá)
#   F2      = Zoom a extensión     (segundo atajo, además de Shift+Z)
#   D J W X Y = Cota, Arco 3 puntos, Sígueme, Texto, Ruta — sin tecla allá.


def test_respetamos_la_tarjeta_habitual(ventana):
    atajos = _todos_los_atajos(ventana)
    for tecla, nombre in TARJETA_HABITUAL.items():
        assert atajos.get(tecla) == [nombre], (
            f"{tecla} debería ser {nombre!r} como en la tarjeta, "
            f"y tiene {atajos.get(tecla)}")


def test_el_arco_de_A_es_el_de_comba(ventana):
    """La tarjeta ata A al «2 Point Arc» (cuerda y comba). El nuestro se llama
    solo «Arc», y es ese: su cuadro pide la comba."""
    from tools.arc import ArcTool
    assert ArcTool.shortcut == "A"
    assert ArcTool.vcb_label == "Bulge"


def test_la_p_es_empujar_tirar_y_la_u_sigue_valiendo(ventana):
    """La P es Empujar/Tirar (Marco, 2026-09-10). La U no se tira: es el mismo
    comando con dos atajos, que es lo legal — dos ACCIONES con un atajo es
    lo que Qt mata."""
    assert _pulsar(ventana, Qt.Key_P) == ["Push / Pull"]
    assert _pulsar(ventana, Qt.Key_U) == ["Push / Pull"]
    atajos = _todos_los_atajos(ventana)
    assert atajos["P"] == ["Push / Pull"] and atajos["U"] == ["Push / Pull"]


def test_la_perspectiva_se_mudo_a_mayus_p(ventana):
    assert _pulsar(ventana, Qt.Key_P, Qt.ShiftModifier) == [
        "Toggle Perspective / Parallel"]


def test_la_barra_muestra_el_atajo_configurado_y_no_el_de_fabrica(ventana, monkeypatch):
    from views import wgpu_ui
    view = ventana.view
    monkeypatch.setattr(wgpu_ui, "_icon_rgba", lambda _key: (1, 1, b"\0" * 4))
    renderer = Mock()
    monkeypatch.setattr(view, "renderer", renderer)
    view._ui_tools_sent = False  # initial state after the renderer attaches
    line = ventana._tool_actions["line"]
    for keys in (["L"], ["Ctrl+Alt+L"], [], ["L"]):
        line.setShortcuts(keys)
        assert not view._ui_tools_sent
        wgpu_ui._send_tools(view, ventana)
        groups = renderer.ui_set_tools.call_args.args[0]
        row = next(row for group in groups for row in group if row[0] == "line")
        assert row[2] == (shortcut_text(keys[0])[1] if keys else "")


#: Vistas estándar por comodidad (Marco, 2026-09-30): Ctrl con el meñique y
#: los dedos en 1-4 sin mover la mano — planta, alzado, lateral y 3D; las
#: raras quedan lejos. Los dígitos solos escriben la medida en la casilla.
VISTAS = {
    Qt.Key_1: "Top", Qt.Key_2: "Front", Qt.Key_3: "Right",
    Qt.Key_4: "Isometric", Qt.Key_5: "Back", Qt.Key_6: "Left",
    Qt.Key_7: "Bottom",
}


def test_ctrl_numero_pone_la_vista_estandar(ventana, monkeypatch):
    pedidas = []
    monkeypatch.setattr(ventana, "_on_standard_view", pedidas.append)
    for tecla, nombre in VISTAS.items():
        assert _pulsar(ventana, tecla, Qt.ControlModifier) == [nombre]
    assert pedidas == ["top", "front", "right", "iso", "back", "left",
                       "bottom"]


def test_menu_y_barra_comparten_la_accion_de_cada_vista(ventana):
    from built_render import command_registry
    for i, name in enumerate(("top", "front", "right", "iso", "back", "left", "bottom"), 1):
        menu_id = f"view:{name}" if i <= 4 else f"view_{name}"
        menu = [(menu_id, "Camera", name.title(), f"Ctrl+{i}")]
        rows = command_registry([], menu)
        assert [row[0] for row in rows if row[5] == menu_id] == [f"view:view_{name}"]
        assert ventana._view_bindings[f"view_{name}"][0] == shortcut_text(f"Ctrl+{i}")[1]


def test_sidebar_shortcuts_use_the_registered_actions(ventana):
    for i, name in enumerate(("project", "tasks", "deliveries"), 1):
        action = ventana._registry_actions[f"view:sidebar_{name}"]
        assert action.portable_keys() == [f"Ctrl+Alt+{i}"]


def test_el_boton_de_vista_muestra_su_atajo(ventana):
    top = ventana._registry_actions["view:view_top"]
    top.setShortcuts(["Ctrl+Alt+1"])
    assert ventana._view_bindings["view_top"] == (shortcut_text("Ctrl+Alt+1")[1], True)


def _caja(nombre: str, x0: float):
    """Un grupo cúbico de lado 1 generado en el momento."""
    from built_core import Vec3 as QVector3D
    from built_core.group import Group
    from built_core.mesh import Mesh
    malla = Mesh()
    P = [QVector3D(x0 + (i & 1), (i >> 1) & 1, i >> 2) for i in range(8)]
    for q in ([0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3],
              [0, 4, 6, 2], [1, 3, 7, 5]):
        malla.add_face([P[i] for i in q])
    return Group(malla, name=nombre)


def test_ocultar_es_ctrl_h_y_mostrar_todo_ctrl_mayus_h(ventana):
    """Ocultar y Mostrar ▸ Todo se usan sin parar y no tenían tecla. La H
    sola es Encuadre y Mayús+H el Transportador, así que van con Ctrl."""
    atajos = _todos_los_atajos(ventana)
    assert atajos["Ctrl+H"] == ["Hide"]
    assert atajos["Ctrl+Shift+H"] == ["All"]
    assert _pulsar(ventana, Qt.Key_H, Qt.ControlModifier) == ["Hide"]
    assert _pulsar(ventana, Qt.Key_H,
                   Qt.ControlModifier | Qt.ShiftModifier) == ["All"]
    # Las dos que ya tenían la H no se mueven.
    assert _pulsar(ventana, Qt.Key_H) == ["Pan"]
    assert _pulsar(ventana, Qt.Key_H, Qt.ShiftModifier) == ["Protractor"]


def test_ctrl_h_oculta_la_seleccion_y_ctrl_mayus_h_lo_devuelve_todo(ventana):
    vp = ventana.viewport
    cajas = [_caja(f"caja{i}", 2.0 * i) for i in range(3)]
    vp.scene.groups += cajas
    vp.scene.version += 1

    for caja in cajas[:2]:                  # dos ocultaciones separadas
        vp.scene.select([caja])
        _pulsar(ventana, Qt.Key_H, Qt.ControlModifier)
    assert [c.hidden for c in cajas] == [True, True, False]

    _pulsar(ventana, Qt.Key_H, Qt.ControlModifier | Qt.ShiftModifier)
    assert not any(c.hidden for c in cajas)
    vp.history.undo()                       # Mostrar todo es un solo paso
    assert [c.hidden for c in cajas] == [True, True, False]
