# Linux desktop parity

Run `scripts/linux_smoke.sh` from a checkout with Docker running. It builds the tracked working tree and writes each session's frame, document state and Vulkan adapter report under `build/linux-smoke/{x11,wayland}/`. Select one session with `scripts/linux_smoke.sh wayland`. An optional second argument selects the output directory. Xvfb and Weston run inside the container. The Wayland session has no X display and Weston starts without Xwayland.

## Platform conditionals

The native shell has 45 exact `#[cfg(target_os = "macos")]` sites after the launcher port, including the hidden PDF/probe window. The following groups account for them all.

| Source | Sites | Linux behavior and limits |
| --- | ---: | --- |
| main.rs | 19 | AppKit menu modules, creation, event dispatch, recent-file and extension-menu updates are Mac-specific. Linux exposes the command registry, extension actions and recents in the application. The desktop entry provides Open With and New Window. File operands are forwarded to a running instance through the Unix socket. Winit also handles dropped files. Linux has no Finder process serial number, Dock delegate, global AppKit menu or macOS window-tab group. Normal windows use compositor decorations. Test windows request hidden/inactive presentation where supported. |
| menu.rs | 1 | AppKit's application menu and system Services use the macOS menu bar. The Linux shell has no desktop-global File/Edit menu or Services integration. Attaching a GTK menu bar directly would require a GTK-owned window. |
| menu_commands.rs registration in native.rs | 1 | Command search on Linux reads the application registry and extension commands. Commands, shortcuts and enablement supplied only by the live AppKit menu have no corresponding Linux menu items. |
| python.rs | 2 | The menu-state and extension-update queries synchronize the Mac menu bar. The application maintains its model and extension actions independently on Linux. |
| native.rs, modal input | 4 | AppKit modifier conversion, nested event pumping and posted keys have Linux paths for blocking questions and scripted answers. GTK presents the question on its own thread so winit's Wayland or X11 connection has one event-loop owner. Synchronous progress painting still lacks general nested pointer dispatch on Linux. |
| native.rs, clipboard | 4 | GTK/GDK implements clipboard read, write and type discovery on both display servers, including application model data and plain text. The private test clipboard is process-local. Cross-application exchange and retention after exit require a desktop-session test. |
| native.rs, window chrome | 3 | AppKit background color, click-through test-window ordering and system window tabs are macOS-specific. Linux uses winit window attributes, its desktop app ID or WM_CLASS, and the application's document tabs. Wayland compositors control placement and stacking. The shell's GPU-drawn chrome remains available. The older Qt/AppKit glass panels live outside these Rust gates. There is no compositor blur implementation. |
| native.rs, application hooks | 2 | Linux dispatches launch arguments, winit close events and desktop file associations. Double-click registration includes native Built documents and projects. macOS Dock quit and Finder URL callbacks have no Linux delegate. |
| native.rs, scrolling | 2 | Linux accepts wheel and raw touch events from winit. Raw touchscreen contacts feed the shared touch handler. Cocoa momentum, device-source classification and the inverted-scroll flag are unavailable. The pinned Linux winit backends do not emit the Mac trackpad pinch, rotation, pan or smart-zoom event variants. Physical input devices were outside the compositor smoke. |
| platform_paths.rs | 4 | The Linux implementations resolve preferred languages from the environment, documents through xdg-user-dir, and application data and caches through XDG paths with home-directory fallbacks. |
| keys.rs | 1 | Linux uses Control for shortcuts and Super for Meta. macOS swaps the platform modifiers. |
| launcher.rs | 2 | Native window handles select Wayland, Xlib or XCB for GPU surfaces. Batch exports and probes use an inactive hidden window. Linux has no AppKit activation policy. |

A compositor smoke exercises file loading, the input script, surface creation and frame capture using Mesa software Vulkan. It does not measure physical GPU performance or exercise a desktop shell's launcher, menu, clipboard manager or touch hardware.
