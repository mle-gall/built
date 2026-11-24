"""Fuzz cases for the differential sweeps (docs/testing.md).

The default run takes a small sample of each sweep's cases; the whole sweep
runs under the ``slow`` marker. The sample is drawn from the run's fuzz seed:
``0`` unless ``BUILT_FUZZ_SEED`` says otherwise, ``random`` picks a new one
(conftest draws it once, before pytest-xdist starts its workers, so every
worker collects the same cases). The seed is printed in the report header;
running again with it collects the same cases.

    BUILT_FUZZ_SEED=random python -m pytest -n 8 -m "not slow" tests
"""
from __future__ import annotations

import os
import random

ENV = "BUILT_FUZZ_SEED"


def run_seed() -> str:
    return os.environ.get(ENV, "0")


def sample(cases, k: int, key: str) -> list:
    """``k`` of ``cases``, the same for a given run seed and ``key`` (the
    sweep's name, so two sweeps do not draw the same positions)."""
    cases = list(cases)
    if k >= len(cases):
        return cases
    return sorted(random.Random(f"{run_seed()}:{key}").sample(cases, k))
