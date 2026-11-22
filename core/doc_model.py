# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust document model's switch and its Python side
(docs/rust-document-model.md).

``NATIVE`` is ``ingetrazo_core`` when the document lives in Rust, else
``None`` (``INGETRAZO_DOC=0``, ``INGETRAZO_RUST=0``, or a build without the
module). Decided once, at import: ``core.group.Group``,
``core.layers.Layer`` and ``core.scene.Scene`` are defined over it, so a
process runs one model or the other, never both (one owner per piece of
state). The Python classes are the oracle the parity tests compare with.

What lives here:

- :class:`PlacedMatrix`, the ``QMatrix4x4`` a Rust placement hands out as
  its ``xform`` / ``axes``: edited in place (``g.xform.translate(v)``) it
  writes the result back to the document, as editing the stored matrix
  did when the matrix was a Python attribute;
- the transaction helpers History drives (:func:`begin`, :func:`commit`,
  :func:`rollback`);
- :func:`new_scene_root`, the root node that owns a scene's
  ``groups`` and ``layers``.
"""
from __future__ import annotations

import os

from PySide6.QtGui import QMatrix4x4


def _module():
    if os.environ.get("INGETRAZO_DOC") == "0" or os.environ.get("INGETRAZO_RUST") == "0":
        return None
    from core import _native
    mod = _native.available()
    if mod is None or not hasattr(mod, "doc_new_root"):
        return None
    return mod


NATIVE = _module()
ENABLED = NATIVE is not None


def engine() -> str:
    """«Rust» or «Python», for the logs and the probes."""
    return "Rust" if ENABLED else "Python"


# --------------------------------------------------------------- matrices

def _write_back(m) -> None:
    bind = m.__dict__.get("_doc_bind")
    if bind is None:
        return
    if not NATIVE.doc_view_changed(bind[0], bind[1], m, m.data(),
                                   getattr(m, "_built_source", None)):
        # Replaced (or its placement gone): an ordinary matrix from now on.
        m.__dict__.pop("_doc_bind", None)


class PlacedMatrix(QMatrix4x4):
    """A placement's matrix as handed out by the document. Arithmetic
    returns plain ``QMatrix4x4`` values; the in-place methods also store the
    result in the placement while this is still its matrix."""

    __slots__ = ()

    def translate(self, *a):
        QMatrix4x4.translate(self, *a)
        _write_back(self)

    def rotate(self, *a):
        QMatrix4x4.rotate(self, *a)
        _write_back(self)

    def scale(self, *a):
        QMatrix4x4.scale(self, *a)
        _write_back(self)

    def setColumn(self, *a):
        QMatrix4x4.setColumn(self, *a)
        _write_back(self)

    def setRow(self, *a):
        QMatrix4x4.setRow(self, *a)
        _write_back(self)

    def setToIdentity(self):
        QMatrix4x4.setToIdentity(self)
        _write_back(self)

    def fill(self, *a):
        QMatrix4x4.fill(self, *a)
        _write_back(self)

    def lookAt(self, *a):
        QMatrix4x4.lookAt(self, *a)
        _write_back(self)

    def ortho(self, *a):
        QMatrix4x4.ortho(self, *a)
        _write_back(self)

    def frustum(self, *a):
        QMatrix4x4.frustum(self, *a)
        _write_back(self)

    def perspective(self, *a):
        QMatrix4x4.perspective(self, *a)
        _write_back(self)

    def viewport(self, *a):
        QMatrix4x4.viewport(self, *a)
        _write_back(self)

    def flipCoordinates(self):
        QMatrix4x4.flipCoordinates(self)
        _write_back(self)

    def optimize(self):
        QMatrix4x4.optimize(self)

    def __imul__(self, other):
        QMatrix4x4.__imul__(self, other)
        _write_back(self)
        return self

    def __iadd__(self, other):
        QMatrix4x4.__iadd__(self, other)
        _write_back(self)
        return self

    def __isub__(self, other):
        QMatrix4x4.__isub__(self, other)
        _write_back(self)
        return self

    def __itruediv__(self, other):
        QMatrix4x4.__itruediv__(self, other)
        _write_back(self)
        return self

    def __reduce__(self):
        return (QMatrix4x4, tuple(self.data()[c * 4 + r]
                                  for r in range(4) for c in range(4)))

    def __deepcopy__(self, memo):
        out = QMatrix4x4(self)
        src = getattr(self, "_built_source", None)
        if src is not None:
            out._built_source = src
        return out

    __copy__ = __deepcopy__


def _make(key, which, data, src):
    """Rust → a new view of a placement's matrix (``data`` column-major)."""
    m = PlacedMatrix(data[0], data[4], data[8], data[12],
                     data[1], data[5], data[9], data[13],
                     data[2], data[6], data[10], data[14],
                     data[3], data[7], data[11], data[15])
    if src is not None:
        m._built_source = src
    m._doc_bind = (key, which)
    return m


