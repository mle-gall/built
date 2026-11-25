# Changelog

All notable changes to Built are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and versions will follow
[SemVer](https://semver.org).

## Unreleased: Built

Built is a fork of IngeTrazo, the GPL-3 push/pull modeller. This first
section describes what the fork is now, compared with the upstream project
it grew from.

- **A native Rust application.**
  - The app (`rust/built_app`) owns the window, the menus and the event loop
    on macOS, Linux and Windows. There is no Qt.
  - The viewport is drawn with wgpu (Metal, Vulkan or DX12), with GPU culling
    and indirect draws.
  - The interface is drawn with egui in the same frame, inside the 3D view:
    - a top bar;
    - a tool rail with every tool visible;
    - a project dock with the tree and an inspector;
    - a bottom bar for units, tool options and typed values.
- **A Rust core.**
  - The document, the geometry kernel, the modelling tools, the undoable
    commands, snapping and inference, the sheets, and the file readers and
    writers are Rust (`rust/built_core`).
  - Every mesh lives in a store with true f64 coordinates, so models at
    survey coordinates keep their precision.
  - Python remains the scripting, plugin and MCP layer, and keeps IFC and the
    object records.
- **Native formats.**
  - `.built` is the document format: a ZIP archive described in
    `docs/built-format.md`.
  - `.builtproject` is a project manifest at the project root that ties
    several models together (`docs/builtproject-format.md`).
- **`.igz` read.** Documents from the upstream project open unchanged.
- **`.skp` import**, including nested components, tags and PBR materials.
- **IFC.**
  - Every element is filed once in the project tree, and the tree gives its
    items their IFC class, predefined type and records.
  - Export writes IFC that passes its checks.
- **Projects and linked models.**
  - A host document links other documents as live, read-only placements.
  - The project tree files every element once, and derives tags,
    classifications and records from it.
- **The new CI.**
  - Every push and pull request runs the fast test suite on Linux and checks
    that the Linux launcher builds.
  - The release workflows package macOS, Linux (AppImage, Flatpak) and
    Windows builds.
- **No IngeTrazo extensions.** Built has its own plugin API
  (`docs/plugins.md`). Extensions written for the upstream project are not
  loaded.

For the history before the fork, see the
[upstream changelog](https://github.com/ingelibre/ingetrazo/blob/main/CHANGELOG.md).
