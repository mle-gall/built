# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust engine is ON unless Preferences turns it off (since 0.5.10),
and BUILT_RUST overrides the preference either way (tests, benches)."""
from __future__ import annotations

import importlib

import pytest

import core._native as _native
from built_core import general_settings


@pytest.fixture
def fresh(monkeypatch):
    """A newly imported core._native (``native`` is fixed per session)."""
    def make():
        _native.__dict__.pop("native", None)
        return importlib.reload(_native)
    saved = _native.__dict__.get("native", _native)
    yield make
    general_settings.set_value("rust_engine", "0")
    make()
    if saved is not _native:
        _native.native = saved      # what the rest of the session saw


def _forget_choice():
    from built_core.settings import settings as preferences
    st = preferences()
    for group in ("General/", "general/"):
        st.remove(group + "rust_engine")


def test_on_by_default(fresh, monkeypatch):
    """A user who never touched the setting gets the Rust engine."""
    if _native.available() is None:
        pytest.skip("native module not built")
    monkeypatch.delenv("BUILT_RUST", raising=False)
    _forget_choice()
    mod = fresh()
    assert mod.wanted() is True
    assert mod.native is not None
    assert mod.engine() == "Rust"


def test_on_by_default_without_the_module_is_python(fresh, monkeypatch):
    """... and a build that does not carry it simply runs on Python."""
    monkeypatch.delenv("BUILT_RUST", raising=False)
    _forget_choice()
    mod = fresh()
    monkeypatch.setattr(mod, "_module", None)
    mod.__dict__.pop("native", None)
    assert mod.native is None
    assert mod.engine() == "Python"


def test_obsolete_engine_preference_is_ignored(fresh, monkeypatch):
    monkeypatch.delenv("BUILT_RUST", raising=False)
    general_settings.set_value("rust_engine", "0")
    mod = fresh()
    assert mod.native is _native.available()
    assert mod.wanted()


def _reset_guard():
    _native._guard_disabled = False
    general_settings.set_value("rust_engine", "0")
    general_settings.set_value(_native.STARTING, "0")
    general_settings.set_value(_native.OFF_BY_GUARD, "")


def test_the_start_guard_watches_a_default_start(monkeypatch):
    """The guard must cover the users who never chose too: a default start
    marks itself under way like an explicit one."""
    monkeypatch.delenv("BUILT_RUST", raising=False)
    try:
        _forget_choice()
        general_settings.set_value(_native.STARTING, "0")
        general_settings.set_value(_native.OFF_BY_GUARD, "")
        assert _native.start_guard([]) is False
        assert general_settings.value(_native.STARTING) == "1"
        assert _native.start_guard([]) is False     # one that never drew
        assert _native.start_guard([]) is True      # two: back off
        assert not _native.wanted()
    finally:
        _reset_guard()


def test_preference_turns_it_on(fresh, monkeypatch):
    if _native.available() is None:
        pytest.skip("native module not built")
    monkeypatch.delenv("BUILT_RUST", raising=False)
    general_settings.set_value("rust_engine", "1")
    mod = fresh()
    assert mod.native is not None
    assert mod.engine() == "Rust"


@pytest.mark.parametrize("env,pref,on", [("0", "1", False), ("1", "0", True)])
def test_environment_overrides_preference(fresh, monkeypatch, env, pref, on):
    if on and _native.available() is None:
        pytest.skip("native module not built")
    monkeypatch.setenv("BUILT_RUST", env)
    general_settings.set_value("rust_engine", pref)
    mod = fresh()
    assert (mod.native is not None) is on


def test_two_starts_that_never_drew_turn_the_engine_off(monkeypatch):
    """The start guard: with the engine on, a start marks itself under way
    and the first drawn frame clears the mark; a start that finds two
    marks in a row (the last two never got to the window) goes back to
    the normal engine, saved, and says so once."""
    monkeypatch.delenv("BUILT_RUST", raising=False)
    try:
        general_settings.set_value("rust_engine", "1")
        general_settings.set_value(_native.STARTING, "0")
        general_settings.set_value(_native.OFF_BY_GUARD, "")
        assert _native.start_guard([]) is False
        assert general_settings.value(_native.STARTING) == "1"
        _native.start_ok()                         # the window drew
        assert general_settings.value(_native.STARTING) == "0"
        assert _native.start_guard([]) is False    # a good start again
        # ... this one dies before its first frame, and the next one too.
        assert _native.start_guard([]) is False
        assert _native.wanted()
        assert _native.start_guard([]) is True
        assert not _native.wanted()
        assert general_settings.value(_native.STARTING) == "0"
        assert _native.start_guard([]) is False    # off now: nothing to say
    finally:
        _reset_guard()


def test_one_start_that_never_drew_leaves_the_engine_on(monkeypatch):
    """A window closed while it loaded, a second window opened before the
    first had drawn: one start without a frame is not the engine failing.
    The engine stays on, and a start that does draw forgets it."""
    monkeypatch.delenv("BUILT_RUST", raising=False)
    try:
        general_settings.set_value("rust_engine", "1")
        general_settings.set_value(_native.STARTING, "0")
        general_settings.set_value(_native.OFF_BY_GUARD, "")
        assert _native.start_guard([]) is False    # never draws
        assert _native.start_guard([]) is False    # the next start: still on
        assert _native.wanted()
        _native.start_ok()                         # and this one draws
        assert general_settings.value(_native.STARTING) == "0"
        assert _native.start_guard([]) is False    # one more that never
        assert _native.start_guard([]) is False    # draws: the count began
        assert _native.wanted()   # again
    finally:
        _reset_guard()


def test_another_version_tries_the_engine_the_guard_turned_off(monkeypatch):
    """The guard turns the engine off for the version that failed, not for
    good: the next version starts with it on once more. What the user
    turned off in Preferences stays off."""
    import built_core.version
    monkeypatch.delenv("BUILT_RUST", raising=False)
    try:
        general_settings.set_value("rust_engine", "1")
        general_settings.set_value(_native.STARTING, "0")
        general_settings.set_value(_native.OFF_BY_GUARD, "")
        monkeypatch.setattr(built_core.version, "__version__", "9.9.1")
        assert _native.start_guard([]) is False
        assert _native.start_guard([]) is False
        assert _native.start_guard([]) is True     # off, by 9.9.1's guard
        assert general_settings.value(_native.OFF_BY_GUARD) == "9.9.1"
        assert _native.start_guard([]) is False    # same version: stays off
        assert not _native.wanted()
        monkeypatch.setattr(built_core.version, "__version__", "9.9.2")
        assert _native.start_guard([]) is False    # the update tries again
        assert _native.wanted()
        assert general_settings.value(_native.STARTING) == "1"
        assert general_settings.value(_native.OFF_BY_GUARD) == ""
        _native.start_ok()
    finally:
        _reset_guard()


def test_python_engine_flag_wins_for_one_run(monkeypatch):
    monkeypatch.delenv("BUILT_RUST", raising=False)
    try:
        general_settings.set_value("rust_engine", "1")
        assert _native.start_guard(["--python-engine"]) is False
        import os
        assert os.environ["BUILT_RUST"] == "0"
        assert general_settings.value("rust_engine") == "1"   # old key is untouched
        assert _native.wanted() is False
    finally:
        # start_guard wrote the variable itself, not through monkeypatch: a
        # monkeypatch.delenv here would record its "0" and put it back at
        # teardown, turning the engine off for every later test on the
        # same worker (the bake tests under xdist).
        import os
        os.environ.pop("BUILT_RUST", None)
        general_settings.set_value("rust_engine", "0")
        general_settings.set_value(_native.STARTING, "0")


def _asked_for_rust():
    import os
    return os.environ.get("BUILT_RUST", "0") != "0"


@pytest.mark.skipif(not _asked_for_rust(), reason="BUILT_RUST not set")
def test_asking_for_rust_without_the_module_fails():
    """BUILT_RUST=1 with no compiled module would quietly run every
    test on Python and report it as the Rust engine: refuse instead."""
    assert _native.available() is not None, (
        "BUILT_RUST=1 but built_core is not installed in this venv: "
        "maturin build --release -m rust/built_core/Cargo.toml, then "
        "pip install the wheel")


def _source_hash(src):
    """build.rs's fingerprint of the sources (FNV-1a 64, CR dropped)."""
    files = ["Cargo.toml", "Cargo.lock"] + sorted(
        p.relative_to(src).as_posix() for p in (src / "src").rglob("*")
        if p.suffix in {".rs", ".json"})
    h = 0xcbf29ce484222325
    for f in files:
        data = f.encode() + b"\0" + (src / f).read_bytes()
        for b in data.replace(b"\r", b""):
            h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


@pytest.mark.skipif(not _asked_for_rust(), reason="BUILT_RUST not set")
def test_the_installed_module_matches_its_sources():
    """A module built before the last change to rust/ measures yesterday's
    kernels: in a source checkout, it must have been built from these
    very sources."""
    from pathlib import Path
    mod = _native.available()
    if mod is None:
        pytest.skip("covered by the test above")
    src = Path(__file__).resolve().parents[1] / "rust" / "built_core"
    if not src.is_dir():
        pytest.skip("not a source checkout")
    built = getattr(mod, "SOURCE_HASH", None)
    assert built == _source_hash(src), (
        "built_core was built from other sources than rust/: "
        "run scripts/build_native.sh")
