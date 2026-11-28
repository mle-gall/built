#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Play a demo beat in a clean Built window, the same way every time.

A beat is a short text file of human-sized steps (docs/recording-beats.md):
point the mouse somewhere over half a second, press a tool's key, click,
type a value one character at a time, orbit with the middle button. This
script turns it into the app's own script steps (`built --script`, see
rust/built_app/src/script.rs) with human-like, seeded timing, and runs
Built with them in a visible window on a fresh, private profile. Every
step goes through the window's real input path: the same tools, inference
and value field as a person's hands. The keystroke overlay shows what is
played; camera moves glide.

    scripts/play_beat.py docs/beats/box-and-cylinder.beat      # play it
    scripts/play_beat.py BEAT --print                          # the steps only
    scripts/play_beat.py BEAT --background --shots DIR          # tests: behind,
                                                               # with captures

Record with your own screen recorder; this script never captures the
screen. `--shots` only asks the app for its own frames, for tests.
"""
from __future__ import annotations

import argparse
import math
import os
import random
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FRAME_MS = 16            # one pointer move per frame while the mouse travels
BUTTONS = {"left", "right", "middle"}


class BeatError(ValueError):
    pass


class Beat:
    """The compiled beat: script steps, the window size and the model."""

    def __init__(self, seed: int = 7):
        self.steps: list[str] = []
        self.size = (1600, 1000)
        self.model: str | None = None
        self.addons: list[str] = []
        self.keystrokes = True
        self.at = (800.0, 500.0)
        self.rng = random.Random(seed)

    # ---- timing ---------------------------------------------------------
    def wait(self, ms: float):
        ms = int(round(ms))
        if ms > 0:
            self.steps.append(f"wait {ms}")

    def jitter(self, ms: float, spread: float = 0.25) -> float:
        """A human's `ms`: up to `spread` either way, the same every run."""
        return ms * (1 + self.rng.uniform(-spread, spread))

    def travel_ms(self, to) -> float:
        """How long a hand takes to point there (Fitts-like, 250–900 ms)."""
        d = math.dist(self.at, to)
        return min(900.0, 250.0 + 120.0 * math.log2(1 + d / 40.0)) if d > 1 else 0.0

    # ---- pointer --------------------------------------------------------
    def travel(self, to, ms: float | None = None):
        """Move the pointer to `to` along an eased path, one move a frame."""
        ms = self.travel_ms(to) if ms is None else ms
        frames = max(1, int(ms / FRAME_MS))
        (x0, y0), (x1, y1) = self.at, to
        # A hand's path bows a little; the same bow every run.
        bow = self.rng.uniform(-0.06, 0.06) * math.dist(self.at, to)
        nx, ny = (-(y1 - y0), x1 - x0)
        norm = math.hypot(nx, ny) or 1.0
        for i in range(1, frames + 1):
            t = i / frames
            e = t * t * (3 - 2 * t)                     # ease in and out
            b = bow * math.sin(math.pi * t)
            x = x0 + (x1 - x0) * e + nx / norm * b
            y = y0 + (y1 - y0) * e + ny / norm * b
            self.steps.append(f"move {x:.1f} {y:.1f}")
            self.wait(FRAME_MS)
        self.at = (float(x1), float(y1))

    def click(self, to, button="left", mods="", ms=None):
        self.travel(to, ms)
        self.wait(self.jitter(140))                     # settle on the target
        x, y = to
        self.steps.append(f"press {x:.1f} {y:.1f} {button} {mods}".rstrip())
        self.wait(self.jitter(70, 0.2))
        self.steps.append(f"release {x:.1f} {y:.1f} {button} {mods}".rstrip())
        self.wait(self.jitter(180))

    def drag(self, start, end, ms=None, button="left", mods=""):
        self.travel(start)
        self.wait(self.jitter(120))
        x, y = start
        self.steps.append(f"press {x:.1f} {y:.1f} {button} {mods}".rstrip())
        self.wait(60)
        self.travel(end, ms if ms is not None else max(400.0, self.travel_ms(end)))
        x, y = end
        self.steps.append(f"release {x:.1f} {y:.1f} {button} {mods}".rstrip())
        self.wait(self.jitter(200))

    # ---- keyboard -------------------------------------------------------
    def key(self, name: str, mods: str = ""):
        self.steps.append(f"key {name} {mods}".rstrip())
        self.wait(self.jitter(260))

    def type(self, text: str):
        for ch in text:
            if ch == "#" or ch.isspace():
                raise BeatError(f"type: {ch!r} cannot be typed in a beat")
            self.steps.append(f"type {ch}")
            self.wait(self.jitter(115, 0.35))
        self.wait(self.jitter(220))


