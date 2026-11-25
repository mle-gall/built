# The viewport right-click menu

The menu shows only the verbs that apply to what you right-clicked. It has
no More › and no submenus. Related verbs share one row as segments (for
example **Intersect: Model | Selection | Context**). No kind has more than
14 rows under its header.

The native shell draws this menu with egui. The Select row grows the selection in the current editing context and leaves document history untouched. Extensions can declare actions in the Plugins menu. Their legacy right-click callbacks have no native host.

## How it works

1. **Right-clicking an unselected object selects it first**, as in
   .skp (`Viewport._context_pick`). Right-clicking inside the selection
   keeps the selection, and the menu then acts on all of it. A right-click
   on nothing clears nothing and gives the empty-space menu.
2. **Header.** It names what was hit, its kind, and where it is filed: the
   group's node path in the project tree (`Building A › Level 2 › Catwalk`),
   or, for loose geometry, the group being edited (or "Model"). A linked
   file's row says "read-only". With several things selected the header
   counts them (`5 items · 3 groups, 2 faces`).
3. **Row 1: Repeat.** It shows the last command, with its value when the
   command has one (`Repeat Push/Pull  450 mm`), and the key ⇧R. It is greyed
   when nothing has run yet. It is the same `MainWindow.repeat_last_command`
   as Edit ▸ Repeat.
4. **Then the verbs for the selection's kind.** The tree's row verbs come
   first, in the tree's order (Open for editing, Rename…, Hide, Isolate,
   Lock, Select instances, Make unique, Zoom to, File to…). Next come what
   the selection can become (Make group, Make component…, Explode). Last are
   the geometry commands that used to sit under More, and only where they
   apply.
5. **One implementation.**
   - A verb shared with the tree runs the tree's own action
     (`wgpu_ui._tree_action` on the row key `i:<uid>`), which goes through
     `core.protocol` (`hide`, `lock`, `rename_placement`…).
   - Every other verb is the Edit menu's slot on the window
     (`_on_reverse_faces`, `_on_intersect_faces(mode)`, `_on_explode_group`…).
     These are the same slots the classic QMenu, the menu bar and the
     selection bar call.
   - `rust/built_render/src/viewport_menu.rs` reads the selection and builds the
     rows. `rust/built_render/src/ui/context_menu.rs` draws them and reports `ctx:<id>`.
6. **Read-only kinds offer no editing verbs.** A linked file's placement, and
   a part inside one, never get Explode, Open for editing, Rename, File to,
   Make unique, Intersect, Soften or Flip. Repeat on them is greyed.

Cut, Copy, Paste, Delete, Select all and Undo are shortcuts every app shares,
so they stay on the keyboard and the Edit menu. The exception is empty
space, where **Paste here** is spatial. Camera items are left out, because
the top bar's navigation cluster shows them all. The exception, again, is
**Zoom extents** on empty space.

Reveal selection shares a row with Zoom to. Editable selections that can take a tag also show Tag… in that row. It opens the selection’s assignment and visibility choices. It opens the branches needed to show the selected occurrence in the Project tree. Its shortcut is ⇧⌘J on macOS and Ctrl+Shift+J elsewhere.

## The table

`|` separates segments of one row. *(if …)* marks a row that appears only
when its condition holds. Row counts leave out the header and include
Repeat.

### Raw geometry

Isolate is left out: it shows placements, and loose geometry has none.

| # | Faces only | Edges only | Faces and edges |
|---|---|---|---|
| header | `Face` · `12.00 m²` · loose geometry · Model *(or the group being edited)*; with several: `n faces` | `Edge` · `4.00 m` · loose geometry · …; with several: `n edges` | `n items` · `a faces, b edges` |
| 1 | Repeat … | Repeat … | Repeat … |
| 2 | Hide | Hide | Hide |
| 3 | Zoom to \| Reveal selection \| Tag… | Zoom to \| Reveal selection \| Tag… | Zoom to \| Reveal selection \| Tag… |
| 4 | Make group \| Make component… | Make group \| Make component… | Make group \| Make component… |
| 5 | Reverse faces \| Orient faces | Make faces *(if they close a loop)* | Reverse faces \| Orient faces |
| 6 | Intersect: Model \| Selection \| Context | Divide… | Intersect: Model \| Selection \| Context |
| 7 | Soften / Smooth edges… | Join segments \| Explode curve *(two or more; a curve)* | Soften / Smooth edges… |
| 8 | Heal overlapping faces | Soften / Smooth edges… | Make faces *(if they close a loop)* |
| 9 | Rebuild faces (planar) | Bridge *(two loops)* | Heal overlapping faces |
| 10 | Simplify mesh… | Flip along: Red \| Green \| Blue | Rebuild faces (planar) |
| 11 | Flip along: Red \| Green \| Blue | | Simplify mesh… |
| 12 | Align view *(one face)* | | Flip along: Red \| Green \| Blue |
| 13 | Texture: Position \| Reset *(a textured face)* | | |
| 14 | Select: Connected \| Boundary \| Same material \| Same tag | Select: Connected \| Same tag | Select: Connected \| Boundary \| Same material \| Same tag |

### Groups and components

