"""Pre-port gestures for the remaining modelling tools, for tool_parity.record."""
from unittest.mock import patch

from core.scene import Scene
from tests.test_parity_tools import _Vp, _ctx, V


CASES = {
    "north": {"aim": ["click", "hover", "end", "undo"], "typed_cancel": ["typed", "undo", "click", "cancel"]},
    "image": {"typed": ["load", "click", "hover", "typed", "undo"],
              "corners": ["load", "click", "hover", "end", "undo"], "cancel": ["load", "click", "cancel"]},
    "section": {"locked": ["hover", "right", "end", "undo"], "ground": ["hover", "down", "end", "undo"]},
    "dimension": {"place": ["click", "hover", "end", "offset", "place", "undo"],
                  "degenerate": ["click", "click", "hover", "cancel"]},
    "text": {"place": ["click", "hover", "end", "undo"], "cancel": ["click", "hover", "cancel"]},
    "geopath": {"typed": ["click", "hover", "typed", "enter", "undo"],
                "closed": ["click", "end", "place", "click", "undo"], "cancel": ["click", "end", "cancel"]},
    "place_group": {"place": ["hover", "end", "undo"], "abandon": ["hover", "deactivate", "end"]},
    "scale_manager": {"missing": ["activate", "typed"], "open": ["arm", "activate", "typed", "deactivate"]},
    "eraser": {"stroke": ["click", "hover", "release", "undo"], "cancel": ["click", "cancel", "release"]},
}


def run(kind, steps):
    from ingetrazo_core import Input as Qt
    from tools.dimension import DimensionTool
    from tools.eraser import EraserTool
    from tools.geopath import GeoPathTool
    from tools.image import ImageTool
    from tools.north import NorthTool
    from tools.place_group import PlaceGroupTool
    from tools.scale_manager import ScaleManagerTool
    from tools.section import SectionPlaneTool
    from tools.text import TextTool
    from tests.test_solid_tools import _pair
    from tests.tool_parity import _pt
    types = dict(dimension=DimensionTool, eraser=EraserTool, geopath=GeoPathTool,
                 image=ImageTool, north=NorthTool, section=SectionPlaneTool,
                 text=TextTool, scale_manager=ScaleManagerTool)
    scene = Scene()
    vp = _Vp(scene)
    group = _pair()[0] if kind in ("place_group", "scale_manager") else None
    tool = PlaceGroupTool(group) if kind == "place_group" else types[kind]()
    vp.window = lambda: None
    vp.drape = lambda point: point
    vp._world_to_pixel = lambda p: (100 * p.x(), 100 * p.y())
    vp.ground_elevation = lambda x, y: .1 * x
    vp.has_ground_surface = lambda: False
    vp.pick_edge = lambda x, y: None
    vp.pick_face_any = lambda x, y: (None, None)
    if kind == "eraser":
        edges = [scene.mesh.add_edge(V(i, 0, 0), V(i + 1, 0, 0)) for i in range(2)]
        vp.pick_edge = lambda x, y: edges[int(x != 0)]
    dialogs = []
    call_dialog = lambda *args, **kwargs: kwargs["done"]("Label")
    schedule = lambda owner, delay, callback: callback()
    with patch("ingetrazo_render.chrome_forms.ask_text", side_effect=call_dialog), \
         patch("core.timers.later", side_effect=schedule), \
         patch.object(ScaleManagerTool, "_run", staticmethod(lambda vp, g: dialogs.append(g.name))):
        if kind != "scale_manager":
            tool.on_activate(vp)
        out = []
        for step in steps:
            result = None
            if step in ("click", "end", "hover", "offset", "place"):
                xy = {"click": (0, 0), "end": (2, 1), "hover": (2, 1), "offset": (1, 3), "place": (1, 3)}
                ctx = _ctx(vp, *xy[step])
                (tool.on_hover if step in ("hover", "offset") else tool.on_click)(ctx)
            elif step == "undo":
                vp.history.undo()
            elif step in ("enter", "right", "down"):
                key = {"enter": Qt.Key_Return, "right": Qt.Key_Right, "down": Qt.Key_Down}[step]
                result = tool.on_key(vp, key, Qt.NoModifier)
            elif step == "typed":
                result = tool.on_value(vp, 2.5)
            elif step == "load":
                tool.load("drawing.png", .5, "Drawing")
            elif step == "arm":
                tool.target = group
            else:
                getattr(tool, "on_" + step)(vp)
            out.append({"result": result, "undo": len(vp.history.undo_stack), "dialogs": list(dialogs),
                        "band": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()],
                        "groups": len(scene.groups), "edges": len(scene.mesh.edges),
                        "dimensions": [[_pt(d.a), _pt(d.b), _pt(d.offset), d.axis] for d in scene.dimensions],
                        "texts": [[_pt(t.anchor), _pt(t.offset), t.text] for t in scene.text_labels],
                        "paths": [[[_pt(p) for p in g.points], g.closed] for g in scene.geo_paths],
                        "images": [[_pt(i.origin), _pt(i.u), _pt(i.v)] for i in scene.image_planes],
                        "sections": [[_pt(s.point), _pt(s.normal)] for s in scene.section_planes],
                        "north": round(scene.north_deg, 6)})
    return out


def solid_cases():
    from tools.solid_tools import SOLID_TOOLS
    return {kind + "_" + mode: (tool_type, mode) for kind, tool_type in SOLID_TOOLS
            for mode in ("click", "preselected")}


def run_solid(tool_type, mode):
    from core.solids import solid_volume
    from tests.test_solid_tools import _VP, _click, _pair
    scene = Scene()
    a, b = _pair()
    scene.groups.extend([a, b])
    vp = _VP(scene, [a, b])
    tool = tool_type()
    if mode == "preselected":
        scene.select([a, b])
    out = []
    for step in ("activate", "first", "second", "undo"):
        if step == "activate":
            tool.on_activate(vp)
        elif step == "undo":
            vp.history.undo()
        elif not vp.history.undo_stack:
            _click(tool, vp)
        out.append({"volumes": [round(solid_volume(g), 6) for g in scene.groups],
                    "busy": tool.dragging, "badge": tool.solid_cursor_key,
                    "undo": len(vp.history.undo_stack), "selection": len(scene.selection)})
    return out
