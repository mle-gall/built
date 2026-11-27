# Menus

Every menu surface of the app, who owns its structure, and the rules
that keep it from growing lane by lane. The native menu bar's full tree,
with ids and key equivalents, is the fixture
`rust/built_app/tests/mac_menu_tree.txt`: the table below is that tree in
readable form. Change the menu in `rust/built_render/src/desktop_menu_model.rs`
(the order of every top-level menu) and `model_menu.rs` (the commands and
their submenus), then refresh the fixture.

## Rules

1. **One home per command in the native menu bar.** A command id appears
   once across all top-level menus. Two different commands never share a
   visible label inside one top-level menu (with its submenus).
   `desktop_menu_model::tests` and `rust/built_app/tests/mac_menu_tree.rs`
   fail on either; the allow-list there names the few deliberate
   exceptions.
2. **The command palette indexes every command once**, by id, with the
   menu path of its home. A command that is only a key (Toggle Hide Rest
   of Model ⌥Q, Toggle Perspective / Parallel ⇧P, X-ray Style) is
   palette-only: it has an id and a key but no menu row
   (`model_menu::PALETTE_ONLY`).
3. **Context menus may repeat** commands that act on their target (the
   viewport and tree menus, a tab's menu, the dock's "⋯", the top bar's
   Model ▾ and Scenes ▾). They use sentence case and may phrase the verb
   for the target ("Add scene from this view").
