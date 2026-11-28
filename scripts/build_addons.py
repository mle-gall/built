"""Build reproducible format-2 archives from the add-on source manifests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


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
                    if listed[name] != "sha256:" + hashlib.sha256(data).hexdigest():
                        raise ValueError(f"{folder.name}/{name} does not match its hash")
                entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = 0o100644 << 16
                archive.writestr(entry, data)
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=ROOT / "dist" / "addons")
    args = parser.parse_args()
    for manifest in sorted((ROOT / "addons").glob("*/pack.json")):
        print(build(manifest.parent, args.dist))
