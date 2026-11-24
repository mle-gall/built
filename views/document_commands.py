"""Bind the Rust document commands to each window's active Python controller."""
import ingetrazo_render as native
from core.i18n import tr


def _repeatable(label: str):
    """Mark a one-shot command as the one Repeat (Shift+R) replays.

    Blender's Shift+R: after Reverse Faces on one face, select the next and
    repeat. Recorded on the SLOT, not on a
    QAction, because the right-click menu builds its actions afresh each
    time — a remembered context-menu QAction would be a dead object by the
    next repeat. Arguments the command takes (Intersect's mode) are kept;
    the ``checked`` flag Qt may append to a menu's ``triggered`` is not."""
    import functools
    import inspect

    def deco(fn):
        takes = len(inspect.signature(fn).parameters) - 1    # minus self

        @functools.wraps(fn)
        def wrapper(self, *args):
            args = args[:takes]
            self._remember_command(label, lambda: fn(self, *args))
            return fn(self, *args)
        return wrapper
    return deco


class WindowDocumentCommands:
    # Keep full collections off the pointer-move path in large documents.
    GC_THRESHOLD0 = 50_000
    _RECENT_MAX = 10
    _CAMERA_FIELDS = native.DOCUMENT_CAMERA_FIELDS

    @property
    def viewport(self):
        doc = self.__dict__.get("_doc")
        if doc is None:
            return None
        return doc.front_view if doc.front_view is not None else doc.viewport

    @viewport.setter
    def viewport(self, view):
        native.set_front_view(self, view)

    @_repeatable("Make Unique")
    def _on_make_unique(self):
        return native.unique_components(self)

    @_repeatable("Merge Groups")
    def _on_merge_groups(self):
        return native.merge_groups(self)

    @_repeatable("Explode Curve")
    def _on_explode_curve(self):
        return native.explode_curves(self)

    def _on_reverse_section(self):
        return native.section_action(self, "reverse")

    def _on_toggle_active_section(self):
        return native.section_action(self, "toggle")

    def _on_align_view_to_section(self):
        return native.section_action(self, "align")

    def _on_align_view_to_face(self, face):
        return native.align_face(self.viewport, face)

    def _on_set_surface(self, mode):
        native.set_path_surface(self, mode)

    def _on_toggle_path_closed(self):
        return native.toggle_paths(self)

    def _assign_layer(self, name):
        return native.assign_tag(self, name)

    def _unlock_group(self, group):
        return native.unlock_group(self.viewport, group)

    def _delete_image(self, image):
        return native.image_delete(self.viewport, image)

    def _unlock_image(self, image):
        return native.image_unlock(self, image)

    def _on_toggle_image_lock(self):
        return native.image_lock(self)

    def _on_image_size(self):
        return native.image_measure(self, True)

    def _on_image_opacity(self):
        return native.image_measure(self, False)

    def _single_textured_face(self):
        return native.textured_face(self.viewport)

    def _on_texture_position(self):
        return native.edit_texture(self, False)

    def _on_texture_reset(self):
        return native.edit_texture(self, True)

    def _selected_text3d(self):
        return native.selected_text(self.viewport.scene)

    def _on_edit_3d_text(self, group=None):
        return native.edit_text(self, group)

    def _assign_new_layer(self):
        from views.chrome_forms import ask_text
        scene = self.viewport.scene

        def assign(answer):
            if self.viewport.scene is scene and answer and answer.strip():
                native.assign_tag(self, answer.strip())
        ask_text(self, tr("New layer"), tr("Layer name:"), text=native.next_tag(scene), done=assign)

    def _on_change_axes(self, group) -> None:
        """Right-click ▸ Change Axes: three clicks give the group or
        component new local axes (tools/change_axes.py)."""
        tool = self._tools["change_axes"]
        tool.target = group
        self._activate_tool("change_axes")
        self.viewport.flash_status(tr("Click the new origin"), 4000)

    def _on_flip_along(self, axis: str) -> None:
        """Right-click ▸ Flip Along ▸ Red / Green / Blue (issue #178)."""
        from tools.flip import FlipTool
        FlipTool().flip(self.viewport, axis)

    def _unlinked(self, cmd=None) -> None:
        """A link was broken or brought back (Make Unique of a link, or
        its undo / redo): watch the sources still linked, and update
        the drawing rows."""
        self.viewport.notify_scene_changed()
        self._link_watcher().refresh()
        self._refresh_layer_views()

    def _on_ndof_motion(self, sample, dt: float) -> None:
        # Only the window being worked in moves (New Window makes several).
        if not self.isActiveWindow():
            return
        from core.ndof import apply_ndof
        from views.ndof_input import current_settings
        vp = self.viewport
        if apply_ndof(vp.camera, sample, dt, vp.height(), current_settings()):
            vp.update()

    def _on_ndof_button(self, number: int, down: bool) -> None:
        # The two buttons every model has: both fit the model, the usual
        # default for the right one and the most useful single command.
        if down and number in (0, 1) and self.isActiveWindow():
            self._on_zoom_extents()

    def _toggle_projection(self) -> None:
        """Shift+P, in the active view only."""
        vp = self._active_view()
        vp.toggle_projection()
        self._sync_projection_buttons(vp.camera.projection_mode())

    def activate_select_tool(self) -> None:
        """Back to Select — what the Image tool calls once a picture is
        placed, so the next click doesn't stamp a second copy."""
        self._activate_tool("select")

    def _ensure_composer(self):
        from views.wgpu_sheets import composer_for
        return composer_for(self)

    def _on_open_composer(self):
        self._ensure_composer().show()

    def _show_sheet(self, index):
        self._last_sheet = index
        self._ensure_composer().show_sheet(index)
        self._refresh_sheet_tabs()

    def set_field_of_view(self, degrees):
        return native.lens_angle(self, self._active_view(), degrees)

    def _ask_field_of_view(self):
        from core.camera import FOV_RANGE
        from views.chrome_forms import ask_number
        view = self._active_view()
        cam = view.camera
        if not cam.perspective:
            native.lens_angle(self, view, cam.fov_deg)
            return
        scene = view.scene
        def apply(value):
            if value is not None and self._active_view() is view and view.scene is scene:
                native.lens_angle(self, view, value)
        ask_number(self, tr("Field of view"), tr("Field of view in degrees (1–120):"),
                   cam.fov_deg, *FOV_RANGE, 1, done=apply, suffix="°")

    def keep_component(self, definition, keep=True):
        native.retain_components(self, keep, definition)

    def keep_selected_components(self, keep=True):
        native.retain_components(self, keep)

    def rename_component(self, definition):
        from functools import partial
        from views.chrome_forms import ask_text
        ask_text(self, tr("Rename Component"), tr("Component name:"), text=definition.name or "",
                 done=partial(native.name_definition, self, self.viewport.scene, definition))

    def prompt_section_name(self, plane, *, automatic=True):
        from functools import partial
        from core.settings import settings
        from views import chrome_forms
        if automatic and str(settings().value("section/ask_name", "1")) == "0":
            return
        chrome_forms.show(self, chrome_forms.OperationForm(
            tr("Name Section Plane"), native.OperationForm.section(plane.name, plane.symbol, automatic),
            partial(native.name_section, self, self.viewport.scene, plane)))

    def _on_soften_edges(self) -> None:
        from core.settings import settings
        from views import chrome_forms
        view = self.viewport
        scene = view.scene
        if not scene.selection:
            view.flash_status(
                tr("Select faces, edges or groups to soften"), 4000)
            return
        saved = settings().value("tools/soften_angle", 30.0)
        chrome_forms.show(self, chrome_forms.OperationForm(
            tr("Soften / Smooth Edges"), native.OperationForm.soften(str(saved)),
            lambda options: native.soften_selection(view, options)
            if self.viewport is view and view.scene is scene else None, note=tr(
                "Edges between faces at this angle or less become smooth; "
                "sharper edges stay hard. Selected groups include their nested "
                "geometry, and component copies share the change.")))

    def _on_terrain_profile(self) -> None:
        """Open the terrain chart in the chrome's bottom bar."""
        from views.inspector_bridge import action
        view = getattr(self._doc, "stand_in", None)
        if getattr(view, "renderer", None) is None:
            self.statusBar().showMessage(tr("The terrain profile needs an active renderer."))
            return
        action(self, view, '["terrain.open",null]')

    def _camera_pose(self) -> tuple:
        cam = self.viewport.camera
        t = cam.target
        return (t.x(), t.y(), t.z(), cam.distance, cam.yaw, cam.pitch)

    def edit_scene_camera(self, view):
        from views.inspector_bridge import open_camera
        if view is not None:
            self.tray.scenes.show_view(view)
            return open_camera(self, view)

    def _on_documentation(self) -> None:
        from core.desktop_open import open_url
        open_url("https://ingetrazo-docs.pages.dev/")

    def _on_get_models(self) -> None:
        from views.chrome_dialogs import information
        information(
            self, tr("Get more models and textures"),
            tr("Free sources that open directly in Built:") + "<br><br>"
            "<b>Poly Haven</b> — <a href='https://polyhaven.com'>"
            "polyhaven.com</a> " + tr("(CC0: models OBJ and PBR textures)")
            + "<br><b>ambientCG</b> — <a href='https://ambientcg.com'>"
            "ambientcg.com</a> " + tr("(CC0 textures — drop the PNG into "
                                      "resources/textures)")
            + "<br><b>Sketchfab</b> — <a href='https://sketchfab.com'>"
            "sketchfab.com</a> " + tr("(filter by CC licence, download OBJ)"))

    def _camera_dict(self, vp=None):
        return native.camera_record(vp or self.viewport)

    def _apply_camera_dict(self, home):
        native.apply_camera_record(self.viewport, home)

    def _apply_camera_home(self):
        self._apply_camera_dict(getattr(self.viewport.scene, "camera_home", None))

    def _doc_is_blank(self, doc=None):
        return native.document_blank(doc or self._doc)

    def _free_untitled_no(self):
        return native.untitled_slot(self._documents)

    @property
    def _saved_version(self):
        return self._saved_content

    @_saved_version.setter
    def _saved_version(self, version):
        self._saved_content = native.saved_content(self.__dict__.get("_doc"), version)

    def _is_dirty(self):
        return native.document_dirty(self)

    def _on_undo(self):
        native.history_step(self, False)

    def _on_redo(self):
        native.history_step(self, True)

    def _figure_bounds(self):
        return native.figure_bounds(self.viewport.scene)

    def _image_bounds(self):
        return native.image_bounds(self.viewport.scene)

    def _extents_bounds(self):
        return native.extents_bounds(self)

    def _parse_skp_threaded(self, skp, cb):
        from ingetrazo_core import DocumentOpen
        try:
            return DocumentOpen(skp, insert=True).wait(), None
        except Exception as exc:
            return None, exc

    def _forget_coordinate(self) -> None:
        """A new or opened document: the UTM readout of the one before
        goes. The viewport only clears it on the next hover, so after File ▸
        New the bar kept the previous model's easting/northing (#759)."""
        self.viewport._last_coordinate = ""
        self.viewport._last_coordinate_full = ""
        self._status_pending.pop("coordinate", None)
        self._coord_label.setText("")
        self._coord_label.setToolTip("")

    def _configure_recovery(self, timer):
        native.configure_recovery(self, timer)

    def _on_autosave_tick(self):
        native.request_recovery(self)

    def _on_toggle_eyedropper(self, on):
        native.toggle_eyedropper(self, on)

    @staticmethod
    def _recent_paths():
        from core import recent
        return recent.menu(limit=WindowDocumentCommands._RECENT_MAX)

    def _remember_recent(self, path, doc=None):
        native.remember_document(self, path, doc)

    def _tool_key(self, tool):
        return native.tool_key(self._tools, tool)

    def _link_watcher(self):
        return native.link_watcher(self)

    def _after_links_loaded(self):
        native.links_loaded(self)

    SAVE_PICTURE_PX = 560

    def _on_select_all(self):
        native.edit_selection(self, "all")

    def _on_invert_selection(self):
        native.edit_selection(self, "invert")

    def _on_select_none(self):
        native.edit_selection(self, "none")

    def _on_delete_selection(self):
        native.delete_entities(self.viewport)

    def _on_paste_in_place(self):
        native.paste_model(self, True)

    def _on_paste(self):
        native.paste_model(self, False)

    def _hideable(self, entities):
        return native.hideable(entities)

    def _bridgeable(self, sel):
        return native.bridgeable(self.viewport.scene.mesh, sel)

    @staticmethod
    def _makes_faces(sel):
        return native.makes_faces(sel)

    def _reset_camera_home(self):
        native.reset_home(self.viewport)

    def _apply_new_document_units(self):
        from core import units
        units.apply_units(self.viewport.scene, units.new_document_units())

    def _insert_scale_figure(self):
        native.insert_scale_figure(self)

    def _make_billboard_person(self, image="person_billboard.png", height=1.75, name=None, position=None):
        return native.billboard_person(image, height, name, position)

    def _on_new(self):
        native.new_document_command(self)

    def _on_new_tab(self):
        native.new_document_command(self, True)

    def settle_heap(self):
        native.settle_document_heap(self.GC_THRESHOLD0)

    def _start_blank_document(self):
        native.start_blank_document(self)

    def _shown_scene_index(self):
        return native.marked_scene_index(self)

    def play_scenes(self, on=True):
        native.play_scenes(self, on)

    def step_scene(self, step):
        native.step_scene(self, step)

    def set_scene_transitions(self, on):
        native.scene_transitions(self, on)

    def _no_scene_selected(self):
        from core.i18n import tr
        self.statusBar().showMessage(tr("Select a scene in the Scenes panel first"), 3000)

    def _can_make_here(self):
        return native.can_make_here(self.viewport)

    def _execute_make(self, cmd):
        native.execute_make(self.viewport, cmd)

    @_repeatable("Make Group")
    def _on_make_group(self):
        native.make_group(self)

    @_repeatable("Make Component")
    def _on_make_component(self):
        native.make_component(self)

    @_repeatable("Explode Group")
    def _on_explode_group(self):
        native.explode_group(self)

    def _on_explode_image(self):
        native.explode_images(self.viewport)

    @_repeatable("Hide")
    def _on_hide(self):
        native.change_visibility(self, "hide")

    def _on_unhide_selected(self):
        native.change_visibility(self, "selected")

    def _on_unhide_last(self):
        native.change_visibility(self, "last")

    def _on_unhide_all(self):
        native.change_visibility(self, "all")

    def _hidden_everywhere(self):
        return native.hidden_everywhere(self.viewport.scene)

    _on_hide_edges = _on_hide
    _on_unhide_all_edges = _on_unhide_all

    def _on_toggle_hide_rest(self, _checked=False):
        native.toggle_edit_visibility(self, False)

    def _on_toggle_hide_similar(self, _checked=False):
        native.toggle_edit_visibility(self, True)

    def repeat_last_command(self):
        return native.repeat_last_command(self)

    @_repeatable("Reverse Faces")
    def _on_reverse_faces(self):
        native.face_winding(self, False)

    @_repeatable("Orient Faces")
    def _on_orient_faces(self):
        native.face_winding(self, True)

    @_repeatable("Heal Overlapping Faces")
    def _on_heal_overlaps(self):
        native.heal_overlaps(self)

    @_repeatable("Delete Guides")
    def _on_delete_guides(self):
        native.delete_guides(self.viewport)

    @_repeatable("Intersect Faces")
    def _on_intersect_faces(self, mode):
        native.intersect_faces(self, mode)

    @_repeatable("Convert Path to Geometry")
    def _on_convert_geopath(self):
        native.convert_geopath(self)

    @_repeatable("Bridge")
    def _on_bridge(self):
        native.create_profile_faces(self, True)

    @_repeatable("Make Faces")
    def _on_make_faces(self):
        native.create_profile_faces(self, False)

    @_repeatable("Rebuild Faces (Planar)")
    def _on_rebuild_planar(self):
        native.rebuild_faces(self)

    def _on_simplify_mesh(self):
        native.ask_simplify_mesh(self)

    @_repeatable("Divide")
    def _on_divide(self):
        native.divide_selected(self)

    @_repeatable("Split into Pieces")
    def _on_split_into_pieces(self):
        native.split_pieces(self)

    @_repeatable("Join Segments")
    def _on_join_segments(self):
        native.join_segments(self)

    def _replacement_choices(self, selection):
        return native.replacement_choices(self.viewport.scene, selection)

    def _on_replace_component(self):
        native.choose_replacement(self)

    def replace_selection_with(self, source):
        return native.replace_selection(self, source)

    def _ask_keep_orphans(self, orphans):
        return native.ask_keep_orphans(self, orphans)

    def _offer_untitled_recovery(self):
        native.offer_untitled_recovery(self)

    def _recover_untitled(self, n, slot):
        native.recover_untitled(self, n, slot)

    def _on_recover_discarded(self):
        native.recover_discarded(self)

    def _on_open_cancelled(self):
        if self._open_in_progress:
            self._open_cancelled = True

    def _finish_cancelled_open(self):
        native.reset_cancelled_open(self)

    def _end_group_edits(self):
        self.viewport.end_group_edit()
        self._doc.viewport.end_group_edit()

    def export_video(self):
        from views.video_export import run_export
        run_export(self)

    def ask_animation(self):
        from views.scene_playback import edit_animation
        edit_animation(self)

    def _on_export_view_svg(self):
        from views.svg_export import open_view_export
        return open_view_export(self)

    def _run_import(self, load, dlg, title, describe=None, command_type=None):
        return native.run_import(self, load, dlg, title, describe, command_type)

    def _prepare_import_display(self, cmd, cb):
        native.prepare_import(self.viewport, cmd, cb)

    def _report_import(self, name, offset=None):
        native.report_import(self, name, offset)

    _survey_note = staticmethod(native.survey_note)
    _dxf_failure_text = staticmethod(native.dxf_failure)

    def _import_rfa_path(self, path):
        return native.import_file(self, path, "rfa")

    def _import_glb_path(self, path):
        return native.import_file(self, path, "glb")

    def _import_dae_path(self, path):
        return native.import_file(self, path, "dae")

    def _dxf_options(self, path, header_scale=None):
        return native.dxf_options(self, header_scale)

    def _import_dxf_path(self, path, label=None):
        return native.import_file(self, path, "dxf", label=label)

    def _apply_obj_import(self, path, scale):
        return native.import_file(self, path, "obj", scale)

    def _apply_stl_import(self, path, scale, simplify_mode):
        return native.import_file(self, path, "stl", scale, simplify_mode)

    def _start_place(self, group, align_to_face=False, anchor=None):
        native.start_placement(self, group, align_to_face, anchor)

    def import_igz_path(self, path, at=None):
        return native.insert_document(self, path, at)

    def _on_insert_3d_text(self):
        from views.font_picker import text3d
        text3d(self, None, self._place_text3d)

    def _place_text3d(self, params):
        native.place_text(self, params)

    def _on_import_image(self):
        native.import_picture(self)

    def _on_import_obj(self):
        native.pick_mesh_file(self, False)

    def _on_import_stl(self):
        native.pick_mesh_file(self, True)

    def _mesh_import_options(self, path, *, open_tab=None):
        return native.ask_mesh_units(self, path, open_tab)

    def _on_export_image(self):
        native.choose_image_export(self)

    def _export_image_to(self, path):
        native.image_export_form(self, path)

    def _save_export_image(self, view, path, width, transparent):
        native.save_export_image(self, view, path, width, transparent)

    def import_skp_path(self, path):
        return native.import_skp(self, path)

    def _import_dwg_path(self, path):
        return native.import_dwg(self, path)

    def _on_clear_texture_cache(self):
        native.clear_texture_cache(self)

    def link_igz_path(self, path, at=None, select=None, *, include_nested=True):
        return native.link_document(self, path, at, select, include_nested)

    def _relink_done(self, note):
        native.link_changed(self, note)

    def _reload_link(self, path):
        native.reload_links(self, path)

    def _reload_all_links(self):
        native.reload_links(self)

    def _set_link_nested(self, group, enabled):
        return native.set_link_nested(self, group, enabled)

    def _reset_link_tags(self, path):
        native.reset_link_tags(self, path)

    def _pack_link(self, path, on):
        native.pack_link(self, path, on)

    def _view_line_groups(self, fast=True):
        from views.svg_export import view_line_groups
        return view_line_groups(self._active_view(), fast)

    def view_svg_builder(self):
        from views.svg_export import active_view_builder
        return active_view_builder(self)

    def open_preferences(self, shortcut_of=None, page=None):
        from views import settings_sheet
        settings_sheet.show(self, page=page, shortcut_of=shortcut_of)

    def _on_units(self):
        self.open_preferences(page="units")

    def _on_check_updates(self):
        from views.release_check import start_check
        start_check(self, quiet=False)

    def check_updates_quietly(self):
        from views.release_check import start_check
        start_check(self, quiet=True)

    def _choose_link_contents(self, path, select=None):
        native.choose_link_contents(self, path, select)

    def _confirm_discard(self, prompt):
        return native.confirm_discard(self, prompt)

    def _confirm_quit(self):
        return native.confirm_quit(self)

    def _save_all_or_stay(self, dirty, answer):
        return native.save_selected(self, dirty, answer)

    def _on_insert_component(self, key, name=None):
        native.place_bundled(self, key, name)

    def _build_bundled_component(self, key, name):
        return native.bundled_component(self, key, name)

    def insert_library_component(self, entry, obj=None):
        native.place_catalogue(self, entry, obj)

    def _build_library_component(self, entry, obj=None):
        return native.catalogue_component(self, entry, obj)

    def _on_insert_person_2d(self, image="person_billboard.png", height=1.75, name=None):
        native.place_figure(self, image, height, name)

    def _on_insert_faceme_image(self):
        native.choose_billboard(self)
