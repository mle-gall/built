# Built design system

Built keeps the model in view. Dense controls sit on graphite surfaces, with Inter labels aligned across the dock, sheets and bars. Use blue only as an accent for selection, the primary action, focus and links. Give Start, Library and Settings the same neutral surfaces as the dock. Avoid navy backgrounds, blue surface tints and page-wide washes.

## Tokens

The public Rust module `ui::tokens` owns colour values. Its palette declaration also registers every role in the gallery. Colours with alpha composite over the surface below them.

| Role | Tokens and values |
|---|---|
| Surface | `SHEET` #1A1A1A, `SIDE` / `GLASS_BAR` #1F1F1F, `CARD` #242424, `CARD_HOVER` #2B2B2B, `WELL` #131313, `DESK` (= `WELL`, the Sheets desk around the paper; Python reads it through `built_render.chrome_token`) |
| Depth | `PANEL` / `POPOVER` premultiplied grey 31 at alpha 245, `FIELD` black at 28%, `SCRIM` black at 28%. `Surface::{Bar,Panel,Pill,Raised}` supply glass, radius and shadow together. |
| Text | `INK` #EEF1F4, `INK_2` #C9CFD6, `DIM` #A2A9B1, `FAINT` #858C94, `OFF` #5C636A, `ON_PRIMARY` graphite on the primary accent, `BRIGHT` white on dark surfaces |
| Boundary | `EDGE` / `DIVIDER` white at 8%, `HOVER_EDGE` white at 18%, `FOCUS_RING` #6AA5FF |
| Controls | `CONTROL` white at 7% (rest fill of buttons and chips), `TRACK` white at 11% (progress, switch), `THUMB` white at 16% (segmented thumb) |
| Interaction | `HOVER` white at 7%, `PRESS` white at 12%, `ACTIVE` accent at 14% (selection wash), `LIT` accent at 28% (an armed tool or a bar toggle that is on, under an `ACCENT_LINE` edge), `ACCENT_LINE` accent at 55%, `ACCENT` #6AA5FF, `PRIMARY_HOVER`, `PRIMARY_PRESS` |
| Feedback | `OK` green, `WARN` amber, `ERROR` red, `INFO` secondary ink, `WARN_FILL` and its hover and press. Pair status colours with a label, icon or tooltip. |
| Link dots | `FOUND` green, `CHANGED` amber, `MISSING` red, `PACKED` dim |
| Agent | `WORKING` secondary ink, `WAITING` dim, `QUESTION` amber, `DONE` green, `FAILED` red |
| Type | Four sizes (`TYPE_*`): meta 11 for labels, captions, counts and shortcuts; body 12 for values and controls; title 13 semibold for a panel's subject; heading 16 semibold for a sheet. Medium weight (`strong`) marks the current item and button labels. `caption_font()` is 11 medium. |
| Heights | `CONTROL_H` 24 for every inline control (fields, chips, segmented, tabs, switches, quiet and icon buttons). `BUTTON_H` 28 for sheet buttons. `ROW_H` 24 for tree, list and menu rows. `PROP_H` 20 for label-and-value rows. |
| Spacing | `SPACE_XS` 4 within a group, `SPACE_S` 8 between items, `SPACE_M` 12 between groups and for panel padding, `SPACE_L` 16 between sections, `SPACE_XL` 24 between sheet regions. `SPACE` lists the same steps. |
| Radii | Radii follow height. `R_ROW` 6 for rows and 24-pt controls, `R_CONTROL` 8 for 28-pt buttons, tool buttons and tiles, `R_INSET` 4 for a thumb or swatch inside a control. Glass: panel 14, raised and pill 12, docked bar 0. |
| Focus | `FOCUS_RING_W` 1 pt, drawn `FOCUS_GAP` 1 pt outside the control. Focus never fills. |
| Motion | Quick feedback 120 ms, fade 200 ms. Preserve a stable hit target throughout a transition. |

### Glass levels

From lowest to highest, each level is lighter and casts more:

| Level | Use | Grey | Opacity | Shadow |
|---|---|---|---|---|
| `Bar` | Docked top and bottom bars | 31 | 0.90, no rim | none |
| `Panel` | Dock, tree, inspector | 33 | 0.80 | 6 / 20, alpha 90 |
| `Pill` | Palette, activity capsule | 36 | 0.70 | 4 / 14, alpha 100 |
| `Raised` | Menus, popovers, toasts, question sheets | 44 | 0.90 | 12 / 32, alpha 150 |

## Component kit

