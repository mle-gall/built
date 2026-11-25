"""The reference values were recorded with macOS libm and Accelerate."""
import math
import sys


def matches(actual, expected):
    if sys.platform == "darwin" or actual == expected:
        return actual == expected
    if isinstance(expected, float) and isinstance(actual, float):
        return math.isclose(actual, expected, rel_tol=1e-12)
    if isinstance(expected, str) and isinstance(actual, str):
        if expected.startswith(("0x", "-0x")) and "p" in expected and "p" in actual:
            return math.isclose(float.fromhex(actual), float.fromhex(expected), rel_tol=1e-12)
    if isinstance(actual, list) and isinstance(expected, list):
        return len(actual) == len(expected) and all(matches(a, b) for a, b in zip(actual, expected))
    if isinstance(actual, dict) and isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(matches(actual[k], expected[k]) for k in expected)
    return False
