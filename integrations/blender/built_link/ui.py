# SPDX-License-Identifier: GPL-3.0-or-later
"""The Built panel in the 3D Viewport sidebar, its operators and the timer."""
import bpy

from . import sync
from .blender_scene import Host
from .controller import Controller

_controller = None
TICK = 0.1


def controller():
    global _controller
    if _controller is None:
        _controller = Controller(Host())
    return _controller


def _prefs():
    addon = bpy.context.preferences.addons.get(__package__)
    return addon.preferences if addon else None


def _kinds(wm):
    return tuple(k for k, on in (("material", wm.built_send_materials), ("light", wm.built_send_lights),
                                 ("camera", wm.built_send_cameras), ("transform", wm.built_send_transforms)) if on)


def _tick():
    c = _controller
    if c is None:
        return None
    try:
        handled = c.tick()
    except Exception as exc:  # a bad message must not stop the timer
        c.notice = f"Link error: {exc}"
        handled = 1
    wm = bpy.context.window_manager
    if c.state == "linked" and wm.built_auto_send and not c.progress:
        c.send(_kinds(wm))
    if handled:
        for window in wm.windows:
            for area in window.screen.areas:
                if area.type == "VIEW_3D":
                    area.tag_redraw()
    return TICK


def start_timer():
    if not bpy.app.timers.is_registered(_tick):
        bpy.app.timers.register(_tick, first_interval=TICK, persistent=True)


def link(pairing, relink=False):
    c = controller()
    c.link(pairing, relink=relink)
    prefs = _prefs()
    if prefs is not None:
        prefs.pairing = pairing
    start_timer()


class BUILT_OT_link(bpy.types.Operator):
    """Link this file to the Built model that shows the pairing code"""
    bl_idname = "built.link"
    bl_label = "Link to Built"

    pairing: bpy.props.StringProperty(name="Pairing code", description="Copied from Built's Render drawer")
    relink: bpy.props.BoolProperty(default=False, options={"HIDDEN"})

    def invoke(self, context, event):
        clip = context.window_manager.clipboard.strip()
        prefs = _prefs()
        self.pairing = clip if clip.startswith("built:") else (prefs.pairing if prefs else "")
        return context.window_manager.invoke_props_dialog(self)

    def execute(self, context):
        try:
            link(self.pairing, self.relink)
        except ValueError as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


class BUILT_OT_relink(bpy.types.Operator):
    """Link this file to the model open in Built instead of the one it was linked to"""
    bl_idname = "built.relink"
    bl_label = "Link to this model instead"

    def execute(self, context):
        c = controller()
        if c.client.port is None:
            return {"CANCELLED"}
        c.client.start(c.client.port, c.client.token, c.hello(), relink=True)
        start_timer()
        return {"FINISHED"}


class BUILT_OT_unlink(bpy.types.Operator):
    """Stop the link (the model stays in this file)"""
    bl_idname = "built.unlink"
    bl_label = "Unlink"

    def execute(self, context):
        controller().unlink()
        return {"FINISHED"}


class BUILT_OT_send(bpy.types.Operator):
    """Send the changed materials, lights, cameras and placements to Built for review"""
    bl_idname = "built.send"
    bl_label = "Send to Built"

    def execute(self, context):
        n = controller().send(_kinds(context.window_manager))
        self.report({"INFO"}, f"{n} change{'s' if n != 1 else ''} sent to Built" if n else "Nothing changed")
        return {"FINISHED"}


class VIEW3D_PT_built_link(bpy.types.Panel):
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Built"
    bl_label = "Built Link"

    def draw(self, context):
        layout = self.layout
        c = controller()
        wm = context.window_manager
        col = layout.column(align=True)
        icon = {"linked": "LINKED", "refused": "ERROR"}.get(c.state, "UNLINKED")
        col.label(text=c.status(), icon=icon)
        if c.state == "linked":
            col.label(text="Geometry from Built · automatic")
        if c.notice and c.state != "linked":
            col.label(text=c.notice)
        row = layout.row(align=True)
        if c.state in ("linked", "connecting", "waiting"):
            row.operator(BUILT_OT_unlink.bl_idname)
        else:
            row.operator(BUILT_OT_link.bl_idname)
        if c.state == "refused" and c.refused_reason == "document":
            layout.operator(BUILT_OT_relink.bl_idname)
        box = layout.box()
        box.label(text="Send to Built")
        grid = box.grid_flow(columns=2, align=True)
        grid.prop(wm, "built_send_materials", toggle=True)
        grid.prop(wm, "built_send_lights", toggle=True)
        grid.prop(wm, "built_send_cameras", toggle=True)
        grid.prop(wm, "built_send_transforms", toggle=True)
        if c.state == "linked":
            items, _ = c.changes(_kinds(wm))
            row = box.row(align=True)
            row.enabled = bool(items)
            label = f"Send {len(items)} change{'s' if len(items) != 1 else ''}" if items else "No changes"
            row.operator(BUILT_OT_send.bl_idname, text=label)
            box.prop(wm, "built_auto_send")
        box.label(text="Built reviews each change before applying it")


class BuiltLinkPreferences(bpy.types.AddonPreferences):
    bl_idname = __package__
    pairing: bpy.props.StringProperty(name="Last pairing code")

    def draw(self, context):
        self.layout.prop(self, "pairing")


CLASSES = (BuiltLinkPreferences, BUILT_OT_link, BUILT_OT_relink, BUILT_OT_unlink, BUILT_OT_send, VIEW3D_PT_built_link)
TOGGLES = (("built_send_materials", "Materials"), ("built_send_lights", "Lights"),
           ("built_send_cameras", "Cameras"), ("built_send_transforms", "Transforms"))


def register():
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    for attr, label in TOGGLES:
        setattr(bpy.types.WindowManager, attr, bpy.props.BoolProperty(name=label, default=True))
    bpy.types.WindowManager.built_auto_send = bpy.props.BoolProperty(
        name="Send automatically", default=False, description="Send changes as they happen")


def unregister():
    global _controller
    if _controller is not None:
        _controller.unlink()
        _controller = None
    if bpy.app.timers.is_registered(_tick):
        bpy.app.timers.unregister(_tick)
    for attr, _ in TOGGLES:
        delattr(bpy.types.WindowManager, attr)
    del bpy.types.WindowManager.built_auto_send
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)


__all__ = ["register", "unregister", "link", "controller", "sync"]
