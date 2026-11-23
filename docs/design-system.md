# Built design system

Built keeps the model in view. Dense controls sit on graphite surfaces, with Inter labels aligned across the dock, sheets and bars. Use blue only as an accent for selection, the primary action, focus and links. Give Start, Library and Settings the same neutral surfaces as the dock. Avoid navy backgrounds, blue surface tints and page-wide washes.

## Tokens

The public Rust module `ui::tokens` owns colour values. Its palette declaration also registers every role in the gallery. Colours with alpha composite over the surface below them.

| Role | Tokens and values |
|---|---|
| Surface | `SHEET` #1A1A1A, `SIDE` / `GLASS_BAR` #1F1F1F, `CARD` #242424, `CARD_HOVER` #2B2B2B, `WELL` #131313 |
| Depth | `PANEL` / `POPOVER` premultiplied grey 31 at alpha 245, `FIELD` black at 28%, `SCRIM` black at 28%. `Surface::{Bar,Pill,Panel,Raised}` supply glass, radius and shadow together. |
| Text | `INK` #EEF1F4, `INK_2` #C9CFD6, `DIM` #A2A9B1, `FAINT` #858C94, `OFF` #5C636A |
| Boundary | `EDGE` / `DIVIDER` white at 8%, `FOCUS_RING` #6AA5FF |
| Interaction | `HOVER` white at 7%, `PRESS` white at 12%, `ACTIVE` accent at 24%, `ACCENT_LINE` accent at 55%, `ACCENT` #6AA5FF. Keep focus visible independently of selection. |
| Feedback | `OK` green, `WARN` amber, `ERROR` red, `INFO` secondary ink. Pair status colours with a label, icon or tooltip. |
| Link dots | `FOUND` green, `CHANGED` amber, `MISSING` red, `PACKED` dim |
| Agent | `WORKING` secondary ink, `WAITING` dim, `QUESTION` amber, `DONE` green, `FAILED` red |
| Type | Inter regular 11 pt for metadata, 12 pt for body. Use medium for emphasis, semibold 13 pt for titles and 18 pt for sheet headings. |
| Geometry | Spaces 4 / 8 / 12 / 16 / 24 pt. Row radius 6 pt, control radius 8 pt. Panel / raised / pill / docked bar radii 14 / 12 / 12 / 0 pt. |
| Motion | Quick feedback 120 ms, fade 200 ms. Preserve a stable hit target throughout a transition. |

## Component kit

| Layer | Components | When to use |
|---|---|---|
| Atoms | Label, status dot, eye and lock icons | Align text, identify status and expose per-row visibility or locking. |
| Molecules | Primary, secondary, destructive, quiet and icon buttons, command and warning chips | Use one primary commit action in a question. Give destructive commands the error ink. Use quiet buttons for subordinate actions and label icon buttons for discovery. |
| Molecules | Segmented switch, underline tabs, choice menu and toggle | Switch workspace or mode, navigate related sections, choose one of many values, or change a Boolean value respectively. |
| Molecules | Text, number with unit, search and colour swatch | Edit a name, a measurement, a local filter or a document colour. Preserve identity across frames and keep IME text in the focused editor. |
| Molecules | List row, tree slots, tile/card, section caption, badge and empty state | Reuse the row hit area, reserve slots for counts and eye/lock controls, group items visually and explain an empty result in context. |
| Organisms | Sheet frame with header, side list, content and detail, scrolling tree, glass popover | Assemble a workspace, browse a hierarchy or show choices anchored to a control. Keep data and action dispatch in the owning surface. |
| Feedback | Activity capsule, progress ring/bar, toast stack | Show active work in the capsule, use a ring for unknown duration and a bar for measured progress. Stack at most three transient results. |

## Adding a component

A surface may introduce a custom atom, molecule or organism when it serves the interaction better. Keep it dense, dark and viewport-first.

1. Place the reusable implementation in the matching module in `ui/kit.rs`. Keep document-specific data and commands in the caller.
2. Use shared colour, type, spacing, radius, motion and glass tokens. Introduce a named role in `ui/tokens.rs` if existing roles cannot express the design.
3. Respect input ownership. A press that begins on a control owns its drag and release. A viewport drag keeps ownership across chrome. A modal sheet owns keys and pointer events while open. Give fields stable focus IDs and forward text and IME input to their owner.
4. Add the piece and its applicable idle, hover, pressed, selected, focused and disabled states to `ui/gallery.rs`. Show empty, loading or error content where relevant.
5. Test the interaction or layout through a real caller. Run the renderer library tests and inspect a fresh gallery capture.

## Verification

Run `cargo test --manifest-path rust/ingetrazo_render/Cargo.toml --lib` for the colour policy and neutral-surface tests. The policy scans Rust sources under `src/ui*` and `src/inspector`. Only `src/ui/tokens.rs` may construct chrome colours. Renderer modules outside that tree and shaders are exempt. Material values and image pixels are data: hex input is decoded by the token module, and texture uploads retain their original bytes.

After building the native module and shell, run `built --gallery` or `.venv/bin/python main.py --gallery`. Run `scripts/design_system_gallery.sh` to write both host captures to `build/design-system/gallery-*.png`. The capture uses an isolated settings directory and reads back the application framebuffer.

## Captures

The gallery fits the full token palette and component specimens into one framebuffer in both hosts. [Rust shell](../build/design-system/gallery-built.png) · [Qt app](../build/design-system/gallery-qt.png)

The comparison uses the same synthetic model metadata in each image. The baseline renderer was built from `4f9710f4`. The dock, tree and tab details are crops of the same application frame.

| Surface | Before | After |
|---|---|---|
| Start | [Before](../build/design-system/before-start.png) | [After](../build/design-system/after-start.png) |
| Library | [Before](../build/design-system/before-library.png) | [After](../build/design-system/after-library.png) |
| Dock | [Before](../build/design-system/before-dock.png) | [After](../build/design-system/after-dock.png) |
| Project tree | [Before](../build/design-system/before-tree.png) | [After](../build/design-system/after-tree.png) |
| Project tabs | [Before](../build/design-system/before-tabs.png) | [After](../build/design-system/after-tabs.png) |
| Question sheet | [Before](../build/design-system/before-sheet.png) | [After](../build/design-system/after-sheet.png) |
| Activity capsule | [Before](../build/design-system/before-activity.png) | [After](../build/design-system/after-activity.png) |
