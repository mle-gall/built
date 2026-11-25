"""The model's pixels, at rest and after scripted zooms, against recorded hashes.

A change to the renderer that is meant to cost less and draw the same
is checked here: the middle of the
window, clear of the chrome, must come out byte for byte as recorded.

The hashes belong to one GPU, system and window scale, so the record names
them and the test skips anywhere else. Two of the documents are the
generated benchmark cities (`python scripts/bench_models.py M XL`, not in
the repository); without them only the fixture is compared.

Record again after an intended change of the picture:

    BUILT_RECORD_FRAME_GOLDENS=1 BUILT_BINARY=... pytest tests/test_native_frame_goldens.py
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import struct
import subprocess
import zlib

import pytest

pytestmark = [pytest.mark.slow, pytest.mark.window]

ROOT = Path(__file__).resolve().parents[1]
GOLDENS = ROOT / "tests/fixtures/frame_goldens.json"
SIZE = (1440, 900)
# The part of the window no panel, rail or bar covers, in window points.
MIDDLE = (140, 90, 1040, 800)
MODELS = {
    "slab-and-cube": ROOT / "tests/fixtures/shell/slab-and-cube.igz",
    "city-M": ROOT / "benchmarks/models/city-M.igz",
    "city-XL": ROOT / "benchmarks/models/city-XL.igz",
}


def rows(png: bytes):
    """Width, height, bytes a pixel and the unfiltered rows of an 8-bit PNG."""
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    at, data, header = 8, b"", None
    while at < len(png):
        length, kind = struct.unpack(">I4s", png[at:at + 8])
        body = png[at + 8:at + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            data += body
        at += 12 + length
    width, height, depth, colour = header[:4]
    assert depth == 8 and colour in (2, 6), header
    step = 4 if colour == 6 else 3
    raw = zlib.decompress(data)
    stride = width * step
    out, previous = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            left = line[x - step] if x >= step else 0
            up = previous[x]
            corner = previous[x - step] if x >= step else 0
            if kind == 1:
                line[x] = (line[x] + left) & 255
            elif kind == 2:
                line[x] = (line[x] + up) & 255
            elif kind == 3:
                line[x] = (line[x] + (left + up) // 2) & 255
            elif kind == 4:
                p = left + up - corner
                a, b, c = abs(p - left), abs(p - up), abs(p - corner)
                line[x] = (line[x] + (left if a <= b and a <= c else up if b <= c else corner)) & 255
        out.append(line)
        previous = line
    return width, height, step, out


def middle_hash(path: Path) -> str:
    width, height, step, lines = rows(path.read_bytes())
    scale = width / SIZE[0]
    x0, y0, x1, y1 = (int(v * scale) for v in MIDDLE)
    digest = hashlib.sha256(f"{x1 - x0}x{y1 - y0}".encode())
    for line in lines[y0:y1]:
        digest.update(bytes(line[x0 * step:x1 * step]))
    return digest.hexdigest()[:24]


def script():
    steps = ["wait-open", "wait 1200", "menu view:zoom_extents", "wait 700", "capture rest"]
    for n, (x, y) in enumerate(((900, 300), (400, 600), (700, 430))):
        steps += [f"notch {x} {y} 1"] * 6 + ["wait 700", f"capture zoom{n}a"]
        steps += [f"notch {x} {y} -1"] * 9 + ["wait 700", f"capture zoom{n}b"]
        steps += [f"wheel {x} {y} 0 start"] + [f"wheel {x} {y} -5 move"] * 12 + [f"wheel {x} {y} 0 end", "wait 700", f"capture zoom{n}c"]
        steps += [f"pinch {x} {y} 0.2", "wait 700", f"capture zoom{n}d"]
    return steps + ["msaa off", "wait 700", "capture plain", "quit"]


def machine(first_capture: Path) -> dict:
    cpu = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip()
    width, height, _, _ = rows(first_capture.read_bytes())
    return {"system": platform.platform(), "cpu": cpu, "capture": [width, height]}


def model_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()[:24]


@pytest.mark.parametrize("name", list(MODELS))
def test_the_middle_of_the_window_is_drawn_as_recorded(name, tmp_path):
    binary = os.environ.get("BUILT_BINARY")
    if not binary or not Path(binary).is_file():
        pytest.skip("set BUILT_BINARY to the native built_app binary")
    model = MODELS[name]
    if not model.is_file():
        pytest.skip(f"{model.relative_to(ROOT)} is not generated (scripts/bench_models.py)")
    recording = os.environ.get("BUILT_RECORD_FRAME_GOLDENS") == "1"
    goldens = json.loads(GOLDENS.read_text()) if GOLDENS.is_file() else {"machine": None, "models": {}}
    recorded = goldens["models"].get(name)
    if not recording:
        if recorded is None:
            pytest.skip(f"no recorded frames for {name}")
        if recorded["model"] != model_hash(model):
            pytest.skip(f"{model.name} is not the document the frames were recorded from")
    steps = tmp_path / "steps"
    steps.write_text("\n".join(script()) + "\n")
    home, settings, shots = tmp_path / "home", tmp_path / "settings", tmp_path / "shots"
    for folder in (home, settings, shots):
        folder.mkdir()
    subprocess.run([binary, "--root", ROOT, "--venv", ROOT / ".venv", "--new-instance", "--shots", shots,
                    "--script", steps, "--size", f"{SIZE[0]}x{SIZE[1]}", model],
                   env=dict(os.environ, HOME=str(home), CFFIXED_USER_HOME=str(home), BUILT_SETTINGS_DIR=str(settings),
                            BUILT_BACKGROUND="1", BUILT_ANSWERS="escape"),
                   capture_output=True, text=True, timeout=600, check=True)
    captures = sorted(shots.glob("*.png"))
    assert len(captures) == 14, [c.name for c in captures]
    here = machine(captures[0])
    seen = {c.stem: middle_hash(c) for c in captures}
    if recording:
        if goldens["machine"] not in (None, here):
            goldens = {"machine": None, "models": {}}       # another machine's record: start over
        goldens["machine"] = here
        goldens["models"][name] = {"model": model_hash(model), "frames": seen}
        GOLDENS.write_text(json.dumps(goldens, indent=1, sort_keys=True) + "\n")
        return
    if goldens["machine"] != here:
        pytest.skip(f"frames were recorded on {goldens['machine']}, this is {here}")
    # The views differ from each other: a blank or stuck frame is not a match.
    assert len(set(seen.values())) >= 10
    assert {k: v for k, v in seen.items() if recorded["frames"].get(k) != v} == {}
