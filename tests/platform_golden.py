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
        if len(actual) != len(expected):
            return False
        if expected and all(type(x) is float for x in expected + actual):
            # A near-zero component inherits cancellation from the vector
            # that produced it. Bound that absolute error by eight ulps at
            # the vector's magnitude in addition to the scalar relative bound.
            magnitude = max(map(abs, expected))
            return all(math.isclose(a, b, rel_tol=1e-12, abs_tol=8 * math.ulp(magnitude))
                       for a, b in zip(actual, expected))
        return all(matches(a, b) for a, b in zip(actual, expected))
    if isinstance(actual, dict) and isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(matches(actual[k], expected[k]) for k in expected)
    return False