| # | One group | One component instance | Several groups / components |
|---|---|---|---|
| header | `Box A` · Group · `Building A › Level 2 › Catwalk` *(its node path, or Unfiled)* | `Door` · Component · `n instances` · path | `n items` · `g groups, c components` |
| 1 | Repeat … | Repeat … | Repeat … |
| 2 | Open for editing | Open for editing | Hide |
| 3 | Rename… | Rename… | Isolate |
| 4 | Hide \| Isolate | Hide \| Isolate | Lock *(Unlock when all are locked)* |
| 5 | Lock *(Unlock when locked)* | Lock *(Unlock when locked)* | Zoom to \| Reveal selection \| Tag… |
| 6 | Zoom to \| Reveal selection \| Tag… | Zoom to \| Reveal selection \| Tag… | File to… |
| 7 | File to… | File to… | Make group \| Make component… |
| 8 | Explode \| Make component… | Select instances \| Make unique | Merge groups \| Explode |
| 9 | Split into pieces | Explode \| Replace component… *(Replace when another definition fits)* | Make unique \| Replace component… \| Keep in file *(if components and applicable)* |
| 10 | Intersect: Model \| Selection \| Context | Keep in file *(check)* | Intersect: Model \| Selection \| Context |
| 11 | Soften / Smooth edges… | Intersect: Model \| Selection \| Context | Solid: Union \| Intersect \| Split \| Outer shell *(all solids; Split for two)* |
| 12 | Flip along: Red \| Green \| Blue | Flip along: Red \| Green \| Blue | Soften / Smooth edges… |
| 13 | Change axes | Change axes | Flip along: Red \| Green \| Blue |
| 14 | Select: Same material \| Same tag | Select: Same material \| Same tag | Select: Same material \| Same tag \| Instances |

### Mixed, linked, annotations, guides, empty space

| # | Geometry and groups | Linked file's placement *(read-only)* | Part inside a linked file *(read-only)* | Dimension or text | Guide | Empty space |
|---|---|---|---|---|---|---|
| header | `n items` · `g groups, f faces, e edges` | `leaf.built ↗` · Linked file · read-only · path | `Stringer-02` · in leaf.built ↗ · read-only | `Dimension` · `4.00 m` / `Text` · “first words” | `Guide line` / `Guide point` | `Nothing here` · Model *(or the group being edited)* |
| 1 | Repeat … | Repeat … *(greyed)* | Repeat … *(greyed)* | Repeat … | Repeat … | Repeat … |
| 2 | Hide | Select contents | Select its linked file | Edit text… | Delete guide | Paste here *(if something is copied)* |
| 3 | Isolate | Hide | Hide | Zoom to \| Reveal selection \| Tag… | Delete all guides | Select all |
| 4 | Zoom to \| Reveal selection \| Tag… | Isolate | Isolate | | Zoom to \| Reveal selection \| Tag… | Unhide all |
| 5 | File to… | Lock *(the placement's own padlock)* | Zoom to \| Reveal selection | | | Show all *(if isolated)* |
| 6 | Make group \| Make component… | Zoom to \| Reveal selection | Edit source | | | Zoom extents |
| 7 | Reverse faces \| Orient faces *(if faces)* | Edit source | Reload | | | Delete all guides *(if any)* |
| 8 | Intersect: Model \| Selection \| Context | Reload | | | | |
| 9 | Soften / Smooth edges… | Include nested files *(check)* | | | | |
| 10 | Flip along: Red \| Green \| Blue | | | | | |
| 11 | Select: Connected \| Boundary \| Same material \| Same tag \| Instances | | | | | |

Hide is not offered on texts, dimensions or guides: Edit ▸ Hide does not
act on them (`MainWindow._hideable`). On a part inside a linked file it
hides this document's view of the part, as the tree's eye does, and edits
nothing in the source.

The rows, labels, keys and checks of every kind come from one table,
`rust/built_render/src/verbs.rs`. The inspector offers the same entries
(same ids and labels; a few shorter chip names: Edit, Group, Component), and
a linked file's tree row takes its file verbs from it too.

### Other hits

These had their own rows in the classic menu and keep them, so they still
have a home.

| Kind | Rows after Repeat |
|---|---|
| Section plane | Section: Reverse \| Active cut \| Align view · Name and symbol… · Zoom to \| Reveal selection \| Tag… |
| Image | Image: Size… \| Opacity… · Explode image · Lock *(check)* · Zoom to \| Reveal selection \| Tag… |
| Geo path | Terrain profile · Terrain surface: Flat \| Draped \| None *(closed)* · Convert path to geometry · Open / close path · Reveal selection |
| Locked group or image under the pointer | Unlock “name” (Image: Unlock “name” \| Delete) on the second row, before the selection's own rows |

### Where the old More items went

| Command | Kinds |
|---|---|
| Intersect: Model / Selection / Context | faces, faces and edges, groups, components, several groups, geometry and groups |
| Soften / Smooth edges… | faces, edges, faces and edges, groups, several groups, geometry and groups |
| Reverse faces \| Orient faces | faces, faces and edges, geometry and groups *(with faces)* |
| Heal overlapping faces | faces, faces and edges |
| Join segments | edges *(two or more)* |
| Rebuild faces (planar) | faces, faces and edges |
| Split into pieces | one group |
| Simplify mesh… | faces, faces and edges |
| Convert path to geometry | geo path |
| Delete guides | guide (this one or all), empty space *(when the model has guides)* |

The selection bar's More ▾ is unchanged on this branch. The menu does not
depend on it.

## Tree row parity

| Tree item row (`ui.rs` `item_menu`) | Viewport |
|---|---|
| Select | the right-click itself selects |
| Open for editing | Open for editing |
| Rename | Rename… (an inline field on the dock's prompt) |
| Hide | Hide |
| Lock / unlock | Lock (Unlock when locked) |
| Select instances | Select instances (components) |
| Make unique | Make unique (components) |
| Zoom to \| Reveal selection | Zoom to \| Reveal selection |
| File to… | File to… |

A linked row's tree menu (Select, Select contents, Isolate, Zoom to,
Show / hide, Lock / unlock, Edit source, Reload) matches the linked
placement's column above.
