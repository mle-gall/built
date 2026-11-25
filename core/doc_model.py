# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Python handles and transaction boundaries for the Rust document.

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
import sys
import weakref

import ingetrazo_core as NATIVE
from ingetrazo_core import Matrix4 as QMatrix4x4


def engine() -> str:
    """Document engine name for logs and probes."""
    return "Rust"


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


def _bind(value, key, which):
    """A matrix assigned to a placement → ``(view, data, built source)``.

    A plain ``QMatrix4x4`` BECOMES the view (its class is switched in
    place), so ``g.xform = m; m.translate(v)`` still moves ``g`` and
    ``g.xform is m`` still holds, as with the Python attribute. A view
    already bound elsewhere is copied: one matrix names one placement."""
    t = type(value)
    if t is QMatrix4x4:
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
            data = [float(v) for v in (value.data() if hasattr(value, "data") else value)]
        except (TypeError, ValueError):
            data = ()
        if len(data) != 16:
            raise TypeError(f"a placement matrix is a QMatrix4x4, not {t.__name__}")
        value = _make(key, which, data, None)
    value._doc_bind = (key, which)
    return value, value.data(), getattr(value, "_built_source", None)


NATIVE.doc_set_hooks(_make, _bind)


#: The tree invariant (one parent per placement, docs/rust-document-model.md
#: §2): ``raise`` refuses a second list with ``ValueError`` (the default),
#: ``warn`` reports it as a ``UserWarning`` and lets it be, ``off`` does not
#: look. ``INGETRAZO_DOC_PARENT`` chooses.
PARENT_MODES = {"off": 0, "warn": 1, "raise": 2}


def parent_mode(mode: str) -> str:
    """Set the tree invariant's mode; returns the previous one."""
    old = NATIVE.doc_parent_mode(PARENT_MODES[mode])
    return {v: k for k, v in PARENT_MODES.items()}[old]


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
# An edit goes to the document as a keyed patch (architecture review 11):
# the path from the attribute's top to the entry, and the entry's new value
# (``doc_attr_patch``), journalled as that entry's before and after, at
# once, inside a transaction or not: what Rust reads of the document is
# what Python sees. Each container knows its parent (weakly) and its key
# there, so its path is found without a scan of the value.
#
# A plain dict or list stored INTO one (``reg = data["links"] = {}``) is
# kept as that very object, as Python would, so edits made through it
# cannot be seen as they happen: it is *suspect*, and re-read whole at
# every boundary (:func:`flush`, which Rust readers call through
# ``doc::py::fresh`` when any is waiting). Once nothing but its parent holds
# it (and nothing outside holds a container inside it), the flush puts a
# tracked copy in its place, which nobody can tell apart, and it is no
# longer suspect: suspects live as long as a reference to them is kept.

_SUSPECT: dict = {}     # id(plain container) -> (anchor, weakref(parent), key, container)
_MISSING = object()


class _Anchor:
    """What every container of one tracked value points at: the binding
    (``(key, attr)``, or None once replaced) and a weak reference to the
    top container. Weak, so a tracked value is no reference cycle: a
    record's dicts go with their node by reference counting, not by the
    collector (a model's links registry is tens of thousands of them)."""

    __slots__ = ("bind", "root")

    def __init__(self):
        self.bind = None
        self.root = None


def _wrap(value, anchor, up=None, key=None):
    """``value`` as tracked containers of ``anchor``; ``up`` (a weak
    reference) and ``key``: where it sits in its parent."""
    t = type(value)
    if t is dict or t is TrackedDict:
        out = TrackedDict.__new__(TrackedDict)
    elif t is list or t is TrackedList:
        out = TrackedList.__new__(TrackedList)
    else:
        return value
    if anchor is None:
        anchor = _Anchor()
        anchor.root = weakref.ref(out)
    out._doc_root = anchor
    out._doc_up = up
    out._doc_key = key
    me = weakref.ref(out)
    if t is dict or t is TrackedDict:
        dict.__init__(out, ((k, _wrap(v, anchor, me, k)) for k, v in dict.items(value)))
    else:
        list.__init__(out, [_wrap(v, anchor, me) for v in list.__iter__(value)])
    return out


