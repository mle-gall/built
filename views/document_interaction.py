# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document editing, picking and painter annotations shared by the presentation hosts."""
from __future__ import annotations
import ingetrazo_core
from core.mesh import element_identity
from core.element_set import ElementSet

import copy
import importlib
import itertools
import math
import os
import sys
import time as _time_mod
import types
from array import array
from pathlib import Path
from typing import Optional
from core.units import fmt_len as _fmt_len
from core.units import typed_value_text as _typed_value_text

_perf_file = None


def _plog(tag: str, ms: float, extra: str = "", floor: float = 50.0) -> None:
    """Frame telemetry (P0). Lines carry the writer's pid and honour
    ``$INGETRAZO_PERF_LOG`` — see ``core.history._plog`` for why."""
    global _perf_file
    if not _PERF or ms < floor:
        return
    if _perf_file is None:
        path = os.environ.get("INGETRAZO_PERF_LOG")
        _perf_file = open(Path(path) if path
                          else Path.home() / "ingetrazo-perf.log",
                          "a", buffering=1)
    _perf_file.write(f"{_time_mod.strftime('%H:%M:%S')} [{os.getpid()}] "
                     f"{tag} {ms:.0f}ms"
                     f"{' ' + extra if extra else ''}\n")

from ingetrazo_core import Type as QEvent, Input as Qt, ScreenPoint as QPointF, ScreenRect as QRectF
from core.host_callbacks import Signal
from core.timers import later, timer as window_timer
from ingetrazo_render import AnnotationFont as QFont, PaintColor as QColor, PaintPen as QPen
import numpy as np
import ingetrazo_core
import ingetrazo_render
from ingetrazo_render import document_value_key as _value_key
from ingetrazo_core import ray_aabb_span as _ray_aabb_span
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D, Vec4 as QVector4D

from core.camera import OrbitCamera
from core.i18n import tr
from core.group import (Group, PlacementProxy, copy_group, effective_material,
                        world_mesh)
from core import layers as _layers
from core.mesh import Edge, Face, smooths
from core.history import EraseSelectionCommand, History
from core.scene import Scene
from core.style import (DEFAULT_BACK_COLOR, DEFAULT_FACE_COLOR,
                        MONOCHROME_GLASS_RGB,
                        effective_back_color)
from core.materials import effective_attrs, material_sig as _material_sig
from core.snap import SnapResult, _AXIS_VECTORS
from core.texture import face_uv_axes
import ingetrazo_core
from core.triangulate import plane_axes
from tools.base import Tool, ToolContext
from core.inference import InferenceEngine

ingetrazo_render.register_document_module(globals())


_HoverEvent = ingetrazo_render.MouseEvent.hover


#: Every chunk entry built (or loaded from disk) takes the next number.
#: Keys that told entries apart by ``id(entry)`` could not: the dict a
#: rebuild replaces is freed at once, and the new one may land at the same
#: address — same id, same rev 0 — so the instance arrays and the GL draw
#: entry derived from the OLD chunk were served as if nothing had changed
#: (a face painted inside a component came back unpainted one level up,
#: Marco, 2026-09-11).
_chunk_uid = itertools.count(1)


class _LazyChunk(dict):
    """A chunk whose draw-only fields are computed on first use: a component
    placement baked for the pick index (#158's lazy placements, hundreds per
    pointer move) needs its triangles, edges and box, never its colours,
    texture runs and re-shading — the dearer two thirds of the bake. Each
    field comes from the same function on the same inputs whenever it is
    asked for, so what is drawn is what the eager bake gave."""
    __slots__ = ("_later",)

    def __init__(self, data, later):
        super().__init__(data)
        self._later = later              # field -> callable making it

    def __missing__(self, key):
        return ingetrazo_render.lazy_chunk_value(self, key)

    def __setitem__(self, key, value):
        self._later.pop(key, None)
        dict.__setitem__(self, key, value)

    def __contains__(self, key):
        return dict.__contains__(self, key) or key in self._later

    def get(self, key, default=None):
        return self[key] if key in self else default

    def _made(self):
        return ingetrazo_render.lazy_chunk_fill(self)

    def keys(self):
        return dict.keys(self._made())

    def items(self):
        return dict.items(self._made())

    def values(self):
        return dict.values(self._made())

    def __iter__(self):
        return dict.__iter__(self._made())

    def __len__(self):
        return dict.__len__(self._made())

    def copy(self):
        return dict(self.items())


