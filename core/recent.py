# SPDX-License-Identifier: GPL-3.0-or-later
"""The recent documents and projects, pinnable, missing ones kept for
relocation: one store, ``home/recent_<kind>`` (JSON rows), which the start
page, File ▸ Open Recent and the native menu all read. The list rules live
in ``rust/ingetrazo_core/src/recents.rs``; this module reads and writes the
settings.

The app used to keep a second, plain path list (``recent_files``). A store
that has no rich list yet reads it, and the first write drops it, so the
user's recents survive and a cleared list stays cleared."""
import json
from pathlib import Path
from core.settings import settings as native_settings

from ingetrazo_core import recent_forget, recent_menu, recent_remember, recent_rows

LIMIT = 20
#: File ▸ Open Recent shows this many.
MENU = 10
_LEGACY = 'recent_files'


def _key(kind):
    return 'home/recent_' + kind


def _rows_json(kind, st):
    stored = st.value(_key(kind)) if st.contains(_key(kind)) else None
    legacy = []
    if kind == 'files':
        old = st.value(_LEGACY, []) or []
        legacy = [str(p) for p in ([old] if isinstance(old, str) else old)]
    return recent_rows(stored if isinstance(stored, str) else None, legacy)


def _store(kind, st, rows_json):
    st.setValue(_key(kind), rows_json)
    if kind == 'files':
        st.remove(_LEGACY)


def entries(kind='files', settings=None):
    return json.loads(_rows_json(kind, settings or native_settings()))


def menu(settings=None, limit=MENU):
    """File ▸ Open Recent: the recent documents still on disk."""
    return recent_menu(_rows_json('files', settings or native_settings()), limit)


def remember(path, pinned=None, settings=None, replace=None):
    st = settings or native_settings()
    path = str(Path(path).resolve())
    kind = 'projects' if Path(path).suffix.lower() in ('.builtproject', '.igzproj') else 'files'
    _store(kind, st, recent_remember(_rows_json(kind, st), path, None if pinned is None else bool(pinned),
                                     None if replace is None else str(replace)))
    if kind == 'files' and pinned is None:
        # Opened or saved, not a pin: the start page's «Continue».
        st.setValue('home/last_file', path)


def forget(paths, settings=None):
    """Take ``paths`` (one or several) off the recent lists: the files
    themselves are left alone. The start page's «Continue» and what it
    remembered of them go too."""
    st = settings or native_settings()
    if isinstance(paths, (str, Path)):
        paths = [paths]
    gone = {str(p) for p in paths}
    gone |= {str(Path(p).resolve()) for p in paths}
    for kind in ('files', 'projects'):
        rows = _rows_json(kind, st)
        kept = recent_forget(rows, sorted(gone))
        if kept != rows:
            _store(kind, st, kept)
    if st.value('home/last_file', '') in gone:
        st.remove('home/last_file')
    try:
        table = json.loads(st.value('home/meta', '{}'))
    except (TypeError, ValueError):
        table = {}
    if isinstance(table, dict) and gone & set(table):
        st.setValue('home/meta', json.dumps({k: v for k, v in table.items() if k not in gone}))


def clear(settings=None):
    """File ▸ Open Recent ▸ Clear list: the documents' rows go, the ones
    pinned on the start page stay."""
    st = settings or native_settings()
    forget([r['path'] for r in entries('files', st) if not r.get('pinned')], st)
    _store('files', st, _rows_json('files', st))


def query(settings=None):
    return {kind: entries(kind, settings) for kind in ('files', 'projects')}


def meta(path, settings=None):
    """What the start page remembers of a document from when it was open
    (``ingetrazo_render.wgpu_start.note``): project, links, origin, unfiled, problems,
    workspace. Empty when never noted."""
    st = settings or native_settings()
    try:
        table = json.loads(st.value('home/meta', '{}'))
    except (TypeError, ValueError):
        table = {}
    row = table.get(str(Path(path).resolve())) if isinstance(table, dict) else None
    return row if isinstance(row, dict) else {}


def note(path, data, settings=None):
    """Merge ``data`` into what is remembered of ``path`` (bounded like the
    recent lists, the newest kept)."""
    st = settings or native_settings()
    try:
        table = json.loads(st.value('home/meta', '{}'))
    except (TypeError, ValueError):
        table = {}
    if not isinstance(table, dict):
        table = {}
    key = str(Path(path).resolve())
    row = dict(table.pop(key, None) or {})
    row.update({k: v for k, v in data.items() if v is not None})
    table[key] = row
    keep = list(table.items())[-LIMIT * 2:]
    st.setValue('home/meta', json.dumps(dict(keep)))
