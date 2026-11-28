"""Build reproducible format-2 archives from the add-on source folders.

Each folder under addons/ is one pack (see addons/README.md):

    python3 scripts/build_addons.py              # every pack into dist/addons/
    python3 scripts/build_addons.py --dist DIR steel-profiles-en
    python3 scripts/build_addons.py --rehash     # after editing or adding rows

A build refuses a folder whose files do not match the hashes in its
pack.json. --rehash (needs the built_core module) lists any new rows/ file,
recomputes every row's own hash and the manifest's file hashes, and
rewrites only the files whose content changed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ADDONS = ROOT / "addons"


def _sha256(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _text(value):
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def _write_if_changed(path, text):
    if path.read_text(encoding="utf-8") != text:
        path.write_text(text, encoding="utf-8")
        return True
    return False


def rehash(folder):
    """Bring a pack folder's hashes up to date; returns the paths rewritten."""
    from built_core import command_protocol

    manifest_path = folder / "pack.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    listed = manifest["rows"]
    listed.extend(sorted(p.relative_to(folder).as_posix() for p in (folder / "rows").glob("*.builtpreset")
                         if p.relative_to(folder).as_posix() not in listed))
    files = manifest["pack"].setdefault("files", {})
    changed = []
    for name in listed:
        path = folder / name
        row = json.loads(path.read_text(encoding="utf-8"))
        row["hash"] = command_protocol.pack_row_hash(row)
        if _write_if_changed(path, _text(row)):
            changed.append(path)
        files[name] = _sha256(path.read_bytes())
    for name in [n for n in files if not (folder / n).is_file()]:
        raise ValueError(f"{folder.name}/{name} is listed in pack.json but missing")
    if _write_if_changed(manifest_path, _text(manifest)):
        changed.append(manifest_path)
    return changed


def build(folder, dist):
    manifest_bytes = (folder / "pack.json").read_bytes()
    manifest = json.loads(manifest_bytes)
    listed = manifest["pack"]["files"]
    if manifest["v"] != 2 or not set(manifest["rows"]).issubset(listed):
        raise ValueError(f"Invalid manifest in {folder.name}")
    dist.mkdir(parents=True, exist_ok=True)
    output = dist / (folder.name + ".builtpack")
    with tempfile.NamedTemporaryFile(dir=dist, delete=False) as tmp:
        temporary = Path(tmp.name)
    try:
        with zipfile.ZipFile(temporary, "w") as archive:
            for name in sorted([*listed, "pack.json"]):
                if name == "pack.json":
                    data = manifest_bytes
                else:
                    source = (folder / name).resolve()
                    if not source.is_relative_to(folder.resolve()):
                        raise ValueError(f"Invalid pack path {name}")
                    data = source.read_bytes()
                    if listed[name] != _sha256(data):
                        raise ValueError(f"{folder.name}/{name} does not match its hash "
                                         "(run scripts/build_addons.py --rehash)")
                # Fixed time stamp and mode: the same sources give the same bytes.
                entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = 0o100644 << 16
                archive.writestr(entry, data)
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
    return output


def folders(names=()):
    found = sorted(m.parent for m in ADDONS.glob("*/pack.json"))
    if names:
        unknown = set(names) - {f.name for f in found}
        if unknown:
            raise SystemExit(f"Unknown add-on: {', '.join(sorted(unknown))}")
        found = [f for f in found if f.name in names]
    return found


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("packs", nargs="*", help="pack folder names (default: all)")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist" / "addons")
    parser.add_argument("--rehash", action="store_true", help="update hashes in the source folders instead of building")
    args = parser.parse_args()
    for folder in folders(args.packs):
        if args.rehash:
            for path in rehash(folder):
                print("updated", path.relative_to(ROOT))
        else:
            print(build(folder, args.dist))