#: The CPU side of a document's bakes — the render/pick chunks of its
#: groups, the instances' derived chunks, the fingerprints and the
#: prototypes' stand-ins — one store per Scene, shared by every viewport
#: that shows it. Four views of the plaza (1 035 objects) baked the model
#: four times over, 16.7 s on the first split; the arrays do not depend on
#: the view, only their upload to each view's own GL context does.
import weakref
#: ``id(scene)`` → ``(weakref to the scene, its store)``. By id, not by the
#: scene itself: a Scene compares by value and has no hash. The weak
#: reference tells a dead scene's id from a new scene born at the same
#: address, and a finalizer drops the store when the scene goes.
_BAKE_STORES: dict = {}


#: Every viewport made, weakly: the decoded textures are kept only while
#: more than one of them is on screen (:func:`_several_views_shown`).
_LIVE_VIEWS: "weakref.WeakSet" = weakref.WeakSet()


#: Texture images decoded once per file, for every GL context that
#: uploads them: ``path`` → ``(mtime, size, mirrored SurveyRaster, cutout)``,
#: the newest last, held to a budget of bytes. Three views joining the
#: plaza's decoded its textures again each (1.6 s of the 3.8 s); the GL
#: texture itself belongs to each context and is made from this.
_DECODED_TEXTURES: dict = {}


# Existing native callers still accept the legacy rounding-mode argument.
from core._native import qt_hypot
_QV_HYPOT3 = qt_hypot()


from ingetrazo_core import point_in_screen_hull as _point_in_screen_hull
from ingetrazo_core import MIN_CIRCLE_DASHES as _MIN_CIRCLE_DASHES




from views.document_tools import DocumentTools
from views.view_navigation import ViewNavigation