| Layer | Components | When to use |
|---|---|---|
| Style | `install_style` | Call it once per egui context. It gives egui's own fields, numbers and choices the kit's state looks. |
| Atoms | `label`, `dot`, eye and lock icons, `chevron`, `centered` / `leading` text placement, `paint_focus`, `paint_selected_row` | Align text, identify status and expose per-row visibility or locking. Place every control label through `centered` or `leading` so baselines match. |
| Molecules: buttons | Primary, secondary and destructive `sheet_button`, `quiet`, `icon_button`, `chip_button` and `amber_button` | Use one primary commit action in a question. Give destructive commands the error ink. Use quiet buttons for subordinate actions and label icon buttons for discovery. |
| Molecules: switches | `segmented`, `underline_tabs`, `choice`, `dropdown`, `toggle` | Use `segmented` to switch a mode or place: an inset track with a raised neutral thumb. Use `underline_tabs` to navigate the sections of one panel: text with an accent underline and no box. Use `choice` to pick one of many values and `toggle` (a switch) for a Boolean. Use `dropdown` (a caption, the current value and a chevron) for a control whose menu holds more than values, such as the tree's Columns control or the delivery picker; open its menu at `popover_pos`, which keeps it inside the window. In a form (settings, operation forms, prompts) a Boolean is a `switch_row`: the same switch at the row's leading edge, then its label; the off value shows the empty track, never bare text. |
| Molecules: fields | `field`, `search`, `filter_button`, `number` with unit, `colour_well` for opaque RGB, `path_field`, `swatch` | Edit a name, a measurement, a local filter or a document colour. A filter field ends with `filter_button`: it opens the quick choices and the sort, and lights while one of them is on. Preserve identity across frames and keep IME text in the focused editor. |
| Molecules: formatted prose | `kit::prose` | Render formatted release-note paragraphs with heading, emphasis, code and actionable link spans. |
| Bevel cross-section | `CurveEditor` | Show the sampled outline and its control points. Double-click adds a point or changes its kind. Use the numeric fields for precise placement. The inspector and gallery use the same editor. |
| Drawing previews | `tag_swatch`, `hatch_swatch`, `line_type_preview`, `piece_sample` | Show a tag’s model colour or the uncoloured default’s diagonal slash, selected hatch colours, signed dash rhythm and text, or scaled masonry dimensions in the Drawing inspector and gallery. |
| Rename preview | `rename_preview` | Show the old and proposed names side by side. Changed names use stronger text. Long lists scroll within the form. |
| Saved-view row | `scene_preview_row` | Reserve a thumbnail beside the saved-view name and indicate the active view while its image loads. |
| Render preview | `render_preview` | Fit the image or inspect at actual pixel size. Scroll to zoom around the pointer and drag to pan. The plugin utility and gallery share the control. |
| SVG page preview | `svg_preview` | Fit the full page in a neutral well. Drag to pan and scroll to zoom. Composition guides draw over the page in the preview. |
| Molecules: data | `list_row`, `tree_slots`, `row_glyph`, `paint_tile` / `paint_card`, `inspector_title`, `prop_row`, `property_control`, `table_row`, `caption`, `disclosure`, `status_chip` / `badge`, `empty_state`, `progress_bar` | Reuse the row hit area and reserve slots for counts and eye/lock controls. Give each kind of tree row its `row_glyph` (tag, tag folder, loose geometry, section planes, scenes, sheets, group, component, image, link). Start an inspector with `inspector_title`: the subject, and what it is at the trailing edge. Show label and value pairs, collapse a section such as Export details, report a state and explain an empty result in context. |
| Organisms | `Glass` (`glass`, `popover`, `activity_capsule`), `menu` with `menu_row` and `menu_separator`, `sheet_frame`, `tree`, `toast` / `toast_stack`, `value_pill` | Assemble a workspace, browse a hierarchy or show choices anchored to a control. Keep data and action dispatch in the owning surface. Show active work in the capsule, using a ring for unknown duration and `progress_bar` for measured progress. Stack at most three transient results. `value_pill` is the value field on the geometry: an accent edge and a drawn caret while typing, a check on the committed value it reads back until the next action (`ui/value_field.rs`). |

The dock has one width, the user's: dragged on its inner edge between `DOCK_MIN_W` and `DOCK_MAX_SHARE` of the window and kept per user. The project tree has one set of rows; Filing, Drawing and Export are presets of its columns (`ui/lens.rs`), chosen in the Columns control with each column's own toggle. A preset changes the trailing cells only: rows, selection, expansion, scroll, the dock's width and the inspector stay. Cells that do not fit beside the name compact to swatches; if they still do not fit, they scroll under the pinned name. The inspector depends on what is in focus alone: a tag shows its drawing style under every preset, and with nothing selected the document shows its counts, the Fix checklist, its tag folders and its line types.

