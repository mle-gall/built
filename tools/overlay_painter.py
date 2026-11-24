"""Submit a tool's overlay to the document's native annotation painter."""


def draw_overlay(tool, viewport, painter):
    return painter.draw_tool(tool.overlay_kind, tool, viewport)