class DocumentInteraction(DocumentTools, ViewNavigation):
    """Editing and input shared by the document presentation hosts."""

    def _configure_surface(self):
        pass

    valueBufferChanged = Signal(str)
    sceneVersionChanged = Signal(int)
    # Live measurement for the VCB box (e.g. "5.00 m", "3.00 × 2.00 m").
    measurementChanged = Signal(str)
    # A base-map tile finished downloading (Track G) — lets the 3D terrain
    # rebuild its mosaic as imagery arrives.
    tilesChanged = Signal()
    # UTM coordinate under the cursor, for the status bar readout (Track G).
    coordinateChanged = Signal(str)
    # The camera's projection changed ("parallel" / "perspective" /
    # "two_point"), whoever changed it — the View toolbar's quick buttons
    # follow it.
    projectionChanged = Signal(str)
    # A trackpad's smart zoom (two-finger double tap on macOS): the window
    # frames the model, and a second one goes back (#248).
    smartZoomRequested = Signal()

    # Soft warm white painted on faces with no material colour — like the matte
    # cardstock of an architecture model (a near-white default).
    DEFAULT_FACE_COLOR = DEFAULT_FACE_COLOR
    # Fixed world light (from above, slightly front-right) for the subtle diffuse
    # face shading. World-fixed so shading is stable while orbiting.
    _LIGHT = QVector3D(0.35, 0.25, 1.0).normalized()

    _bind_bake_store = _bind_bake_store

    def __init__(self, parent=None, **surface) -> None:
        super().__init__(parent, **surface)
        self.view_filter = None
        self.view_tint = None
        self.active_view_key = None
        # The model and its camera are the same in every language: a right-
        # to-left interface (#1067) does not mirror what is drawn here.
        self._configure_surface()
        self.setMinimumSize(640, 480)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setMouseTracking(True)

        self._init_document()

        ingetrazo_render.init_document_interaction(self)
        ingetrazo_render.init_navigation(self)

    # Chunk arrays are a pure function of the mesh content; persisting them
    # turns the multi-second cold build of a big group into a disk read.
    # Lives in the app's own cache dir (like extracted textures) and is
    # keyed by a STABLE sha1 content digest — a stale entry can only miss.

    def _offer_link_source(self, group) -> None:
        """«“Site.igz” is linked: open it to edit it?» — and open it in a
        tab of its own (or bring that tab forward) on yes."""
        from core import links
        from ingetrazo_render import chrome_dialogs
        path = Path(links.link_of(group)["path"])
        if not path.is_file():
            chrome_dialogs.warning(
                self, tr("Linked document"),
                tr("“{name}” is a linked document, and its file was not "
                   "found:\n{path}", name=path.name, path=str(path)))
            return
        # The Built UI asks on its sheet; Qt when the chrome is not on screen.
        answer = chrome_dialogs.ask(
            self, tr("Open the source file “{name}”?", name=path.name),
            tr("It is edited in its own document; every change saved there "
               "shows here."),
            [("cancel", tr("Cancel")), ("open", tr("Open {name}", name=path.name))],
            default="open", cancel="cancel")
        if answer != "open":
            return
        window = getattr(self, "window", None)
        window = window() if callable(window) else None
        opener_fn = getattr(window, "open_path", None)
        if callable(opener_fn):
            from views.wgpu_tabs import open_link_source
            open_link_source(window, path, group)

    # The two post-processors of the search (the axis-source cue for the
    # tools that read the model axes as a source, the extension hooks) moved
    # behind ``self.inference.snap()`` too. The names stay on the viewport
    # because the extension tests and the probe call them from here.
    _MODIFIER_KEYS = frozenset({Qt.Key_Control, Qt.Key_Shift, Qt.Key_Alt,
                                Qt.Key_Meta, Qt.Key_AltGr})

    def event(self, ev) -> bool:
        if ingetrazo_render.document_event(self, ev):
            return True
        return super().event(ev)

    def keyPressEvent(self, ev) -> None:
        if ingetrazo_render.document_key_press(self, ev):
            return
        return super().keyPressEvent(ev)

    #: The one extension item selected — ``(entry, item_id, scene version)``
    #: — or None. Kept APART from ``scene.selection``, which only ever holds
    #: the model's own entities, so nothing that walks the selection (Move,
    #: layers, zoom, copy) meets a thing it does not know.
    extension_pick = None

    def keyReleaseEvent(self, ev) -> None:
        if ingetrazo_render.document_key_release(self, ev):
            return
        return super().keyReleaseEvent(ev)

    #: The view shown in this one's place (the Metal view), which takes the
    #: keyboard whenever this hidden one is given it.
    keys_stand_in = None

    def setFocus(self, *args) -> None:  # noqa: N802 — Qt's name
        if ingetrazo_render.document_focus(self, args):
            return
        return super().setFocus(*args)

    def focusNextPrevChild(self, next) -> bool:  # noqa: A002 — Qt's name
        """Tab is a tool key (``views.shortcuts.reserved_reason``): Qt's
        focus chain took it and left the keyboard on a tray button, where
        the arrows, typed values and Enter did nothing."""
        return False

    def focusOutEvent(self, ev) -> None:
        ingetrazo_render.document_focus_out(self)
        super().focusOutEvent(ev)

ingetrazo_render.register_document_host(DocumentInteraction)


from ingetrazo_render import register_view_class, register_view_functions
register_view_class(DocumentInteraction)
register_view_functions(globals())
