"""Plugins discover the public tool modules and hold references across imports."""
import importlib
import pkgutil
import pickle

import pytest
import tools

EXPORTS = {
    "arc": "ArcTool CenterArcTool PieTool ThreePointArcTool COLOR_FILLET commit_arc",
    "change_axes": "ChangeAxesTool",
    "circle": "CircleTool PolygonTool busy_plane flat_drawing",
    "dimension": "DimensionTool",
    "eraser": "EraserTool",
    "fillet": "FilletTool",
    "flip": "FlipTool",
    "followme": "FollowMeTool",
    "freehand": "FreehandTool",
    "geopath": "GeoPathTool",
    "image": "ImageTool",
    "line": "LineTool",
    "move": "MoveTool rotation_grips gather_targets gather_images",
    "north": "NorthTool COMPASS_PX north_deg_towards compass_segments refresh_north_display",
    "offset": "OffsetTool",
    "paint": "PaintTool DEFAULT_FACE_COLOR RECENT_MAX recent_key push_recent rename_recent clicked_back_side",
    "paste": "PasteTool",
    "place_group": "PlaceGroupTool",
    "protractor": "ProtractorTool ProtractorBase DISC_PX TICK_DEG",
    "rectangle": "RectangleTool",
    "rotate": "RotateTool",
    "rotated_rectangle": "RotatedRectangleTool",
    "scale": "ScaleTool",
    "scale_manager": "ScaleManagerTool",
    "section": "SectionPlaneTool",
    "select": "SelectTool selection_mode delete_selection_or_hover erase_entities expand_pick",
    "solid_tools": "SolidTool OuterShellTool UnionTool SubtractTool TrimTool IntersectTool SplitTool SOLID_TOOLS",
    "tape": "TapeMeasureTool",
    "text": "TextTool",
    "walkthrough": "PositionCameraTool LookAroundTool WalkTool FirstPersonTool DEFAULT_EYE_HEIGHT DEFAULT_LOOK_SENSITIVITY eye_height set_eye_height look_sensitivity look_deg_per_px",
}


@pytest.mark.parametrize("name", EXPORTS)
def test_tool_module_discovery_and_reexports(name):
    names = {entry.name for entry in pkgutil.iter_modules(tools.__path__)}
    assert name in names
    module = importlib.import_module(f"tools.{name}")
    saved = {key: getattr(module, key) for key in EXPORTS[name].split()}
    importlib.invalidate_caches()
    assert importlib.reload(module) is module
    assert getattr(tools, name) is module
    for key, value in saved.items():
        assert getattr(module, key) is value
        if isinstance(value, type):
            assert pickle.loads(pickle.dumps(value)) is value


def test_native_host_modules_preserve_public_class_and_subclass_binding():
    from tools.flip_host import FlipHost
    from tools.paste_host import PasteHost
    from tools.texture_position_metal import TexturePreview
    from tools.rust_tools import FlipTool, PasteTool

    assert issubclass(FlipTool, FlipHost)
    assert issubclass(PasteTool, PasteHost)
    for cls in (FlipHost, PasteHost, TexturePreview):
        assert pickle.loads(pickle.dumps(cls)) is cls
    view = object()
    assert TexturePreview(view=view).view is view


def test_native_tool_signatures_keep_evaluated_and_postponed_annotations():
    import inspect
    from built_core.mesh import Edge
    from tools.offset_host import _run_from_edge
    from tools.paint_host import recent_key
    from tools.walkthrough import look_deg_per_px
    from tools.scale_host import _Grip

    assert inspect.signature(_run_from_edge).parameters["edge"].annotation is Edge
    assert inspect.signature(recent_key).return_annotation is tuple
    assert inspect.signature(look_deg_per_px).parameters["sensitivity"].annotation == int | None
    assert inspect.signature(_Grip.kind).return_annotation == "str"


def test_registered_callback_sees_the_current_host_module(monkeypatch):
    import sys
    from types import ModuleType, SimpleNamespace
    from tools.rust_tool_base import RustTool

    bridge = ModuleType("tools.rust_tool_base")
    bridge._mods = lambda value: 17
    bridge._tuple = lambda value: (3, 2, 1)
    monkeypatch.setitem(sys.modules, bridge.__name__, bridge)
    event = RustTool._pointer("hover", SimpleNamespace())
    assert (event["mods"], event["world"]) == (17, (3, 2, 1))


@pytest.mark.parametrize(("module_name", "class_name"), [
    ("eraser_host", "EraserHost"), ("fillet_host", "FilletHost"),
    ("followme_host", "FollowMeHost"), ("geopath_host", "GeoPathTool"),
    ("image_host", "ImageTool"), ("move_host", "MoveHost"),
    ("north_host", "NorthTool"), ("paint_host", "PaintHost"),
    ("place_group_host", "PlaceGroupHost"), ("pushpull", "PushPullTool"),
    ("rotate_host", "RotateHost"), ("rust_tools", "SelectTool"),
    ("scale_host", "ScaleHost"), ("section_host", "SectionPlaneTool"),
    ("solid_host", "OuterShellTool"), ("tape_host", "TapeHost"),
])
def test_host_class_keeps_its_plugin_import_path(module_name, class_name):
    path = f"tools.{module_name}"
    discovered = {entry.name for entry in pkgutil.iter_modules(tools.__path__, "tools.")}
    assert path in discovered
    cls = getattr(importlib.import_module(path), class_name)
    assert (cls.__module__, cls.__name__) == (path, class_name)
    assert pickle.loads(pickle.dumps(cls)) is cls


def test_plugin_tool_contract_and_context_roundtrip():
    from dataclasses import fields, replace
    from tools.base import Tool, ToolContext

    assert Tool.__abstractmethods__ == {"on_activate", "on_deactivate"}
    class Extension(Tool):
        def on_activate(self, viewport):
            pass
        def on_deactivate(self, viewport):
            pass
        def on_click(self, ctx):
            self.last = ctx

    tool = Extension()
    ctx = ToolContext(None, None, None, 0, None)
    assert [f.name for f in fields(ctx)] == ["viewport", "world", "screen", "modifiers", "snap"]
    assert replace(ctx, modifiers=1).modifiers == 1
    assert pickle.loads(pickle.dumps(ctx)) == ctx
    tool.on_double_click(ctx)
    assert tool.last is ctx
    assert tool.preview_faces() is not tool.preview_faces()
    assert tool.ctrl_tapped(object()) is True