4. **Labels.** Menu-bar items and command names are Title Case (macOS);
   context menus and popover rows are sentence case. A trailing "…" means
   the command asks for more before it acts. File kinds are named the
   same everywhere: "Built model (.built)", "Built project
   (.builtproject)", ".igz (IngeTrazo)" for the legacy format, ".skp" for
   that format (never the other application's name).
5. **Model vs project.** A *model* is one .built document (a tab); a
   *project* is a .builtproject that links models. File, the tabs, the
   top bar's document menu and the start page use these two words only.
6. **Where each kind of command goes.**
   - **File**: New / Open / Recent / Start Page; Close; Save, Save As,
     Save a Copy, Recover; Import ▸ (formats only), Link, Reload Linked
     Files; Export ▸ — *Deliveries…* first (the multi-output handover:
     Export = Deliveries), then single-format quick exports (IFC, glTF,
     COLLADA, STL, OBJ), then the views (Current View as Image / SVG / DXF,
     Scenes as Video), then the legacy .igz copy. No export entries
     elsewhere in File.
   - **Edit**: history, clipboard, selection, grouping, visibility, and
     the operations on the selected geometry.
   - **View**: the camera (navigation, standard views, projection,
     walkthrough), display (style, X-ray, axes, edges, shadows, sections),
     scenes and animation, the model view layouts, full screen.
   - **Model**: settings and reports of the whole model (site, units, cut
     list).
   - **Draw** and **Tools**: the modelling tools you arm; Tools ends with
     diagnostics, the Python console and maintenance.
   - **Extensions**: the extension manager, folder, developer guide, then
     what extensions contribute.
   - **Window**: models (next, previous, move to a window), the sidebar
     destinations and their keys (Project, Tasks, Deliveries, Library),
     recent activity, the agent, Review (hide the panels).
   - **Help**: documentation, content, updates.
7. **Shortcuts.** A native item shows its key when the key uses ⌘. Keys
   with only ⌥ or no modifier stay out of the menu bar (they type text in
   a field); the palette shows them. ⌘H, ⌘Space, ⌘M, ⌘Q belong to the
   system.

## Audit of 2026-11-26

The menu bar had grown lane by lane: File had two export entry points
("Export…" and "Export ▸") and three more export rows beside them, and
nobody owned the order. This audit inventoried every surface, fixed the
menu bar and the palette, and added the tests of rule 1.

| Kind | Found | Fixed | Left (why) |
|---|---|---|---|
| Duplicate (same command twice, or two commands with one name in one menu) | 4 | 4 | 0 |
| Near-duplicate (one job split across places) | 9 | 7 | 2 (N8 kept by rule 6, N9 the tool rail) |
| Wrong or stale label | 9 | 9 | 0 |
| Entry in the wrong menu | 9 | 9 | 0 |
| Inconsistent wording | 7 | 5 | 2 (I6 core filter, I7 context menus) |
| Missing shortcut shown elsewhere | 13 | 5 | 8 (rule 7: sheet keys, system keys, ⌥ and bare keys) |
| Dead entry | 0 | — | — |
| **Total** | **51** | **39** | **12** |

### Duplicates
- D1 File: "Export…" (`delivery_export`) beside the "Export ▸" submenu,
  and "Export Image…", "Export Current View as SVG…", "Save a Copy as
  Built (.igz)…" outside it. → One Export ▸: Deliveries… first, then the
  formats, the views, the legacy copy.
- D2 "Export Video…" (`animation_video`, Camera ▸ Animation) and "Scenes as
  Video…" (`export:video`, File ▸ Export) ran the same handler. → Only
  File ▸ Export ▸ Scenes as Video….
- D3 Camera: "X-ray" (`xray`, the toggle) and Camera ▸ Style ▸ "X-ray"
  (`style_xray`, the style). → The style is palette-only, named "X-ray
  Style".
- D4 Camera ▸ Rest of model while editing ▸ "Hide (fastest)" and Camera ▸
  Component Edit ▸ "Hide Rest of Model" set the same state. → One
  submenu; the toggle (⌥Q) is palette-only "Toggle Hide Rest of Model".

### Near-duplicates
- N1 Standard views split: Iso/Top/Front/Right in Camera (`view:*` ids),
  Back/Left/Bottom in a submenu (`view_*` ids). → View ▸ Standard Views,
  all seven, one id each.
- N2 "Toggle Perspective / Parallel" beside Projection ▸ Perspective /
  Parallel. → The toggle (⇧P) is palette-only.
- N3 Drawing tools split: Line/Rectangle/Circle in Tools,
  Freehand/Path/Arcs/Shapes in Draw. → All in Draw.
- N4 "Section cuts" in Camera, Section Planes and Section Fill in Camera ▸
  Sections. → View ▸ Sections.
- N5 Scene stepping (Next/Previous Scene) in Animation, the scene
  commands in Scenes. → View ▸ Scenes.
- N6 Extensions: Window ▸ Extension Manager…, Plugins ▸ Open extensions
  folder / Develop an extension…. → One Extensions menu.
- N7 Display settings forms (Edit style…, Shadows and sun…) and Organise
  scenes were only in the palette and the top bar's View ▾. → Beside
  their siblings in View.
- N8 *Left by rule:* Deliveries: File ▸ Export ▸ Deliveries… and Window ▸ Deliveries both
  open the Deliveries destination. Kept by rule 6 (the handover entry and
  the destination), two ids.
- N9 *Left:* the geometry operations stay in Edit while Bridge and Make
  Faces stay in Tools: those two are buttons on the tool rail, and Tools
  lists the rail.

### Wrong or stale labels
- L1 "Save a Copy as Built (.igz)…", its dialog "Save a Copy as Built" and
  filter "Built (*.igz)": .igz is the legacy IngeTrazo format. → "Save a
  Copy as .igz (IngeTrazo)…", filter "IngeTrazo (*.igz)".
- L2 Import "Built model, as a group…" opens .built and .igz. → "Built
  Model as a Group (.built, .igz)…".
- L3 The "Plugins" menu, whose rows all say "extension". → "Extensions".
- L4 The palette described File ▸ Export ▸ COLLADA and OBJ as imports
  ("Bring in a … model"). → Export descriptions.
- L5 "Camera ▸ Hidden Objects / Geometry" in the hint of Unhide Selected
  with nothing hidden: no such item. → "View ▸ Hidden Geometry".
- L6 "Camera ▸ Animation ▸ Animation Settings…" in the video error and
  "Camera ▸ Animation" in the Scenes as Video description. → View.
- L7 "Window ▸ Extensions" (report sheets), "Extensions ▸ Manage
  extensions" (docs/plugins.md), "Plugins menu" (docs/context-menu.md),
  "Camera ▸ Shadows" (docs/north.md). → Current paths.
- L8 Review's description "Open the model for review." (it hides the
  panels). → Says what it does; label "Review (Hide Panels)".
