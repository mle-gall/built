"""Bind the Rust document commands to each window's active Python controller."""
import ingetrazo_render as native


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
