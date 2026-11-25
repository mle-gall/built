"""Publishing gate for Python imports, embedded scripts and PyO3 imports."""
import ast
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
APP = (
    "analysis", "app_shell", "core", "examples", "formats", "georef", "materials",
    "packaging", "plugins", "resources", "scripts", "styles", "tools", "views",
)
QT = {"PySide6", "shiboken6", "QtCore", "QtGui", "QtWidgets"}
DYNAMIC = {"import_module", "__import__", "importorskip"}
SCRIPT_IMPORT = re.compile(r'^\s*(?:from|import)\s+(PySide6|shiboken6|QtCore|QtGui|QtWidgets)\b')
RUST_IMPORT = re.compile(
    r'(?:\.\s*import(?:_bound)?\s*\(\s*'
    r'|\b(?:PyModule::import(?:_bound)?|imported|import_names)\s*\(\s*[^,;]+,\s*)'
    r'"(?:PySide6|shiboken6|QtCore|QtGui|QtWidgets)\b'
)


def python_lines(source):
    found = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            names = [name.name for name in node.names]
        elif isinstance(node, ast.ImportFrom):
            names = [node.module or ""]
        elif isinstance(node, ast.Call) and node.args:
            name = getattr(node.func, "id", None) or getattr(node.func, "attr", None)
            value = node.args[0]
            names = [value.value] if name in DYNAMIC and isinstance(value, ast.Constant) and isinstance(value.value, str) else []
        else:
            continue
        if any(name.partition(".")[0] in QT for name in names):
            found.add(node.lineno)
    return found


def test_app_sources_have_no_qt_imports():
    remaining = []
    paths = [*ROOT.glob("*.py"), *ROOT.glob("*.spec")]
    for folder in APP:
        for pattern in ("*.py", "*.sh"):
            paths.extend((ROOT / folder).rglob(pattern))
    for path in sorted(paths):
        source = path.read_text(encoding="utf-8")
        if not any(name in source for name in QT):
            continue
        lines = python_lines(source) if path.suffix != ".sh" else set()
        lines.update(i for i, line in enumerate(source.splitlines(), 1) if SCRIPT_IMPORT.search(line))
        remaining.extend(f"{path.relative_to(ROOT)}:{i}" for i in sorted(lines))
    for path in sorted((ROOT / "rust").glob("*/src/**/*.rs")):
        source = path.read_text(encoding="utf-8")
        for match in RUST_IMPORT.finditer(source):
            remaining.append(f"{path.relative_to(ROOT)}:{1 + source.count(chr(10), 0, match.start())}")
    assert not remaining, "\n" + "\n".join(remaining)
