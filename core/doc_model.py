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


#: The tree invariant (one parent per placement, docs/rust-document-model.md
#: §2): ``raise`` refuses a second list with ``ValueError`` (the default),
#: ``warn`` reports it as a ``UserWarning`` and lets it be, ``off`` does not
#: look. ``INGETRAZO_DOC_PARENT`` chooses.
PARENT_MODES = {"off": 0, "warn": 1, "raise": 2}


def parent_mode(mode: str) -> str:
    """Set the tree invariant's mode; returns the previous one."""
    if not ENABLED:
        return "off"
    old = NATIVE.doc_parent_mode(PARENT_MODES[mode])
    return {v: k for k, v in PARENT_MODES.items()}[old]


if ENABLED:
    parent_mode(os.environ.get("INGETRAZO_DOC_PARENT", "raise")
                if os.environ.get("INGETRAZO_DOC_PARENT") in PARENT_MODES
                else "raise")


# ---------------------------------------------------- tracked containers
#
# A custom attribute (``g.ifc``, ``g.ext``, ``g.text3d``), a scene table
# (``scene.folder_parents``, ``scene.plugin_data``…) is a document value. Read,
# a dict or a list comes out as a tracked container: a real ``dict`` /
# ``list`` (json, ``isinstance``, ``==`` and C code see one) whose in-place
# edits — at any depth — are written back to the document, so
# ``g.ext["link"]["x"] = 1`` is journalled and undoable as the assignment
# is. Once the attribute is assigned another value (or an undo puts back an
# old one) the container is a plain one: editing it no longer writes.
#
# Edits are written back at the next transaction boundary (:func:`flush`,
# run by :func:`begin` and :func:`commit`, so by every History step), not
# one by one: filling a table of 1,000 folders is one write. Python reads
# the container itself meanwhile. A plain dict or list stored INTO one
# (``reg = data["links"] = {}``) is kept as that very object, as Python
# would, and its root re-read at every boundary from then on (``suspect``):
# edits made through the plain reference are written back too.

_DIRTY: dict = {}       # id(root) -> root: edited since the last flush
_SUSPECT: dict = {}     # id(root) -> root: holds containers it does not track


def _wrap(value, root):
    t = type(value)
    if t is dict or t is TrackedDict:
        out = TrackedDict.__new__(TrackedDict)
        top = out if root is None else root
        out._doc_root = top
        dict.__init__(out, ((k, _wrap(v, top)) for k, v in dict.items(value)))
    elif t is list or t is TrackedList:
        out = TrackedList.__new__(TrackedList)
        top = out if root is None else root
        out._doc_root = top
        list.__init__(out, [_wrap(v, top) for v in list.__iter__(value)])
    else:
        return value
    return out


def _plain(value):
    t = type(value)
    if t is TrackedDict or t is dict:
        return {k: _plain(v) for k, v in dict.items(value)}
    if t is TrackedList or t is list:
        return [_plain(v) for v in list.__iter__(value)]
    return value


def _edited(node) -> None:
    root = node._doc_root
    if getattr(root, "_doc_bind", None) is not None:
        _DIRTY[id(root)] = root


def _stored(node, value):
    """``value`` going into ``node``: a container this root does not track
    makes the root suspect (see above)."""
    if isinstance(value, (dict, list)) and getattr(value, "_doc_root", None) is not node._doc_root:
        root = node._doc_root
        if getattr(root, "_doc_bind", None) is not None:
            _SUSPECT[id(root)] = root
    return value


def flush() -> None:
    """(Python document: nothing to do.) Write back every tracked container edited in place since the last
    call (and every suspect one); a container whose attribute was replaced
    meanwhile becomes a plain one."""
    if not ENABLED or not (_DIRTY or _SUSPECT):
        return
    roots = list(_DIRTY.values())
    _DIRTY.clear()
    roots.extend(r for k, r in _SUSPECT.items() if k not in {id(x) for x in roots})
    for root in roots:
        bind = getattr(root, "_doc_bind", None)
        if bind is None or not NATIVE.doc_attr_changed(bind[0], bind[1], root):
            root._doc_bind = None        # replaced: a plain container now
            _SUSPECT.pop(id(root), None)


