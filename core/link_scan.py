# SPDX-License-Identifier: GPL-3.0-or-later
"""A document's linked files, read without opening it (the start page's
preview: "will this 40-second open work?").

The links are the ``"link": {"path": …, "rel": …}`` records of the placed
copies in the archive's ``document.json`` (or the whole file, when a small
document is saved as plain JSON; :mod:`core.links`). Unpacked
links save empty placements, so a host's ``document.json`` is small; it is
scanned with a pattern, not parsed. Each source is then looked for as
:func:`core.links.locate` does (next to the host first, then the absolute
path) and scanned in turn for its own links, to ``depth`` levels.

The state of a link is decided by the caller from what is on disk *now*
(:func:`state`): missing, changed since the host was saved, or found.
"""
from __future__ import annotations

import json
import os
import re
import zipfile
from pathlib import Path

#: Larger document.json entries are not read (a packed giant): no links listed.
MAX_DOC_BYTES = 64 * 1024 * 1024
#: At most this many links listed, nested ones included.
MAX_LINKS = 64

_LINK = re.compile(rb'"link"\s*:\s*\{\s*"path"\s*:\s*"((?:[^"\\]|\\.)*)"'
                   rb'(?:\s*,\s*"rel"\s*:\s*"((?:[^"\\]|\\.)*)")?')


def _text(raw: bytes) -> str:
    return json.loads(b'"' + raw + b'"')


def records(path) -> list[tuple[str, str]] | None:
    """(absolute path, relative path) of each linked source, each once, in
    the order the file places them. None: not a document we can read."""
    try:
        with open(path, "rb") as fh:
            zipped = fh.read(4) == b"PK\x03\x04"
            if not zipped:
                # A plain JSON document (a small one, or an older save).
                fh.seek(0)
                data = fh.read(MAX_DOC_BYTES + 1)
                if len(data) > MAX_DOC_BYTES:
                    return None
        if zipped:
            from ingetrazo_core import DocumentArchive
            with DocumentArchive(path) as archive:
                info = archive.getinfo("document.json")
                if info.file_size > MAX_DOC_BYTES:
                    return None
                data = archive.read(info)
    except (OSError, KeyError, zipfile.BadZipFile, ValueError, RuntimeError):
        return None
    seen, out = set(), []
    for m in _LINK.finditer(data):
        try:
            p = _text(m.group(1))
            rel = _text(m.group(2)) if m.group(2) else ""
        except ValueError:
            continue
        key = os.path.normcase(os.path.abspath(p))
        if key in seen:
            continue
        seen.add(key)
        out.append((p, rel))
    return out


def locate(host, path, rel) -> Path | None:
    """Where a source is: next to the host first (a moved project folder),
    then where it was."""
    if rel:
        cand = Path(host).parent / rel
        if cand.is_file():
            return cand.resolve()
    p = Path(path)
    return p if p.is_file() else None


def scan(host, depth: int = 3) -> list[dict]:
    """The links of ``host`` and theirs, depth first: dicts of ``path``
    (as recorded), ``found`` (the file found, or None), ``depth`` (0 for
    the host's own) and ``via`` (the file that links it)."""
    out: list = []
    seen = {os.path.normcase(os.path.abspath(str(host)))}

    def walk(file, level):
        if level >= depth or len(out) >= MAX_LINKS:
            return
        recs = records(file) or []
        for p, rel in recs:
            if len(out) >= MAX_LINKS:
                return
            found = locate(file, p, rel)
            out.append({"path": p, "found": str(found) if found else None, "depth": level,
                        "via": str(file)})
            key = os.path.normcase(os.path.abspath(str(found or p)))
            if found is not None and key not in seen:
                seen.add(key)
                walk(found, level + 1)
    walk(Path(host), 0)
    return out


def state(link: dict, host_mtime: float | None) -> str:
    """``missing``, ``changed`` (the source is newer than the file that
    links it was saved) or ``found``."""
    found = link.get("found")
    if not found:
        return "missing"
    try:
        src = os.stat(found).st_mtime
        ref = os.stat(link["via"]).st_mtime if link.get("via") else host_mtime
    except OSError:
        return "missing"
    if ref is not None and src > ref + 2.0:
        return "changed"
    return "found"
