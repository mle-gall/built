<h1 align="center">Built</h1>

<p align="center"><strong>Push/pull modelling at native speed, with BIM built in.</strong></p>

<p align="center">
  <img src="docs/images/built/overview.png" alt="A large model in the Built window: tools on the left, the project tree on the right" width="100%">
</p>

Built is a native 3D modeller: draw faces, push and pull them into shape with inference guiding every click, and file each element for BIM and IFC as you go. It stays fast on models of millions of faces.

## Highlights

### Draw, push, pull

<img src="docs/images/built/line-tool.png" alt="The Line tool snapping to an endpoint inside a large model, with the length shown beside the cursor" width="100%">

Draw a face and pull it into a solid. The cursor snaps to endpoints, edges and faces, and the arrow keys lock a line to an axis.
Offset, follow-me, fillet and solid operations work the same direct way, with the length typed or read as you go.

### Fast on very large models

The renderer runs on the GPU (wgpu, Metal on macOS) and culls there, so a model of several million faces stays interactive while you orbit, zoom and draw.
Edits stay local: changing one part does not rebuild the rest of the model.

### BIM built in

<img src="docs/images/built/tree-and-record.png" alt="An element selected in the model, with the project tree and its record beside it" width="100%">

One project tree files every element. A node carries the IFC class and type, tag, classification and default properties, and each element's record travels in the file.

<img src="docs/images/built/export-checks.png" alt="The Export columns of the project tree: a readiness count per node and a list of what to fix" width="100%">

Before export, the same tree shows node by node how many items are ready for IFC and what is missing.

More in [`docs/features.md`](docs/features.md).

## Status

Pre-release, with no public download yet. Built runs on macOS on Apple silicon; Linux and Windows builds are in progress.

## Formats

Saves `.built` ([format](docs/built-format.md)), opens `.igz`, imports `.skp` files, and exports [IFC](docs/ifc-export.md) (IFC4 and IFC4X3).

## Build from source

<details>
<summary>Commands (Python 3.13 and a Rust toolchain)</summary>

```bash
python3.13 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
VENV=.venv scripts/build_native.sh                                  # the Rust kernel and renderer, as Python extensions
cargo build --release --manifest-path rust/built_app/Cargo.toml     # the application
rust/built_app/target/release/built --root . --venv .venv           # run it from the checkout
```

For the macOS application bundle, with PyInstaller and ifcopenshell installed in the same environment:

```bash
packaging/build-macos-native.sh dist    # writes dist/Built.app
```

IDS checks need `ifcopenshell`, which `requirements.txt` does not install.

</details>

## Tests

See [`docs/testing.md`](docs/testing.md) for the test selections and what each one needs.

## Extensions

Built will have its own extension repository and website; neither is open yet. IngeTrazo extensions are not supported.

## License and credits

Built is free software under the GNU General Public License, version 3 or later ([`LICENSE`](LICENSE)). It began as a fork of IngeTrazo and contains its code and history: credit for the upstream project goes to Marco Sumari Tellez and IngeTrazo contributors. All authors are listed in [`AUTHORS`](AUTHORS) and keep their copyright.
