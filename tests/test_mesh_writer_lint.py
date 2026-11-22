# SPDX-License-Identifier: GPL-3.0-or-later
"""Mesh topology writes belong in the kernel so views stay store-backed."""
import ast
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
KERNEL = {"core/mesh.py", "core/store_mesh.py"}
MUTATORS = {"append", "extend", "insert", "remove", "pop", "clear", "reverse",
            "sort", "__setitem__", "__delitem__", "__iadd__", "__imul__"}
# Non-mesh values with the same spelling, identified by the exact statement.
EXCEPTIONS = {
    ("core/scene_animation.py", "self.loop = bool(loop)"),
    ("formats/dxf_in.py", "per_layer.setdefault(layer, _Bucket()).faces.append(loop)"),
}


def field(expr):
    while isinstance(expr, ast.Subscript):
        expr = expr.value
    return expr.attr if isinstance(expr, ast.Attribute) else None


def writes_topology(node):
    if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
        attr = field(node.func.value)
        return attr in {"loop", "hole_loops", "faces"} and node.func.attr in MUTATORS
    if isinstance(node, (ast.Attribute, ast.Subscript)) and isinstance(
            node.ctx, (ast.Store, ast.Del)):
        return field(node) in {"loop", "hole_loops"} or (
            isinstance(node, ast.Subscript) and field(node) == "faces")
    return False


def test_no_direct_topology_writes_outside_kernel():
    violations = []
    paths = [ROOT / "main.py"]
    for folder in ("analysis", "core", "formats", "georef", "materials",
                   "plugins", "resources", "styles", "tools", "views"):
        paths.extend((ROOT / folder).rglob("*.py"))
    for path in paths:
        relative = path.relative_to(ROOT).as_posix()
        if relative in KERNEL:
            continue
        source = path.read_text(encoding="utf-8")
        lines = source.splitlines()
        for node in ast.walk(ast.parse(source, filename=relative)):
            if writes_topology(node):
                line = lines[node.lineno - 1].strip()
                if (relative, line) not in EXCEPTIONS:
                    violations.append(f"{relative}:{node.lineno}: {line}")
    assert not violations, "\n".join(violations)