def _plain(value):
    t = type(value)
    if t is TrackedDict or t is dict:
        return {k: _plain(v) for k, v in dict.items(value)}
    if t is TrackedList or t is list:
        return [_plain(v) for v in list.__iter__(value)]
    return value


def _index(parent, node, key=None):
    """``node``'s place in ``parent`` (a key or an index), or _MISSING;
    ``key``: where it was put (a dict's key), looked at first."""
    if isinstance(parent, dict):
        if key is None and hasattr(node, "_doc_root"):
            key = node._doc_key
        if key is not None and dict.get(parent, key, _MISSING) is node:
            return key
        return next((k for k, v in dict.items(parent) if v is node), _MISSING)
    return next((i for i, v in enumerate(list.__iter__(parent)) if v is node), _MISSING)


def _path(node):
    """The keys from the attribute's top to ``node``, or None when it is
    no longer in the value (taken out, or its attribute replaced)."""
    path = []
    while True:
        up = node._doc_up
        if up is None:
            root = node._doc_root.root
            return path[::-1] if root is not None and root() is node else None
        parent = up()
        if parent is None:
            return None
        key = _index(parent, node)
        if key is _MISSING:
            return None
        path.append(key)
        node = parent


def _send(anchor, edits) -> None:
    """``edits`` ([(path, value, delete)]) into the document now; the whole
    value when a path does not lead into it any more."""
    root = anchor.root() if anchor.root is not None else None
    if root is None or anchor.bind is None:
        anchor.bind = None
        return
    try:
        bound = NATIVE.doc_attr_patch(*anchor.bind, root, edits)
    except (ValueError, TypeError):
        bound = NATIVE.doc_attr_changed(*anchor.bind, root)
    if not bound:
        anchor.bind = None               # replaced: a plain container now


def _edited(node, rel, value=None, delete=False) -> None:
    """``node`` edited: ``rel`` is the path below it (``[key]`` for one
    entry, ``[]`` for the node itself, as it is now)."""
    anchor = node._doc_root
    if anchor.bind is None:
        return
    path = _path(node)
    if path is not None:
        _send(anchor, [(path + rel, node if not rel and not delete else value, delete)])


def _stored(node, key, value):
    """``value`` going into ``node`` at ``key`` (None: a list's): a
    container this root does not track there is re-read at boundaries."""
    if isinstance(value, (dict, list)):
        tracked = getattr(value, "_doc_root", None) is node._doc_root
        here = tracked and value._doc_up is not None and value._doc_up() is node and value._doc_key == key
        if not here and node._doc_root.bind is not None:
            _SUSPECT[id(value)] = (node._doc_root, weakref.ref(node), key, value)
    return value


def flush() -> None:
    """Write back the suspect subtrees as they are now (plain containers
    stored into tracked ones; see above), and put a tracked copy in the
    place of each one nothing else holds any more."""
    if not _SUSPECT:
        return
    by_anchor = {}
    for sid, (anchor, parent_ref, key, value) in list(_SUSPECT.items()):
        parent = parent_ref()
        place = _MISSING if parent is None else _index(parent, value, key)
        path = None if place is _MISSING or anchor.bind is None else _path(parent)
        if path is None:
            del _SUSPECT[sid]
            continue
        by_anchor.setdefault(id(anchor), (anchor, []))[1].append((path + [place], value, False))
    for anchor, edits in by_anchor.values():
        _send(anchor, edits)
        if anchor.bind is None:
            for sid in [sid for sid, entry in _SUSPECT.items() if entry[0] is anchor]:
                del _SUSPECT[sid]
    by_anchor = edits = parent = value = None   # no name holds a suspect
    for sid in list(_SUSPECT):
        _promote(sid)


def _unshared(value) -> bool:
    """Whether no container inside ``value`` is held outside it (each one
    referenced once, by its parent)."""
    items = dict.values(value) if isinstance(value, dict) else list.__iter__(value)
    for v in items:
        # The parent's slot, this loop's name and the call's argument.
        if isinstance(v, (dict, list)) and (sys.getrefcount(v) > 3 or not _unshared(v)):
            return False
    return True


