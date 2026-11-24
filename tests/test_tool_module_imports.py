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
