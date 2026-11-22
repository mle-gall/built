# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare small-fixture imports across checkouts.

From each checkout run PYTHONPATH=. python /path/to/skp_fixture_snapshot.py
tests/data/fixture.skp > snapshot.json, then diff the two snapshots. Use the
same interpreter, backend and fixture filename for both. Timing and peak RSS
go to stderr, measured before snapshot serialization.
"""
import argparse
import hashlib
import itertools
import json
import os
import resource
import sys
import tempfile
import time
from pathlib import Path
from unittest.mock import patch


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    path = parser.parse_args().fixture
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtCore import QSettings
    from PySide6.QtWidgets import QApplication
    app = QApplication.instance() or QApplication([])
    with tempfile.TemporaryDirectory() as cache:
        os.environ["INGETRAZO_TEXTURE_CACHE"] = cache
        QSettings.setDefaultFormat(QSettings.IniFormat)
        for scope in (QSettings.UserScope, QSettings.SystemScope):
            QSettings.setPath(QSettings.IniFormat, scope, cache)
        from core import group, layers
        from core.scene import Scene
        from formats import igz, skp

        # The document includes freshly generated group and tag IDs in
        # addition to stable file identities. Make only the allocators repeatable.
        ids = (f"{i:016x}" for i in itertools.count(1))
        with patch.object(group, "new_uid", side_effect=ids), \
                patch.object(layers, "new_uid", side_effect=ids):
            scene = Scene()
            start = time.perf_counter()
            skp.load_skp(scene, path)
            elapsed = time.perf_counter() - start
        rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        print(f"{elapsed:.6f} s, {rss / (1024**2 if sys.platform == 'darwin' else 1024):.3f} MiB",
              file=sys.stderr)
        steps = igz.prepare_scene_steps(scene)
        try:
            while True:
                next(steps)
        except StopIteration as done:
            payload, geometry, large, _ = done.value
        if large:
            raise ValueError("Only small fixtures with inline geometry are supported")

        def normalized(value):
            if isinstance(value, dict):
                return {k: normalized(v) for k, v in value.items()}
            if isinstance(value, list):
                return [normalized(v) for v in value]
            if isinstance(value, str) and value.startswith(cache + os.sep):
                texture = Path(value)
                return [texture.name, hashlib.sha256(texture.read_bytes()).hexdigest()]
            return value

        print(json.dumps(normalized(payload), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