try:
    from shiboken6 import Shiboken as _shiboken
    _created_by_python = _shiboken.createdByPython
except ImportError:  # pragma: no cover - PySide6 always ships it
    def _created_by_python(_value):
        return False


def _bind(value, key, which):
    """A matrix assigned to a placement → ``(view, data, built source)``.

    A plain ``QMatrix4x4`` BECOMES the view (its class is switched in
    place), so ``g.xform = m; m.translate(v)`` still moves ``g`` and
    ``g.xform is m`` still holds, as with the Python attribute. A view
    already bound elsewhere is copied: one matrix names one placement."""
    t = type(value)
    if t is QMatrix4x4 and _created_by_python(value):
        # (A matrix Qt made, an operator's result, cannot change class:
        # shiboken never ran the subclass's __init__ on it. Copied below.)
        value.__class__ = PlacedMatrix
    elif t is PlacedMatrix:
        if value.__dict__.get("_doc_bind") not in (None, (key, which)):
            src = getattr(value, "_built_source", None)
            value = PlacedMatrix(value)
            if src is not None:
                value._built_source = src
    elif isinstance(value, QMatrix4x4):
        src = getattr(value, "_built_source", None)
        value = PlacedMatrix(value)
        if src is not None:
            value._built_source = src
    else:
        # 16 numbers, column-major as ``QMatrix4x4.data()`` gives them.
        try:
            data = [float(v) for v in value]
        except (TypeError, ValueError):
            data = ()
        if len(data) != 16:
            raise TypeError(f"a placement matrix is a QMatrix4x4, not {t.__name__}")
        value = _make(key, which, data, None)
    value._doc_bind = (key, which)
    return value, value.data(), getattr(value, "_built_source", None)


if ENABLED:
    NATIVE.doc_set_hooks(_make, _bind)


# ----------------------------------------------------------- transactions

def begin() -> int:
    """Open the step's transaction (or a savepoint in the open one)."""
    return NATIVE.doc_begin() if ENABLED else 0


def commit():
    """Close it: the outermost level returns its ``Transaction``."""
    return NATIVE.doc_commit() if ENABLED else None


def rollback() -> None:
    """Put back what the innermost level wrote, and close it."""
    if ENABLED:
        NATIVE.doc_rollback()


def stats() -> dict:
    return NATIVE.doc_stats() if ENABLED else {}


# ---------------------------------------------------------------- handles

def new_handle(cls):
    """An empty handle of ``cls`` (``Group``, ``Layer``): what copy and
    pickle make before ``set_handle_state`` fills it."""
    return cls.__new__(cls)


def handle_state(obj, fields) -> dict:
    """The fields ``obj`` has set, by name; lists as plain lists."""
    out = {}
    for name in fields:
        try:
            value = getattr(obj, name)
        except AttributeError:
            continue
        if type(value).__name__ == "HandleList":
            value = list(value)
        out[name] = value
    return out


def is_list(value) -> bool:
    """A list, or a document list (``g.children``, ``scene.groups``,
    ``scene.layers``) that behaves as one."""
    return isinstance(value, list) or (
        ENABLED and type(value) is NATIVE.HandleList)


def object_state(obj) -> dict:
    """Every attribute of a placement or a tag by name, what ``vars(obj)``
    or its ``__slots__`` gave when all of them were Python attributes
    (document lists as plain lists)."""
    from core.group import GROUP_FIELDS, Group
    from core.layers import Layer, layer_values
    if isinstance(obj, Group):
        return handle_state(obj, GROUP_FIELDS)
    if isinstance(obj, Layer):
        return layer_values(obj)
    if hasattr(obj, "__dict__"):
        return vars(obj)
    return {k: getattr(obj, k) for k in obj.__slots__ if hasattr(obj, k)}


def set_object_state(obj, values: dict) -> None:
    """Put back an :func:`object_state` (the instance dict replaced, as
    ``vars(obj).clear(); vars(obj).update(values)`` did)."""
    from core.layers import TAG_FIELDS, Layer
    values = dict(values)
    doc = {}
    if ENABLED and isinstance(obj, Layer):
        doc = {k: values.pop(k) for k in TAG_FIELDS if k in values}
    if hasattr(obj, "__dict__"):
        vars(obj).clear()
        vars(obj).update(values)
    else:
        for key, value in values.items():
            setattr(obj, key, value)
    for key, value in doc.items():
        setattr(obj, key, value)


def set_handle_state(obj, state) -> None:
    for name, value in state.items():
        setattr(obj, name, value)


# ------------------------------------------------------------------ roots

def new_scene_root():
    """A scene's root: ``.children`` is ``scene.groups``, ``._tags`` is
    ``scene.layers`` (with the default tag, as the Python field had)."""
    from core.layers import DEFAULT_LAYER, Layer
    root = NATIVE.doc_new_root()
    root._tags.append(Layer(DEFAULT_LAYER))
    return root