class TrackedDict(dict):
    """A document attribute's dict (see above)."""

    __slots__ = ("_doc_root", "_doc_bind")

    def __setitem__(self, k, v):
        dict.__setitem__(self, k, _stored(self, v))
        _edited(self)

    def __delitem__(self, k):
        dict.__delitem__(self, k)
        _edited(self)

    def clear(self):
        dict.clear(self)
        _edited(self)

    def pop(self, *a):
        out = dict.pop(self, *a)
        _edited(self)
        return out

    def popitem(self):
        out = dict.popitem(self)
        _edited(self)
        return out

    def setdefault(self, k, default=None):
        if k in self:
            return dict.__getitem__(self, k)
        dict.__setitem__(self, k, _stored(self, default))
        _edited(self)
        return default

    def update(self, *a, **kw):
        for k, v in dict(*a, **kw).items():
            dict.__setitem__(self, k, _stored(self, v))
        _edited(self)

    def __ior__(self, other):
        self.update(other)
        return self

    def copy(self):
        return dict(self)

    def __copy__(self):
        return dict(self)

    def __deepcopy__(self, memo):
        from copy import deepcopy
        return deepcopy(_plain(self), memo)

    def __reduce__(self):
        return (dict, (_plain(self),))


class TrackedList(list):
    """A document attribute's list (see above)."""

    __slots__ = ("_doc_root", "_doc_bind")

    def append(self, v):
        list.append(self, _stored(self, v))
        _edited(self)

    def extend(self, items):
        list.extend(self, [_stored(self, v) for v in items])
        _edited(self)

    def insert(self, i, v):
        list.insert(self, i, _stored(self, v))
        _edited(self)

    def remove(self, v):
        list.remove(self, v)
        _edited(self)

    def pop(self, *a):
        out = list.pop(self, *a)
        _edited(self)
        return out

    def clear(self):
        list.clear(self)
        _edited(self)

    def sort(self, *a, **kw):
        list.sort(self, *a, **kw)
        _edited(self)

    def reverse(self):
        list.reverse(self)
        _edited(self)

    def __setitem__(self, i, v):
        if isinstance(i, slice):
            v = [_stored(self, x) for x in v]
        else:
            v = _stored(self, v)
        list.__setitem__(self, i, v)
        _edited(self)

    def __delitem__(self, i):
        list.__delitem__(self, i)
        _edited(self)

    def __iadd__(self, items):
        self.extend(items)
        return self

    def __imul__(self, n):
        list.__imul__(self, n)
        _edited(self)
        return self

    def copy(self):
        return list(self)

    def __copy__(self):
        return list(self)

    def __deepcopy__(self, memo):
        from copy import deepcopy
        return deepcopy(_plain(self), memo)

    def __reduce__(self):
        return (list, (_plain(self),))


#: The exact types a JSON-safe check accepts for an object and an array:
#: plain ones and the document's tracked ones.
JSON_DICTS = (dict, TrackedDict)
JSON_LISTS = (list, TrackedList)


def _track(key, attr, value):
    """Rust → the tracked container attribute ``attr`` of node ``key`` is
    handed out as."""
    out = _wrap(value, None)
    out._doc_bind = (key, attr)
    return out


def plain(value):
    """``value`` with every tracked container made a plain one."""
    return _plain(value)


if ENABLED:
    NATIVE.doc_set_track_hook(_track)


def attribute(name: str, track: bool = True):
    """The class attribute that keeps ``name`` in the document."""
    return NATIVE.DocAttr(name, track=track)


# ----------------------------------------------------------- transactions

def begin() -> int:
    """Open the step's transaction (or a savepoint in the open one). In-place
    edits of tracked containers made before it are written back first, so
    they are not this step's."""
    if not ENABLED:
        return 0
    if NATIVE.doc_depth() == 0:
        flush()
    return NATIVE.doc_begin()


def commit():
    """Close it: the outermost level returns its ``Transaction`` (with the
    step's in-place edits of tracked containers written back into it)."""
    if not ENABLED:
        return None
    if NATIVE.doc_depth() == 1:
        flush()
    return NATIVE.doc_commit()


def rollback() -> None:
    """Put back what the innermost level wrote (in-place edits of tracked
    containers included), and close it."""
    if ENABLED:
        flush()
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
