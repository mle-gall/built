# SPDX-License-Identifier: GPL-3.0-or-later
"""Bounded, pinnable recent paths retain missing entries for relocation."""
import json
from pathlib import Path
from PySide6.QtCore import QSettings

LIMIT = 20


def entries(kind='files', settings=None):
    st = settings or QSettings()
    try:
        rows = json.loads(st.value('home/recent_' + kind, '[]'))
    except (TypeError, ValueError):
        rows = []
    if not rows and kind == 'files':
        old = st.value('recent_files', []) or []
        rows = [{'path': p, 'pinned': False} for p in ([old] if isinstance(old, str) else old)]
    return [r for r in rows if isinstance(r, dict) and isinstance(r.get('path'), str)][:LIMIT]


def remember(path, pinned=None, settings=None, replace=None):
    st = settings or QSettings()
    path = str(Path(path).resolve())
    kind = 'projects' if Path(path).suffix.lower() == '.igzproj' else 'files'
    rows = entries(kind, st)
    previous = next((r for r in rows if r['path'] in (path, replace)), {})
    row = {'path': path, 'pinned': previous.get('pinned', False) if pinned is None else bool(pinned)}
    rows = [row] + [r for r in rows if r['path'] not in (path, replace)]
    rows.sort(key=lambda r: not r.get('pinned', False))
    st.setValue('home/recent_' + kind, json.dumps(rows[:LIMIT]))
    if kind == 'files' and pinned is None:
        # Opened or saved, not a pin: the start page's «Continue».
        st.setValue('home/last_file', path)


def query(settings=None):
    return {kind: entries(kind, settings) for kind in ('files', 'projects')}


def meta(path, settings=None):
    """What the start page remembers of a document from when it was open
    (``views.wgpu_start.note``): project, links, origin, unfiled, problems,
    workspace. Empty when never noted."""
    st = settings or QSettings()
    try:
        table = json.loads(st.value('home/meta', '{}'))
    except (TypeError, ValueError):
        table = {}
    row = table.get(str(Path(path).resolve())) if isinstance(table, dict) else None
    return row if isinstance(row, dict) else {}


def note(path, data, settings=None):
    """Merge ``data`` into what is remembered of ``path`` (bounded like the
    recent lists, the newest kept)."""
    st = settings or QSettings()
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
