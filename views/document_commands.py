"""Bind the Rust document commands to each window's active Python controller."""
import ingetrazo_render as native


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
