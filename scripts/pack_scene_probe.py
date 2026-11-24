#!/usr/bin/env python3
"""Headless wall-clock pack split and content oracle, without cProfile overhead.

Run with the worktree's PYTHONPATH. An optional
--reference-source (a saved wgpu_scene.py) measures the old Python preparation
against the same native wheel. Digests stream bytes to avoid an extra resident
vertex allocation. The document is read only; recovery prompts are disabled.
Inclusive stage times overlap (_material_columns is inside _source_arrays).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import sys
import time

import numpy as np


def digest(value):
    return hashlib.sha256(memoryview(np.ascontiguousarray(value)).cast('B')).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('file', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('--reference-source', type=Path)
    args = parser.parse_args()
    from core import autosave, _native
    from core.scene import Scene
    from formats.igz import load_into
    import views.wgpu_scene as module
    autosave.pending = lambda *a, **k: None
    if args.reference_source:
        exec(compile(args.reference_source.read_text(), str(args.reference_source), 'exec'), module.__dict__)
    stats = {}
    peak = lambda: resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if sys.platform == 'darwin' else 1024)
    def wrap(obj, name):
        old = getattr(obj, name)
        def timed(*a, **k):
            start = time.perf_counter()
            result = old(*a, **k)
            entry = stats.setdefault(name, dict(seconds=0, calls=0, peak_bytes=0))
            entry['seconds'] += time.perf_counter() - start
            entry['calls'] += 1
            entry['peak_bytes'] = max(entry['peak_bytes'], peak())
            return result
        setattr(obj, name, timed)
    for name in ('_source_arrays', '_material_columns', '_texture_array', '_profile_endpoints'):
        if hasattr(module, name):
            wrap(module, name)
    wrap(np, 'unique')
    wrap(np, 'concatenate')
    wrap(_native.available(), 'bake_scene')
    if hasattr(_native.native, 'pack_weld'):
        wrap(_native.native, 'pack_weld')
    scene = Scene()
    start = time.perf_counter()
    load_into(scene, args.file)
    load = time.perf_counter() - start
    loaded = peak()
    start = time.perf_counter()
    packed = module.pack_scene(scene)
    elapsed = time.perf_counter() - start
    peak_pack = peak()
    fields = {k: dict(bytes=v.nbytes, shape=v.shape, dtype=str(v.dtype), sha256=digest(v))
              for k, v in vars(packed).items() if isinstance(v, np.ndarray)}
    profiles = hashlib.sha256()
    profile_bytes = 0
    for code, values in packed.profile_exclusions:
        profiles.update(int(code).to_bytes(8, 'little'))
        profiles.update(memoryview(np.asarray(values, np.float32)).cast('B'))
        profile_bytes += sys.getsizeof(values)
        if isinstance(values, list):
            profile_bytes += sum(sys.getsizeof(v) for v in values)
    textures = packed.textures
    result = dict(load=load, pack=elapsed, stages=stats, timings=packed.timings,
                  load_average=os.getloadavg(), peak_load_bytes=loaded,
                  peak_pack_bytes=peak_pack, arrays=fields,
                  profile_storage_bytes=profile_bytes, profile_sha256=profiles.hexdigest(),
                  texture_bytes=sum(map(len, textures[3])),
                  texture_sha256=[hashlib.sha256(v).hexdigest() for v in textures[3]],
                  texture_dimensions=textures[:3], texture_paths=packed.texture_paths,
                  uids_sha256=hashlib.sha256(repr(packed.uids).encode()).hexdigest(),
                  prototype_paints_sha256=hashlib.sha256(repr(packed.prototype_paints).encode()).hexdigest(),
                  core_module=_native.available().__file__)
    args.out.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
