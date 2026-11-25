# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Saved views — "Scenes": a named snapshot of the camera and of
the layer-visibility state, re-applied with one click.

A saved view never owns geometry; it is pure presentation state. Together
with layers it completes the '2D that emerges' workflow: "Plan view" =
top camera + parallel projection + only the plan layers on, stored as a
view and recalled instantly.

The camera snapshot is stored in the orbit camera's own terms (target /
distance / yaw / pitch), so capture → apply round-trips exactly. Importers
that carry eye/target/up cameras (e.g. .skp) convert once at import time.
"""
from __future__ import annotations

import ingetrazo_core as _NATIVE
from ingetrazo_core import presentation as _presentation  # noqa: F401

from core.doc_model import Record, record_fields

from ingetrazo_core.history import CommandRejected
# Native steps (docs/commands-port.md), kept under these names.
from ingetrazo_core import MoveSceneCommand, RenameSceneCommand  # noqa: F401


@record_fields("name", "navigator_view", "folder", "target", "distance", "yaw",
               "pitch", "fov_deg", "perspective", "two_point", "hidden_layers",
               "hidden_folders", "style", "section", "georef", "shadows",
               "hidden_objects", "hidden_shown", "lens")
class SavedView(Record):
    """A named camera + layer-visibility snapshot."""

    _photo_tick = 0

    def __init__(self, name: str, target=(0.0, 0.0, 0.0), distance: float = 20.0,
                 yaw: float = -0.7853981633974483, pitch: float = 0.5235987755982988,
                 fov_deg: float = 45.0, perspective: bool = True,
                 two_point: bool = False, hidden_layers=None, style=None, section=None,
                 georef=None, shadows=None, hidden_objects=None,
                 hidden_shown=None, navigator_view=None, folder=None,
                 lens=None, hidden_folders=None) -> None:
        self.navigator_view = navigator_view
        self.name = name
        #: The folder of the Scenes tab the scene is filed under, or
        #: ``None`` at the top (ingetrazo_core.scene_tree, #1158). Only organises
        #: the list: nothing of the view depends on it.
        self.folder = folder or None
        self.target = tuple(target)
        self.distance = float(distance)
        self.yaw = float(yaw)
        self.pitch = float(pitch)
        self.fov_deg = float(fov_deg)
        self.perspective = bool(perspective)
        #: Two-point perspective (verticals kept vertical).
        self.two_point = bool(two_point)
        #: Layer NAMES hidden in this view. Every other layer shows — a layer
        #: created after the view was saved defaults to visible.
        self.hidden_layers = list(hidden_layers or [])
        #: Tag folder KEYS switched off in this view (their own switch;
        #: the tags inside keep theirs). ``None``: a view saved before
        #: folders had a switch, which leaves the folders as they are.
        self.hidden_folders = (None if hidden_folders is None
                               else list(hidden_folders))
        #: Display-style snapshot (core.style.Style.to_dict()) or ``None`` —
        #: a scene remembers the style it was saved with.
        self.style = dict(style) if style else None
        #: Section state (scenes remember it): the ACTIVE plane's
        #: uid (or None = no cut) + the two visibility toggles. ``None``
        #: entirely = a view loaded from a document older than sections,
        #: which is left alone on recall. A view captured with NO planes in
        #: the model still records {"active": None}: "Active Section
        #: Planes" is saved per scene by default, so a plan or
        #: elevation scene made BEFORE any cut switches the cut OFF when
        #: recalled — every scene stands on its own (Marco, 2026-09-02).
        self.section = dict(section) if section else None
        #: Geographic reference layers shown in this view: the flat base
        #: map, the 3D terrain and the photogrammetric survey. In a .skp
        #: those are groups on their own layers, so a scene hides or shows
        #: them like anything else; here they hang off the scene as display
        #: objects, so the view records their visibility explicitly — a
        #: plan scene taken WITH the map and a detail scene taken WITHOUT it
        #: must each come back (and render on a sheet) as they were made
        #: (Marco, 2026-09-14). ``None`` = a view from a document older than
        #: this field, left alone on recall.
        self.georef = dict(georef) if georef else None
        #: Shadow settings snapshot (core.sun.ShadowSettings.to_dict()):
        #: on/off, date, time, darkness. Scenes save "Shadow Settings"
        #: too — a 3D captured with shadows renders its sheet
        #: frame with shadows, one captured without renders without (Marco,
        #: 2026-09-14). ``None`` = a view from before this field, hands-off.
        self.shadows = dict(shadows) if shadows else None
        #: The objects (group uids) hidden when the scene was saved —
        #: scenes remember "Hidden Objects" (Rafael, 2026-09-16:
        #: «una escena en donde esto esté oculto»). ``None`` = a view from
        #: before this field, hands-off; a list, even empty, is applied.
        self.hidden_objects = (list(hidden_objects)
                               if hidden_objects is not None else None)
        #: View ▸ Hidden Objects / Hidden Geometry as the scene was saved
        #: (``{"objects": bool, "geometry": bool}``); scenes keep
        #: both. ``None`` = hands-off.
        self.hidden_shown = dict(hidden_shown) if hidden_shown else None
        #: The camera's lens beyond the field of view — frame shift, roll,
        #: clipping, frame shape (``OrbitCamera.lens_dict``). ``{}`` is an
        #: ordinary lens, and recalling it takes off whatever lens the view
        #: had before: every scene stands on its own, like sections.
        self.lens = dict(lens) if lens else {}

    # ---- Snapshot / recall ---------------------------------------------------
    # In Rust (``rust/ingetrazo_core/src/model/saved_view_state.rs``), read
    # and written attribute for attribute as the Python did.

    @classmethod
    def capture(cls, name: str, scene, camera, style=None) -> "SavedView":
        """Snapshot the live camera and the current layer visibility.
        ``style``: the one the view shows when it has its own (one of
        several views); else the scene's."""
        return _NATIVE.saved_view_capture(cls, name, scene, camera, style)

    def take_camera(self, camera) -> None:
        """Keep ``camera`` as this scene's camera — and nothing else: the
        layers, style, section and hidden objects it remembers stay as they
        are (Camera Settings editing a scene)."""
        _NATIVE.saved_view_take_camera(self, camera)

    def recapture(self, scene, camera, style=None) -> None:
        """Update this view in place from the live state (keeps the name
        and the folder). ``style``: as in :meth:`capture`."""
        _NATIVE.saved_view_recapture(self, scene, camera, style)

    def apply(self, scene, camera, put_style=None) -> None:
        """Recall the view: camera first, then the layer-visibility state.
        The caller bumps ``scene.version`` / repaints (tray convention).
        ``put_style``: where the style goes, for a view with its own (one
        of several views); else it becomes the scene's."""
        _NATIVE.saved_view_apply(self, scene, camera, put_style)

    def apply_camera(self, camera) -> None:
        """Only the camera, its lens included (a scene transition glides it
        there first)."""
        _NATIVE.saved_view_apply_camera(self, camera)

    def apply_state(self, scene, put_style=None) -> None:
        """Everything but the camera: layers, style, section, geographic
        layers, shadows, hidden objects — each only when the view
        remembers it. ``put_style`` as in :meth:`apply`."""
        _NATIVE.saved_view_apply_state(self, scene, put_style)

    # ---- Serialisation (.igz) ------------------------------------------------
    def to_dict(self) -> dict:
        """The .igz entry: defaults left out, ``parallel`` for the inverse
        of ``perspective`` (the document's own archive, ``doc/file_view.rs``)."""
        return self._file_view()

    @classmethod
    def from_dict(cls, raw: dict) -> "SavedView":
        return _NATIVE.saved_view_from_dict(cls, raw)


def from_lookat(name: str, eye, target, up, fov_deg: float = 45.0,
                perspective: bool = True, ortho_height: float | None = None,
                hidden_layers=None) -> SavedView:
    """A :class:`SavedView` from an eye/target/up camera (importer entry).

    ``eye``/``target``/``up`` are (x, y, z) metres, Z-up. The orbit camera
    derives its screen-up from the world Z axis, so only the LOOK direction
    survives the conversion exactly; for a straight-down view (plan), the
    stored ``up`` becomes the yaw (screen rotation) instead — that keeps
    "north up" plans north-up. For a parallel view, ``ortho_height``
    (visible height in metres) sets the framing distance
    (``rust/ingetrazo_core/src/model/saved_view.rs``).
    """
    from ingetrazo_core import saved_view_lookat
    distance, yaw, pitch = saved_view_lookat(eye, target, (-up[0], -up[1]), fov_deg, bool(perspective),
                                             ortho_height or None)
    return SavedView(name, target=tuple(target), distance=distance, yaw=yaw,
                     pitch=pitch, fov_deg=fov_deg, perspective=perspective,
                     hidden_layers=hidden_layers)
