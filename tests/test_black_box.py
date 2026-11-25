# SPDX-License-Identifier: GPL-3.0-or-later
"""Each native library records its own Rust panics in the application crash log."""
import importlib

import pytest


@pytest.mark.parametrize("name", ["ingetrazo_core", "ingetrazo_render"])
def test_a_rust_panic_lands_in_the_log(tmp_path, name):
    module = importlib.import_module(name)
    path = tmp_path / "crash.log"
    assert module.install_panic_log(str(path))
    with pytest.raises(BaseException, match=f"evidence from {name}"):
        module._panic_for_test(f"evidence from {name}")
    text = path.read_text()
    assert f"Rust panic in {name}" in text
    assert f"evidence from {name}" in text