def _promote(sid) -> None:
    """Suspect ``sid`` tracked in place when nothing else holds it."""
    anchor, parent_ref, key, value = _SUSPECT.pop(sid)
    parent = parent_ref()
    # The parent's slot, the name ``value`` and the call's argument.
    if parent is None or sys.getrefcount(value) > 3 or not _unshared(value):
        _SUSPECT[sid] = (anchor, parent_ref, key, value)
        return
    place = _index(parent, value, key)
    if place is _MISSING:
        return
    tracked = _wrap(value, anchor, weakref.ref(parent), place if isinstance(parent, dict) else None)
    if isinstance(parent, dict):
        dict.__setitem__(parent, place, tracked)
    else:
        list.__setitem__(parent, place, tracked)


class TrackedDict(dict):
    """A document attribute's dict (see above)."""

    __slots__ = ("_doc_root", "_doc_up", "_doc_key", "__weakref__")

    @property
    def _doc_bind(self):
        return self._doc_root.bind

    def __setitem__(self, k, v):
        dict.__setitem__(self, k, _stored(self, k, v))
        _edited(self, [k], v)

    def __delitem__(self, k):
        dict.__delitem__(self, k)
        _edited(self, [k], delete=True)

    def clear(self):
        dict.clear(self)
        _edited(self, [])

    def pop(self, k, *a):
        had = k in self
        out = dict.pop(self, k, *a)
        if had:
            _edited(self, [k], delete=True)
        return out

    def popitem(self):
        k, v = dict.popitem(self)
        _edited(self, [k], delete=True)
        return k, v

    def setdefault(self, k, default=None):
        if k in self:
            return dict.__getitem__(self, k)
        dict.__setitem__(self, k, _stored(self, k, default))
        _edited(self, [k], default)
        return default

    def update(self, *a, **kw):
        for k, v in dict(*a, **kw).items():
            dict.__setitem__(self, k, _stored(self, k, v))
            _edited(self, [k], v)

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
    """A document attribute's list (see above). Setting an index or
    appending is one entry; any other edit writes the list again."""

    __slots__ = ("_doc_root", "_doc_up", "_doc_key", "__weakref__")

    @property
    def _doc_bind(self):
        return self._doc_root.bind

    def append(self, v):
        list.append(self, _stored(self, None, v))
        _edited(self, [len(self) - 1], v)

    def extend(self, items):
        list.extend(self, [_stored(self, None, v) for v in items])
        _edited(self, [])

    def insert(self, i, v):
        list.insert(self, i, _stored(self, None, v))
        _edited(self, [])

    def remove(self, v):
        list.remove(self, v)
        _edited(self, [])

    def pop(self, *a):
        out = list.pop(self, *a)
        _edited(self, [])
        return out

    def clear(self):
        list.clear(self)
        _edited(self, [])

    def sort(self, *a, **kw):
        list.sort(self, *a, **kw)
        _edited(self, [])

    def reverse(self):
        list.reverse(self)
        _edited(self, [])

    def __setitem__(self, i, v):
        if isinstance(i, slice):
            list.__setitem__(self, i, [_stored(self, None, x) for x in v])
            _edited(self, [])
            return
        list.__setitem__(self, i, _stored(self, None, v))
        _edited(self, [i % len(self) if i < 0 else i], v)

    def __delitem__(self, i):
        list.__delitem__(self, i)
        _edited(self, [])

    def __iadd__(self, items):
        self.extend(items)
        return self

    def __imul__(self, n):
        list.__imul__(self, n)
        _edited(self, [])
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
    out._doc_root.bind = (key, attr)
    return out


def _up(container):
    """A weak reference to ``container`` when it is tracked (a plain one
    held inside a tracked value is re-read whole: its children need none)."""
    return weakref.ref(container) if hasattr(container, "_doc_root") else None