The tool rail (`ui/rail.rs`) lays out from the band's height alone: two columns when every row fits between the bars, three when two do not, and three that scroll only as a last resort. A tool's place is its index in its group. The model and Sheets rails share it.

The About sheet uses `credits_roll` for contributor names, linked profiles and translated roles. The band pauses under the pointer and accepts wheel movement in either direction. The gallery includes a sample credit.

### States

| State | Look |
|---|---|
| Rest | Buttons and chips are a `CONTROL` wash with no outline. Fields are a `FIELD` well with no outline. Rows and quiet buttons have no fill. |
| Hover | Adds `HOVER`. Fields gain a `HOVER_EDGE` hairline. Text goes from `INK_2` or `DIM` to `INK`. |
| Pressed | Adds `PRESS`. The primary button uses `PRIMARY_PRESS`. |
| Selected | Adds the `ACTIVE` wash. An armed tool and a bar toggle that is on are lit instead (`paint_lit`: `LIT` under a 1-pt `ACCENT_LINE` edge, glyph `BRIGHT`). Rows add the 2-pt accent marker. The segmented control raises its thumb, the tabs underline the current tab, the switch fills its track and menus show a check. A control is never filled with full accent. |
| Focus | Shows the 1-pt `FOCUS_RING` outside the control and nothing else. Focus marks where keys go and stays visibly separate from selection. |
| Disabled | Uses `OFF` ink and drops the wash. A disabled primary button loses its accent. |

### Large sheets

Start, Library and Settings share the kit’s glass `sheet_frame`. It lays out the header, optional side list, content, optional detail pane and optional footer. Each sheet supplies the contents and handles its own commands. Header tabs use the kit’s underline treatment because they navigate sections of the sheet, such as Materials, Components and Profiles in Library. Start uses its header for file actions and Settings uses its side list for section navigation.

## Adding a component

A surface may introduce a custom atom, molecule or organism when it serves the interaction better. Keep it dense, dark and viewport-first.

1. Place the reusable implementation in the matching module in `ui/kit.rs`. Keep document-specific data and commands in the caller.
2. Use shared colour, type, height, spacing, radius, motion and glass tokens. Introduce a named role in `ui/tokens.rs` if existing roles cannot express the design. Stay within the four type sizes.
3. Respect input ownership. A press that begins on a control owns its drag and release. A viewport drag keeps ownership across chrome. A modal sheet owns keys and pointer events while open. Give fields stable focus IDs and forward text and IME input to their owner.
4. Read its state through `state_of` and paint through `paint_state`, `paint_focus` or `paint_selected_row`, so the gallery's forced states (`specimen`) apply to it.
5. Add a row to the molecule matrix in `ui/gallery.rs`, or place it in the surfaces band. `gallery::coverage` fails when a public kit function is missing from the gallery (non-components are listed there with a reason). with its applicable states: rest, hover, pressed, selected, focus and disabled. Add an organism to the organisms band. Use real-looking content, such as file names, IFC fields and units.
6. Test the interaction or layout through a real caller. Run the renderer library tests and inspect a fresh gallery capture.

The Font picker in the gallery's organisms band opens the same compact popover used by 3D Text, sheet text and the shared dimension and label style. The gallery example includes a recent family and a starred family. Typing filters the installed families, arrows move the selection, Enter applies and Escape cancels.

## Verification

Run `cargo test --manifest-path rust/built_render/Cargo.toml --lib` for these tests:
- the colour and literal font-size policies;
- the neutral-surface tests;
- the glass-elevation order;
- the four-size type scale.

The policy scans Rust sources under `src/ui*` and `src/inspector`. Only `src/ui/tokens.rs` may construct chrome colours. Renderer modules outside that tree and shaders are exempt. Material values and image pixels are data: hex input is decoded by the token module, and texture uploads retain their original bytes.

After building the native module and shell, run `built --gallery` or `.venv/bin/python main.py --gallery`. Run `scripts/design_system_gallery.sh` to write both host captures to `build/design-system/gallery-*.png`. Set `DS_OUT` to choose another folder and `DS_PREFIX` to prefix the file names. `scripts/design_system_capture.py before|after --dest <folder>` captures the real surfaces from synthetic records. The surfaces are the start page, dock and tree, tabs, inspector with Export details opened by a real click, Library, activity capsule, toast and question sheet. Both scripts use an isolated settings directory and check that the preferences file is untouched.