- L9 "Iso" (menu) vs "Isometric" (registry, View ▾) for one command. →
  "Isometric".

### Entries in the wrong menu
- W1 The "Camera" menu held display (styles, X-ray, axes, edges, shadows,
  sections). → Renamed View (the macOS name, and the top bar's).
- W2 Recent activity in Camera. → Window.
- W3 Units… and Cut list… in Window. → Model.
- W4 Extension Manager… in Window. → Extensions.
- W5 Model view layouts and Maximize / Restore View in Window. → View ▸
  Model Views.
- W6 "Clear imported texture cache…" in File ▸ Import (not a format). →
  Tools, after Diagnostics and the console.
- W7 Soften / Smooth Edges… between the clipboard and Select All. → With
  the face operations.
- W8 Recover a discarded auto-save… at the bottom of File, after Export. →
  The Save group.
- W9 Diagnostics and Python Console above the modelling tools in Tools. →
  At the end.

### Inconsistent wording
- I1 "Export…" vs "Export ▸"; "Export Current View as SVG…" vs "Current
  view as DXF…" vs "Export Image…". → "Current View as Image / SVG / DXF…"
  under Export ▸.
- I2 Case: "New model", "Close model", "Next model", "Move model to new
  window", "Site placement…", "Recent activity", "Back edges", "Hidden
  geometry", "Section cuts", "Camera inspector…", "Repeat last command",
  "Terrain profile of selection", "Open extensions folder", "Recover a
  discarded auto-save…", "Get more models and textures…", "Rest of model
  while editing", "Ask an agent…" among Title Case rows. → Title Case
  (rule 4; a test checks it).
- I3 Format names differ between Import and Export ("glTF (.glb,
  .gltf)…" vs "glTF / GLB…", "STL mesh (.stl)…" vs "STL (3D printing)…",
  ".dxf…" vs "IFC (BIM)…"). → "Name (.ext)…" on both sides.
- I4 Model vs project: the start page offered "New project…", File did
  not. → File ▸ New Project….
- I5 Shared registry labels in sentence case (Edit style…, Shadows and
  sun…, Organise scenes). → Title Case.
- I6 *Left:* File ▸ Open… lists .built and .igz while the start page's
  Open also lists .builtproject (the filter lives in the core's
  `DOCUMENT_FILTER`; out of this lane).
- I7 *Left:* the tree's "Lock / unlock", "Show / hide" vs the viewport
  menu's "Lock" / "Unlock" and "Hide". Context menus, allowed to differ
  (rule 3), owned by the Project tree.

### Missing shortcuts
The palette and the keyboard settings knew these keys; the menu bar did
not show them.
- Shown now (⌘ keys): Top…Bottom ⌘1–⌘7, Zoom Selection ⌥⌘Z, Invert
  Selection ⇧⌘I, Unhide ▸ All ⇧⌘H, Review ⌘0.
- *Left out on purpose* (rule 7): Make Group ⌘G and Explode Group ⇧⌘G
  (a sheet's Group / Ungroup keys: a menu key equivalent would run the
  model command over a sheet), Hide ⌘H (Hide Built), Maximize / Restore
  View ⌘Space (Spotlight), X-ray ⌥X, Axes ⌥A, Hide Similar Components ⌥W
  (⌥ keys type characters in a field), Previous / Next Scene PgUp / PgDn
  (bare keys). The palette shows them.

### Dead entries
None: every id of the menu bar is run by `shell_files::command`,
`shell_host` (`tool:`, `panel:`, `view:`) or `model_menu::run_model_menu`.

## Surfaces