def _patch(old, new, root):
    """``old`` (a container in a tracked attribute, tracked or plain) made
    equal to ``new`` (plain) in place, keeping the containers it holds
    where they still are containers of the same kind: a reference held to
    one of them sees the value too. Order follows ``new``."""
    if isinstance(old, dict):
        items = []
        for k, v in new.items():
            cur = dict.get(old, k, _patch)
            if isinstance(cur, dict) and type(v) is dict or (
                    isinstance(cur, list) and type(v) is list):
                _patch(cur, v, root)
                items.append((k, cur))
            else:
                items.append((k, _wrap(v, root, _up(old), k)))
        dict.clear(old)
        for k, v in items:
            dict.__setitem__(old, k, v)
        return
    out = []
    for i, v in enumerate(new):
        cur = list.__getitem__(old, i) if i < list.__len__(old) else None
        if isinstance(cur, dict) and type(v) is dict or (
                isinstance(cur, list) and type(v) is list):
            _patch(cur, v, root)
            out.append(cur)
        else:
            out.append(_wrap(v, root, _up(old)))
    list.__setitem__(old, slice(None), out)


def _refill(view, value) -> bool:
    """Rust → an undo, redo or rollback put ``value`` back into the
    attribute ``view`` is the tracked container of: refill it in place (a
    held reference sees the value, as a command's own undo editing the
    dict in place showed it). False when it cannot (another kind)."""
    if isinstance(view, TrackedDict) and type(value) is dict or (
            isinstance(view, TrackedList) and type(value) is list):
        _patch(view, value, view._doc_root)
        return True
    return False


def plain(value):
    """``value`` with every tracked container made a plain one."""
    return _plain(value)


NATIVE.doc_set_track_hook(_track)
NATIVE.doc_set_refill_hook(_refill)
NATIVE.doc_set_suspects(_SUSPECT, flush)


def attribute(name: str, track: bool = True):
    """The class attribute that keeps ``name`` in the document."""
    return NATIVE.DocAttr(name, track=track)


class Record(NATIVE.DocRecord):
    """Python editor handle for a document record."""

    __slots__ = ()

    def __reduce__(self):
        state = dict(getattr(self, "__dict__", ()))
        state.update(handle_state(self, self._record_fields))
        return new_handle, (type(self),), state

    def __setstate__(self, state):
        set_handle_state(self, state)

    def __copy__(self):
        out = type(self).__new__(type(self))
        for name in self._record_fields:
            setattr(out, name, getattr(self, name))
        if hasattr(self, "__dict__"):
            out.__dict__.update(self.__dict__)
        return out

    def __deepcopy__(self, memo):
        from copy import deepcopy
        out = type(self).__new__(type(self))
        memo[id(self)] = out
        for name in self._record_fields:
            setattr(out, name, deepcopy(getattr(self, name), memo))
        if hasattr(self, "__dict__"):
            out.__dict__.update(deepcopy(self.__dict__, memo))
        return out


def record_fields(*names, extra=()):
    """Bind the editor's declared fields to native journalled values."""
    def bind(cls):
        cls._record_fields = (names or tuple(cls.__dataclass_fields__)) + extra
        for name in extra:
            setattr(cls, name, None)
        for name in cls._record_fields:
            setattr(cls, name, attribute(name))
        return cls
    return bind


# ----------------------------------------------------------- transactions

def begin(document=None) -> int:
    """Open the step's transaction (or a savepoint in the open one). The
    suspect containers are written back first, so their edits made before
    it are not this step's."""
    return NATIVE.doc_begin(document)


def commit():
    """Close it: the outermost level returns its ``Transaction`` (with the
    step's in-place edits of tracked containers written back into it)."""
    if NATIVE.doc_depth() == 1:
        flush()
    return NATIVE.doc_commit()


def rollback() -> None:
    """Put back what the innermost level wrote (in-place edits of tracked
    containers included), and close it."""
    flush()
    NATIVE.doc_rollback()


def stats() -> dict:
    return NATIVE.doc_stats()


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
        type(value) is NATIVE.HandleList)


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
    if isinstance(obj, Layer):
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
