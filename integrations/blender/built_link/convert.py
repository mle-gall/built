# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender object data ↔ Built values, with plain lists (no mathutils).

Built and Blender share the axes (Z up) and the unit (metre); matrices
travel column-major. A Blender light or camera looks down its local -Z
with +Y up.
"""
import math


def flat(rows):
    """Row-major 4×4 (Blender's ``Matrix``) → 16 floats column-major."""
    return [float(rows[r][c]) for c in range(4) for r in range(4)]


def rows(values):
    """16 floats column-major → row-major 4×4 lists."""
    return [[float(values[4 * c + r]) for c in range(4)] for r in range(4)]


def _sub(a, b):
    return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]


def _cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def _unit(v, fallback=(0.0, 0.0, -1.0)):
    n = math.sqrt(sum(x * x for x in v))
    return [x / n for x in v] if n > 1e-12 else list(fallback)


def column(m, c):
    """Column ``c`` (xyz) of a row-major 4×4."""
    return [float(m[0][c]), float(m[1][c]), float(m[2][c])]


def look_rows(eye, forward, up=(0.0, 0.0, 1.0)):
    """A row-major 4×4 at ``eye`` whose -Z is ``forward`` and +Y is as near ``up`` as it can be."""
    z = _unit([-forward[0], -forward[1], -forward[2]])
    x = _cross(up, z)
    if math.sqrt(sum(a * a for a in x)) < 1e-9:
        x = _cross((0.0, 1.0, 0.0), z)
    x = _unit(x, (1.0, 0.0, 0.0))
    y = _cross(z, x)
    return [[x[0], y[0], z[0], eye[0]], [x[1], y[1], z[1], eye[1]], [x[2], y[2], z[2], eye[2]], [0.0, 0.0, 0.0, 1.0]]


def light_value(kind, m, color, energy, spot_size, name, on=True):
    """A Blender light as Built's: point and spot only."""
    pos = column(m, 3)
    direction = _unit([-x for x in column(m, 2)])
    value = {"kind": "spot" if kind == "SPOT" else "point", "pos": pos, "dir": direction,
             "color": [float(c) for c in color[:3]], "power": float(energy),
             "angle": math.degrees(spot_size) if kind == "SPOT" else 60.0, "name": name, "on": bool(on)}
    if kind not in ("POINT", "SPOT"):
        value["kind"] = kind.lower()
        value["unsupported"] = f"{kind.title()} lights stay in Blender"
    return value


def vertical_fov(sensor_fit, angle_x, angle_y, width, height):
    """Built's field of view (vertical, degrees) for a Blender camera."""
    aspect = width / max(1.0, height)
    if sensor_fit == "VERTICAL" or (sensor_fit == "AUTO" and aspect < 1.0):
        fov = angle_y if sensor_fit == "VERTICAL" else angle_x
        return math.degrees(fov)
    return math.degrees(2.0 * math.atan(math.tan(angle_x / 2.0) / aspect))


def camera_value(m, distance, fov_deg, perspective):
    eye = column(m, 3)
    forward = _unit([-x for x in column(m, 2)])
    d = distance if distance and distance > 1e-6 else 10.0
    return {"eye": eye, "target": [eye[i] + forward[i] * d for i in range(3)],
            "up": _unit(column(m, 1), (0.0, 0.0, 1.0)), "fov_deg": float(fov_deg), "perspective": bool(perspective)}


def camera_pose(value):
    """Row-major 4×4 and distance for a Built camera value."""
    eye, target = value["eye"], value["target"]
    forward = _sub(target, eye)
    distance = math.sqrt(sum(x * x for x in forward))
    return look_rows(eye, forward, value.get("up") or (0.0, 0.0, 1.0)), distance


def light_pose(value):
    return look_rows(value["pos"], value.get("dir") or (0.0, 0.0, -1.0))