| Surface | Code | Owner of the structure | Notes |
|---|---|---|---|
| Native menu bar (macOS) | `built_app/src/menu.rs` from `desktop_menu_model::GROUPS` + `model_menu` | this file's rules | fixture `built_app/tests/mac_menu_tree.txt` |
| In-window menu bar (Linux, Windows) | `built_render/src/desktop_menu.rs` | same model | same rows and order as macOS |
| Command palette (⌘K, F3) | `commands::command_registry`, `command_metadata` | registry | every command once, path = its menu home; Sheets has its own scope (`sheet_search.rs`) |
| Top bar: document ▾ | `ui.rs` `bar_menu` "docs" | context | models, New model, Close model, Start page |
| Top bar: Model ▾ / Sheets ▾ | `ui.rs` "work", `ui/dock_menus.rs::model_rows` | context | whole-model rows: rename rows, link selection, quantities, tags, hatch patterns, Plugins |
| Top bar: Scenes ▾ | `ui.rs` "scenes" | context | scene list, Camera inspector…, Add scene from this view, Edit camera / Update / Duplicate / Rename / Move / Delete of the current scene, visibility sets |
| Top bar: View ▾ | `commands::command_chrome`, `ui.rs` `view_popup` | registry (`Surface::ViewMenu`) | Review — hide the panels; Views, Projection, Style, Display, Sections, Settings, Activity |
| Top bar: Render, Library | `ui.rs` | buttons | Render opens the drawer's Render tab (Blender lane); Library ⇧⌘L |
| Project tabs: tab menu | `ui/tabs.rs::menu_rows` | context | New model, Open…, Save, Save As…, Close model / to the right / to the left / all, Move to new window, Picture on hover |
| Viewport and inspector verbs | `verbs.rs`, `viewport_menu.rs`, `ui/context_menu.rs` | docs/context-menu.md | sentence case; repeats Edit's operations for the selection |
| Project tree rows | `ui/tree_rows.rs` | context | node, item, component, link, section, tag and folder rows |
| Dock "⋯", Columns, Filter | `ui/dock_menus.rs` | context | New node, New tag, Propose structure…, Expand / Collapse all, Reveal selection ⇧⌘J, Show all |
| Drawer | `plugin_panels.rs` | tabs | Render, Console, Diagnostics tabs; no menu |
| Extensions' rows | `extension_menu.rs` | extensions | appended to the Extensions menu after its own rows |

## The menu bar

