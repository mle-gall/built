# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Render targets sharing one document’s resident geometry and inference index."""
class TargetRenderer:
    """The renderer as one view sees it: every call is made with that
    view's surface live. Attribute reads too (``gpu_memory``,
    ``pick_cache_valid`` belong to the live view)."""

    __slots__ = ("raw", "target")

    def __init__(self, raw, target: int) -> None:
        object.__setattr__(self, "raw", raw)
        object.__setattr__(self, "target", int(target))

    def __getattr__(self, name):
        raw, target = self.raw, self.target
        raw.bind_target(target)
        value = getattr(raw, name)
        if not callable(value):
            return value

        def call(*args, **kwargs):
            raw.bind_target(target)
            return value(*args, **kwargs)
        call.__name__ = name
        return call

    def __repr__(self) -> str:
        return f"<TargetRenderer {self.target} of {self.raw!r}>"


def raw_renderer(renderer):
    """The native renderer behind ``renderer`` (itself, unwrapped)."""
    return getattr(renderer, "raw", renderer) if isinstance(
        renderer, TargetRenderer) else renderer


#: What a peer reads and writes on the document's first view: the resident
#: scene and everything keyed to it.
SHARED = (
    "packed", "scene", "uid_rows", "inference_index", "_inference_future",
    "_inference_worker", "_inference_queued", "_instance_updates",
    "_content_token", "_paint_key", "_billboards", "_container_boxes",
    "_container_box_rows", "_resident_count", "_resident_edit_copies",
    "_scene_dirty", "_remote_meshes", "_parent_map", "_rest_signature", "_selection_rows",
    "suppressed_faces", "build_seconds", "upload_seconds", "after_open",
)


def _forward(name):
    def get(self):
        primary = self.__dict__.get("primary")
        if primary is None:
            try:
                return self.__dict__[name]
            except KeyError:
                raise AttributeError(name) from None
        return getattr(primary, name)

    def put(self, value):
        primary = self.__dict__.get("primary")
        if primary is None:
            self.__dict__[name] = value
        elif self.__dict__.get("_peer_ready"):
            setattr(primary, name, value)
        # Else WgpuView.__init__'s defaults: dropped. Not kept in the
        # peer's own __dict__ either: shiboken looks there before the
        # class's properties, and the peer would read its own None.

    def drop(self):
        primary = self.__dict__.get("primary")
        if primary is not None and self.__dict__.get("_peer_ready"):
            delattr(primary, name)
        else:
            self.__dict__.pop(name, None)
    return property(get, put, drop)


class ResidentPeer:
    """Another view of ``primary``'s resident scene, on its own surface."""

    def __init__(self, primary, host_or_parent=None) -> None:
        self.__dict__["primary"] = primary
        super().__init__(host_or_parent)
        self._target = None
        self._peer_ready = True
        primary.add_peer(self)

    @property
    def _shared(self):
        return self.primary

    # ---- the document: the first view's ------------------------------------
    def set_scene(self, scene, *, progress=None, opening=False):
        self.primary.set_scene(scene, progress=progress, opening=opening)
        self.update()

    def blank(self):
        self.primary.blank()

    def _prepare_inference(self):
        self.primary._prepare_inference()

    def _refresh_inference(self):
        self.primary._refresh_inference()

    def _reserve_edit_room(self, tracker):
        self.primary._reserve_edit_room(tracker)

    def _upload(self, force=False):
        if (self.isVisible() and self._can_create_surface()) or force:
            if self.primary._scene_dirty or self.primary.renderer is None:
                self.primary._upload(force=True)
            self._attach()
        self.update()

    def release(self) -> None:
        """Hand the surface back (the grid going away)."""
        renderer, self.renderer = self.renderer, None
        if isinstance(renderer, TargetRenderer):
            _release(renderer.raw, renderer.target)
        self.primary.remove_peer(self)

def _release(raw, target) -> None:
    try:
        raw.remove_target(target)
    except Exception:  # noqa: BLE001 — already gone with its renderer
        pass


for _name in SHARED:
    setattr(ResidentPeer, _name, _forward(_name))
del _name
