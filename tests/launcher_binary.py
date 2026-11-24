"""Run the checkout's native launcher under a private preference profile."""
import os
from pathlib import Path

import pytest

from scripts import isolated_launch


def run(*args):
    root = Path(__file__).resolve().parents[1]
    name = "built.exe" if os.name == "nt" else "built"
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "rust/built_app/target"))
    binary = Path(os.environ.get("BUILT_BINARY", target / "release" / name))
    if not binary.exists() and "BUILT_BINARY" not in os.environ:
        binary = target / "debug" / name
    if not binary.is_file():
        pytest.skip("build built_app to exercise the native launcher")
    return isolated_launch.run([str(binary), *args], text=True, capture_output=True,
                               timeout=90, env=dict(os.environ, BUILT_ROOT=str(root),
                                                    BUILT_BACKGROUND="1"))