def _num(word: str, line: int) -> float:
    try:
        return float(word)
    except ValueError:
        raise BeatError(f"line {line}: {word!r} is not a number") from None


def _split_tail(words, line):
    """Trailing [ms] [button] [mods] of a pointer verb."""
    ms, button, mods = None, None, ""
    for w in words:
        if w in BUTTONS:
            button = w
        elif w.replace(".", "", 1).isdigit():
            ms = float(w)
        elif all(m in {"shift", "cmd", "alt", "ctrl"} for m in w.split("+")):
            mods = w
        else:
            raise BeatError(f"line {line}: unexpected {w!r}")
    return ms, button, mods


def compile_beat(text: str, seed: int = 7) -> Beat:
    """The beat's text as script steps (see docs/recording-beats.md)."""
    beat = Beat(seed)
    body: list[tuple[int, list[str]]] = []
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        words = shlex.split(line)
        verb = words[0]
        if verb == "size":
            beat.size = (int(_num(words[1], n)), int(_num(words[2], n)))
        elif verb == "model":
            beat.model = words[1]
        elif verb == "addon":
            beat.addons += [w for w in words[1:] if w not in beat.addons]
        elif verb == "keystrokes":
            beat.keystrokes = words[1:] != ["off"]
        else:
            body.append((n, words))
    beat.at = (beat.size[0] / 2, beat.size[1] / 2)
    beat.wait(900)                                      # the window settles
    if beat.model is None:
        # A fresh profile opens on the Start page: Enter makes a blank model.
        beat.steps.append("key enter")
        beat.wait(800)
    if beat.keystrokes:
        beat.steps.append("menu show_keystrokes")
    beat.steps.append(f"move {beat.at[0]:.1f} {beat.at[1]:.1f}")
    beat.wait(400)
    for n, words in body:
        verb, args = words[0], words[1:]
        if verb == "pause":
            beat.wait(_num(args[0], n))
        elif verb == "key":
            beat.key(args[0], args[1] if len(args) > 1 else "")
        elif verb == "type":
            beat.type(" ".join(args))
        elif verb == "point":
            ms, _, _ = _split_tail(args[2:], n)
            beat.travel((_num(args[0], n), _num(args[1], n)), ms)
            beat.wait(beat.jitter(120))
        elif verb == "click":
            ms, button, mods = _split_tail(args[2:], n)
            beat.click((_num(args[0], n), _num(args[1], n)), button or "left", mods, ms)
        elif verb == "drag":
            ms, button, mods = _split_tail(args[4:], n)
            beat.drag((_num(args[0], n), _num(args[1], n)), (_num(args[2], n), _num(args[3], n)),
                      ms, button or "left", mods)
        elif verb == "orbit":
            # The middle button dragged from where the pointer is.
            dx, dy = _num(args[0], n), _num(args[1], n)
            ms, _, _ = _split_tail(args[2:], n)
            start = beat.at
            beat.drag(start, (start[0] + dx, start[1] + dy), ms or 900.0, "middle")
        elif verb == "zoom":
            # Mouse-wheel notches at the pointer, one every 90 ms or so.
            notches = int(_num(args[0], n))
            for _ in range(abs(notches)):
                beat.steps.append(f"notch {beat.at[0]:.1f} {beat.at[1]:.1f} {1 if notches > 0 else -1}")
                beat.wait(beat.jitter(90))
            beat.wait(beat.jitter(200))
        elif verb == "menu":
            beat.steps.append(f"menu {args[0]}")
            beat.wait(beat.jitter(300))
        elif verb == "capture":
            beat.steps.append(f"capture {args[0]}")
        elif verb == "step":
            beat.steps.append(" ".join(args))           # a raw script step
        else:
            raise BeatError(f"line {n}: unknown verb {verb!r}")
    return beat


ADDON_PLUGIN = """\
# Installs the beat's add-ons into this private profile's My library.
PACKS = {packs!r}


def setup(app):
    from built_render import wgpu_library
    for path in PACKS:
        report = wgpu_library.import_pack(app.window, path, "mine")
        print(f"add-on {{path.rsplit('/', 1)[-1]}}: {{report['ok']}} rows in My library", flush=True)
"""


