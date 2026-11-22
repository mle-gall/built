# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Auto-save recovery slots — the usual General ▸ Auto-save, our way.

One recovery slot per document in the user data dir (NOT beside the
document: the project folder may live in a syncing drive — pCloud has
truncated mid-write files there before — and recovery files are session
state, not project files). The slot's name is the document's stem plus a
hash of its absolute path, so two ``casa.igz`` in different folders never
share a slot; a document with no path yet uses the ``untitled`` slot.

The invariant that makes recovery detection trivial: **a slot exists only
between a change and the next clean save/close.** The main window clears it
on save, on close, and when a document is discarded — so a slot found on
disk means a session that never got to say goodbye (crash, power cut), and
its mere existence is the "offer to recover" signal.
"""
from __future__ import annotations

import hashlib
import os
import sys
from formats import built
from pathlib import Path


def _app_data_base() -> Path:
    """The per-user data folder the slots live under.

    On Windows the base is built from the process's ``APPDATA`` variable
    whenever one is set (#1180): ``QStandardPaths`` resolves the roaming
    folder through the OS and never looks at the variable, so a caller
    that points ``APPDATA`` somewhere else — the test suite points it at
    a sandbox — still wrote its slots into the real profile. The
    ``org/app`` suffix Qt appends is kept, so a normal run (whose
    ``APPDATA`` IS the roaming folder) lands exactly where it always has.
    The suffix is the organization and application name ``main.py``
    registers — asked of ``QCoreApplication`` instead, a caller that
    never set them (a test runner) would get a name derived from
    ``argv[0]`` and a different folder again.
    """
    if sys.platform == "win32" and os.environ.get("APPDATA"):
        return Path(os.environ["APPDATA"]) / "IngeTrazo" / "IngeTrazo"
    from PySide6.QtCore import QStandardPaths
    return Path(QStandardPaths.writableLocation(
        QStandardPaths.AppDataLocation) or str(Path.home() / ".ingetrazo"))


def autosave_dir() -> Path:
    d = _app_data_base() / "autosave"
    d.mkdir(parents=True, exist_ok=True)
    return d


def slot_for(path: Path | None, untitled: int = 1) -> Path:
    """The recovery slot for a document (``None`` = an unsaved document).

    With several projects open in tabs (#1066) each untitled one has its
    own slot, named untitled for the first document and untitled-N after that."""
    if path is None:
        n = max(1, int(untitled or 1))
        stem = "untitled" if n == 1 else f"untitled-{n}"
        return autosave_dir() / (stem + built.BUILT_SUFFIX)
    p = Path(path).absolute()
    digest = hashlib.sha1(str(p).encode("utf-8")).hexdigest()[:8]
    return autosave_dir() / f"{p.stem}-{digest}{built.BUILT_SUFFIX}"


def write(scene, path: Path | None, untitled: int = 1) -> Path:
    """Save ``scene`` into the document's slot; returns the slot path."""
    from formats import igz
    slot = slot_for(path, untitled)
    igz.save_scene(scene, slot, backup=False)
    return slot


def pending(path: Path | None, untitled: int = 1) -> Path | None:
    """The slot file, if an interrupted session left one."""
    slot = slot_for(path, untitled)
    legacy = slot.with_suffix(".igz")
    if slot.is_file():
        return slot
    return legacy if legacy.is_file() else None


def pending_untitled() -> list:
    """Every untitled slot an interrupted session left, as ``(n, slot)``
    pairs in slot order."""
    import re
    found = {}
    for slot in autosave_dir().glob("untitled*"):
        m = re.fullmatch(r"untitled(?:-(\d+))?", slot.stem)
        if m and slot.suffix in built.DOCUMENT_SUFFIXES and slot.is_file():
            n = int(m.group(1) or 1)
            if n not in found or built.is_built(slot):
                found[n] = slot
    return sorted(found.items())


#: How many retired slots to keep in ``autosave_dir()/descartados``.
KEEP_DISCARDED = 20


def discarded_dir() -> Path:
    d = autosave_dir() / "descartados"
    d.mkdir(parents=True, exist_ok=True)
    return d


_slot_epochs: dict[Path, int] = {}


def slot_epoch(slot: Path) -> int:
    """UI-thread generation: prevents a pending writer reviving a retired slot."""
    return _slot_epochs.get(slot, 0)


def clear(path: Path | None, untitled: int = 1) -> None:
    """Retire the document's slot (clean save / clean close / discarded).

    The slot is MOVED into ``descartados/`` with a timestamp, never deleted:
    "Quit without saving" answered by mistake used to erase the only copy
    of a whole afternoon (Marco, 2026-09-03 — five hours of a bench model
    gone at the close dialog). The invariant that drives recovery stays —
    no slot in the live folder means a clean goodbye — while the retired
    copy waits in the folder Archivo ▸ Recover a discarded auto-save… opens.
    Only the newest :data:`KEEP_DISCARDED` are kept."""
    slot = slot_for(path, untitled)
    _slot_epochs[slot] = slot_epoch(slot) + 1
    for candidate in (slot, slot.with_suffix(".igz")):
        if candidate.is_file():
            _retire(candidate)


def _retire(slot):
    import datetime
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    target = discarded_dir() / f"{slot.stem}-{stamp}{slot.suffix}"
    try:
        slot.replace(target)
    except OSError:
        slot.unlink(missing_ok=True)
        return
    kept = sorted((p for p in discarded_dir().iterdir() if p.suffix in built.DOCUMENT_SUFFIXES),
                  key=lambda p: p.stat().st_mtime, reverse=True)
    for old in kept[KEEP_DISCARDED:]:
        try:
            old.unlink()
        except OSError:
            pass