Generated from the fixture (`(system …)` rows are AppKit's own).

| Menu | Item | Command id | Key |
|---|---|---|---|
| Built | About Built | `about` |  |
| Built | Settings… | `prefs` | ⌘, |
| Built | Services ▸ |  |  |
| Built | Hide <application> | `(system hide:)` | ⌘H |
| Built | Hide Others | `(system hideOtherApplications:)` | ⌥⌘H |
| Built | Show All | `(system unhideAllApplications:)` |  |
| Built | Quit Built | `quit` | ⌘Q |
| File | New Model | `new` | ⌘N |
| File | New Project… | `new_project` |  |
| File | New Window | `new_window` | ⇧⌘N |
| File | Open… | `open` | ⌘O |
| File | Open Recent ▸ |  |  |
| File ▸ Open Recent | (no recent documents) |  |  |
| File | Start Page | `home` |  |
| File | Close Model | `close_tab` | ⌘W |
| File | Close Window | `close` | ⇧⌘W |
| File | Save | `save` | ⌘S |
| File | Save As… | `save_as` | ⇧⌘S |
| File | Save a Copy… | `save_copy` | ⌥⌘S |
| File | Recover a Discarded Auto-Save… | `recover_discarded` |  |
| File | Import ▸ |  |  |
| File ▸ Import | SKP Model (.skp)… | `import:skp` |  |
| File ▸ Import | COLLADA (.dae)… | `import:dae` |  |
| File ▸ Import | DXF Drawing (.dxf)… | `import:dxf` |  |
| File ▸ Import | DWG Drawing (.dwg)… | `import:dwg` |  |
| File ▸ Import | RFA Family (.rfa)… | `import:rfa` |  |
| File ▸ Import | glTF (.glb, .gltf)… | `import:glb` |  |
| File ▸ Import | Built Model as a Group (.built, .igz)… | `import:igz` |  |
| File ▸ Import | Wavefront OBJ (.obj)… | `import:obj` |  |
| File ▸ Import | STL (.stl)… | `import:stl` |  |
| File ▸ Import | Photogrammetric Mesh (WebODM)… | `import:photomesh` |  |
| File ▸ Import | Georeference (KML / GeoJSON)… | `import:alignment` |  |
| File ▸ Import | Survey Points (UTM CSV)… | `import:survey_points` |  |
| File ▸ Import | Orthomosaic (GeoTIFF)… | `import:orthomosaic` |  |
| File ▸ Import | Image (PNG / JPG)… | `import:image` |  |
| File ▸ Import | Drawing into a Sketch (.dxf, .svg, .dwg)… | `import:sketch` |  |
| File | Link a Document… | `link` |  |
| File | Reload Linked Files | `reload_links` |  |
| File | Export ▸ |  |  |
| File ▸ Export | Deliveries… | `delivery_export` |  |
| File ▸ Export | IFC (.ifc)… | `export:ifc` |  |
| File ▸ Export | glTF (.glb)… | `export:glb` |  |
| File ▸ Export | COLLADA (.dae)… | `export:dae` |  |
| File ▸ Export | STL (.stl)… | `export:stl` |  |
| File ▸ Export | Wavefront OBJ (.obj)… | `export:obj` |  |
| File ▸ Export | Current View as Image… | `export_image` |  |
| File ▸ Export | Current View as SVG… | `export_view_svg` |  |
| File ▸ Export | Current View as DXF… | `export:dxf` |  |
| File ▸ Export | Scenes as Video… | `export:video` |  |
| File ▸ Export | Save a Copy as .igz (IngeTrazo)… | `export_igz` |  |
| Edit | Undo | `undo` | ⌘Z |
| Edit | Redo | `redo` | ⇧⌘Z |
| Edit | Repeat Last Command | `repeat_last_command` |  |
| Edit | Cut | `cut` | ⌘X |
| Edit | Copy | `copy` | ⌘C |
| Edit | Paste | `paste` | ⌘V |
| Edit | Paste in Place | `paste_in_place` | ⌥⌘V |
| Edit | Select All | `select_all` | ⌘A |
| Edit | Select None | `select_none` | ⌘T |
| Edit | Invert Selection | `invert_selection` | ⇧⌘I |
| Edit | Make Group | `make_group` |  |
| Edit | Make Component… | `verb:make_component` |  |
| Edit | Explode Group | `explode_group` |  |
| Edit | Split into Pieces | `split_pieces` |  |
| Edit | Hide | `hide` |  |
| Edit | Unhide ▸ |  |  |
| Edit ▸ Unhide | Selected | `unhide_selected` |  |
| Edit ▸ Unhide | Last | `unhide_last` |  |
| Edit ▸ Unhide | All | `unhide_all` | ⇧⌘H |
| Edit | Delete Guides | `delete_guides` |  |
| Edit | Intersect Faces ▸ |  |  |
| Edit ▸ Intersect Faces | With Model | `intersect_model` |  |
| Edit ▸ Intersect Faces | With Selection | `intersect_selection` |  |
| Edit ▸ Intersect Faces | With Context | `intersect_context` |  |
| Edit | Soften / Smooth Edges… | `soften` |  |
| Edit | Reverse Faces | `reverse_faces` |  |
| Edit | Orient Faces | `orient_faces` |  |
| Edit | Heal Overlapping Faces | `heal_overlaps` |  |
| Edit | Rebuild Faces (Planar) | `rebuild_planar` |  |
| Edit | Weld Edges | `weld_edges` | `join_segments` is an alias kept for scripts |
| Edit | Simplify Mesh… | `verb:simplify_mesh` |  |
| Edit | Convert Path to Geometry | `convert_path` |  |
| View | Orbit | `orbit` |  |
| View | Pan | `pan` |  |
| View | Zoom | `zoom` |  |
| View | Zoom Window | `zoom_window` |  |
| View | Zoom Extents | `zoom_extents` | ⇧⌘E |
| View | Zoom Selection | `zoom_selection` | ⌥⌘Z |
| View | Look at Face | `look_at_face` | V (the face under the cursor; right-click a face for the same) |
| View | Standard Views ▸ |  |  |
| View ▸ Standard Views | Top | `view_top` | ⌘1 |
| View ▸ Standard Views | Front | `view_front` | ⌘2 |
| View ▸ Standard Views | Right | `view_right` | ⌘3 |
| View ▸ Standard Views | Isometric | `view_iso` | ⌘4 |
| View ▸ Standard Views | Back | `view_back` | ⌘5 |
| View ▸ Standard Views | Left | `view_left` | ⌘6 |
| View ▸ Standard Views | Bottom | `view_bottom` | ⌘7 |
| View | Projection ▸ |  |  |
| View ▸ Projection | Perspective | `projection_perspective` |  |
| View ▸ Projection | Parallel Projection | `projection_parallel` |  |
| View ▸ Projection | Two-Point Perspective | `projection_two_point` |  |
| View | Field of View… | `verb:field_of_view` |  |
| View | Camera Inspector… | `camera` |  |
| View | Walkthrough ▸ |  |  |
| View ▸ Walkthrough | Look Around | `look_around` |  |
| View ▸ Walkthrough | Walk | `walk` |  |
| View ▸ Walkthrough | Position Camera | `position_camera` |  |
| View ▸ Walkthrough | First Person | `tool:first_person` |  |
| View | Style ▸ |  |  |
| View ▸ Style | Default | `style_default` |  |
| View ▸ Style | Architectural | `style_architectural` |  |
| View ▸ Style | Shaded | `style_shaded` |  |
| View ▸ Style | Hidden line | `style_hidden_line` |  |
| View ▸ Style | Monochrome | `style_monochrome` |  |
| View ▸ Style | Wireframe | `style_wireframe` |  |
| View ▸ Style | Edit Style… | `form-style` |  |
| View | X-ray | `xray` |  |
| View | Axes | `axes` |  |
| View | Back Edges | `back_edges` |  |
| View | Hidden Geometry | `hidden_geometry` |  |
| View | Shadows | `shadows` |  |
| View | Shadows and Sun… | `form-shadows` |  |
| View | Sections ▸ |  |  |
| View ▸ Sections | Section Cuts | `section_cuts` |  |
| View ▸ Sections | Show All Cutters | `show_cutters` |  |
| View ▸ Sections | Section Planes | `section_planes` |  |
| View ▸ Sections | Section Fill | `section_fill` |  |
| View | Scenes ▸ |  |  |
| View ▸ Scenes | Add Scene | `scene_add` |  |
| View ▸ Scenes | Show Scene | `scene_show` |  |
| View ▸ Scenes | Update Scene | `scene_update` |  |
| View ▸ Scenes | Rename Scene… | `scene_rename` |  |
| View ▸ Scenes | Duplicate Scene | `scene_duplicate` |  |
| View ▸ Scenes | Delete Scene | `scene_delete` |  |
| View ▸ Scenes | Previous Scene | `scene_previous` |  |
| View ▸ Scenes | Next Scene | `scene_next` |  |
| View ▸ Scenes | Organise Scenes | `organise-scenes` |  |
| View | Animation ▸ |  |  |
| View ▸ Animation | Scene Transitions | `scene_transitions` |  |
| View ▸ Animation | Play / Stop | `scene_play` |  |
| View ▸ Animation | Animation Settings… | `animation_settings` |  |
| View | Rest of Model While Editing ▸ |  |  |
| View ▸ Rest of Model While Editing | Show Normally | `rest_normal` |  |
| View ▸ Rest of Model While Editing | Fade | `rest_fade` |  |
| View ▸ Rest of Model While Editing | Hide (Fastest) | `rest_hide` |  |
| View ▸ Rest of Model While Editing | Fade Intensity… | `rest_intensity` |  |
| View ▸ Rest of Model While Editing | Hide Similar Components | `hide_similar` |  |
| View | Model Views ▸ |  |  |
| View ▸ Model Views | One view | `layout:single` |  |
| View ▸ Model Views | Four views | `layout:four` |  |
| View ▸ Model Views | Two views | `layout:two` |  |
| View ▸ Model Views | Three views | `layout:three` |  |
| View ▸ Model Views | Two views, one above the other | `layout:two_stacked` |  |
| View ▸ Model Views | Three views, the large one on the right | `layout:three_mirror` |  |
| View ▸ Model Views | Two views, one wider | `layout:two_wide` |  |
| View ▸ Model Views | Maximize / Restore View | `maximize_model_view` |  |
| View | Show Keystrokes | `show_keystrokes` | check; the overlay for recordings (docs/recording-beats.md) |
| View | Toggle Full Screen | `(system toggleFullScreen:)` | ⌃⌘F |
| Model | Site Placement… | `site_placement` |  |
| Model | Units… | `units` | ⌘U |
| Model | Cut List… | `cutlist` |  |
| Draw | Line | `tool:line` |  |
| Draw | Freehand | `tool:freehand` |  |
| Draw | Path | `tool:geopath` |  |
| Draw | Rectangle | `tool:rectangle` |  |
| Draw | Rotated Rectangle | `tool:rotated_rect` |  |
| Draw | Circle | `tool:circle` |  |
| Draw | Polygon | `tool:polygon` |  |
| Draw | Arcs ▸ |  |  |
| Draw ▸ Arcs | Arc | `tool:arc` |  |
| Draw ▸ Arcs | 3-Point Arc | `tool:arc3` |  |
| Draw ▸ Arcs | Center Arc | `tool:center_arc` |  |
| Draw ▸ Arcs | Pie | `tool:pie` |  |
| Draw | New Profile… | `sketch:new` |  |
| Draw | New Section… | `sketch:new:section` |  |
| Tools | Select | `tool:select` |  |
| Tools | Eraser | `tool:eraser` |  |
| Tools | Paint | `tool:paint` |  |
| Tools | Push/Pull | `tool:pushpull` |  |
| Tools | Move | `tool:move` |  |
| Tools | Rotate | `tool:rotate` |  |
| Tools | Scale | `tool:scale` |  |
| Tools | Flip | `tool:flip` |  |
| Tools | Follow Me | `tool:followme` |  |
| Tools | Offset | `tool:offset` |  |
| Tools | Fillet | `tool:fillet` |  |
| Tools | Chamfer | `tool:chamfer` |  |
| Tools | Outer Shell | `tool:outer_shell` |  |
| Tools | Solid Tools ▸ |  |  |
| Tools ▸ Solid Tools | Intersect | `tool:solid_intersect` |  |
| Tools ▸ Solid Tools | Union | `tool:solid_union` |  |
| Tools ▸ Solid Tools | Subtract | `tool:solid_subtract` |  |
| Tools ▸ Solid Tools | Trim | `tool:solid_trim` |  |
| Tools ▸ Solid Tools | Split | `tool:solid_split` |  |
| Tools | Bridge | `bridge` |  |
| Tools | Make Faces | `make_faces` |  |
| Tools | Tape Measure | `tool:tape` |  |
| Tools | Protractor | `tool:protractor` |  |
| Tools | Dimension | `tool:dimension` |  |
| Tools | Text | `tool:text` |  |
| Tools | 3D Text… | `text3d` |  |
| Tools | Section Plane | `tool:section` |  |
| Tools | North | `tool:north` |  |
| Tools | Terrain Profile of Selection | `terrain_profile` |  |
| Tools | Diagnostics | `panel:solid_inspector` |  |
| Tools | Python Console | `panel:python_console` | ⇧⌘P |
| Tools | Clear Imported Texture Cache… | `clear_texture_cache` |  |
| Extensions | Extension Manager… | `extensions` |  |
| Extensions | Open Extensions Folder | `plugins_folder` |  |
| Extensions | Develop an Extension… | `develop_plugin` |  |
| Extensions | Blender Render | `panel:render_blender` |  |
| Window | Minimize | `(system performMiniaturize:)` | ⌘M |
| Window | Zoom | `(system performZoom:)` |  |
| Window | Next Model | `next_doc` | ⌃Tab |
| Window | Previous Model | `prev_doc` | ⌃⇧Tab |
| Window | Move Model to New Window | `move_to_window` |  |
| Window | Project | `sidebar_project` | ⌥⌘1 |
| Window | Tasks | `sidebar_tasks` | ⌥⌘2 |
| Window | Deliveries | `sidebar_deliveries` | ⌥⌘3 |
| Window | Library | `library` | ⇧⌘L |
| Window | Recent Activity | `recent-activity` |  |
| Window | Ask an Agent… | `ask_agent` | ⇧⌘A |
| Window | Review (Hide Panels) | `review` | ⌘0 |
| Help | Documentation | `documentation` |  |
| Help | Get More Models and Textures… | `get_models` |  |
| Help | Check for Updates… | `check_updates` |  |