def install_addons(settings: Path, packs: list[str]) -> Path | None:
    """Build each add-on from addons/ and import it into My library at start,
    the same way the tests do (a start-up plugin in the private profile)."""
    if not packs:
        return None
    sys.path.insert(0, str(ROOT))
    from scripts.build_addons import ADDONS, build
    plugins = settings / "data" / "built" / "plugins"
    plugins.mkdir(parents=True, exist_ok=True)
    missing = [p for p in packs if not (ADDONS / p / "pack.json").is_file()]
    if missing:
        raise BeatError(f"unknown add-on {', '.join(missing)} (see addons/)")
    paths = [str(build(ADDONS / p, settings / "addons")) for p in packs]
    plugin = plugins / "beat_addons.py"
    plugin.write_text(ADDON_PLUGIN.format(packs=paths))
    return plugin


def _binary() -> Path:
    env = os.environ.get("BUILT_BINARY")
    return Path(env) if env else ROOT / "rust/built_app/target/release/built"


def play(beat: Beat, *, background: bool, shots: Path | None, quit_at_end: bool,
         binary: Path | None = None, timeout: float | None = None) -> subprocess.CompletedProcess:
    """Run Built on the compiled steps with a fresh, private profile."""
    binary = binary or _binary()
    if not binary.is_file():
        raise SystemExit(f"no Built binary at {binary} (build it or set BUILT_BINARY)")
    with tempfile.TemporaryDirectory(prefix="built-beat-") as folder:
        folder = Path(folder)
        steps = list(beat.steps)
        if beat.model:
            # A copy: the beat never touches the file it starts from.
            src = (ROOT / beat.model) if not Path(beat.model).is_absolute() else Path(beat.model)
            model = folder / src.name
            model.write_bytes(src.read_bytes())
            steps.insert(0, f"open {model}")
        if quit_at_end:
            steps.append("quit")
        script = folder / "beat.txt"
        script.write_text("\n".join(steps) + "\n")
        settings = folder / "settings"
        home = folder / "home"
        settings.mkdir()
        home.mkdir()
        install_addons(settings, beat.addons)
        env = dict(os.environ)
        env.update(
            BUILT_SETTINGS_DIR=str(settings), BUILT_LOG_DIR=str(settings / "logs"),
            HOME=str(home), CFFIXED_USER_HOME=str(home),
            XDG_CONFIG_HOME=str(settings), XDG_DATA_HOME=str(settings / "data"),
            # Camera moves glide as they do for a person (a script jumps).
            BUILT_CAMERA_MOTION="1",
            BUILT_BACKGROUND="1" if background else "0",
        )
        args = [str(binary), "--new-instance", "--root", str(ROOT),
                "--venv", str(ROOT / ".venv"), "--settings", str(settings),
                "--size", f"{beat.size[0]}x{beat.size[1]}", "--script", str(script)]
        if not background:
            args.append("--present")
        if shots:
            args += ["--shots", str(shots)]
        return subprocess.run(args, env=env, capture_output=background, text=True, timeout=timeout)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("beat", type=Path)
    ap.add_argument("--print", action="store_true", help="print the script steps and stop")
    ap.add_argument("--background", action="store_true",
                    help="play behind other windows and quit at the end (tests)")
    ap.add_argument("--shots", type=Path, help="the app's own frames for `capture` steps (tests)")
    ap.add_argument("--quit", action="store_true", help="quit when the beat ends")
    ap.add_argument("--seed", type=int, default=7, help="another, still repeatable, human timing")
    ap.add_argument("--addon", action="append", default=[], metavar="PACK",
                    help="install this add-on from addons/ into My library first (repeatable)")
    a = ap.parse_args(argv)
    try:
        beat = compile_beat(a.beat.read_text(), a.seed)
    except BeatError as e:
        print(f"{a.beat}: {e}", file=sys.stderr)
        return 2
    beat.addons += [p for p in a.addon if p not in beat.addons]
    if a.print:
        print("\n".join(beat.steps))
        return 0
    result = play(beat, background=a.background, shots=a.shots, quit_at_end=a.quit or a.background)
    if a.background:
        sys.stdout.write(result.stdout)
        sys.stderr.write(result.stderr)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
