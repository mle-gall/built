# UI patterns

Patterns the Built chrome (egui in the Metal frame, `rust/built_render/src/ui*.rs`)
reuses from one surface to the next. A new surface picks one of these
before it is designed; a surface that fits none is a design question for
Maxence, not a new one-off.

## The large sheet

> A large sheet over the dimmed, blurred model: browse, pick, set.

Chosen on 2026-11-23 for three surfaces at once ("I see a pattern
emerging here"): the **Library** (option B with C's palette), the **start
page** (option C) and **Settings** (option B's content). One-off **Export**, **Extension
Manager**, **About** and report sheets also use this component.

### When to use it

- The surface is **browsed**: more than a menu or a 300 pt dock column can
  hold (hundreds of materials, every setting, every recent file), with a
  search across all of it.
- It is **visited, then left**: you open it, find or set something, and
  go back to the model. Nothing in it needs the model visible at the same
  time. (What does — painting, filing, editing a material while looking at
  it — stays in the dock, the strip or the inspector; the sheet links to it.)
- It is **not a question**. A question that changes data is the small
  sheet hung from the top bar (`set_sheet`, `built_render.chrome_dialogs`, `dialogs_host.rs`); a
  form with OK/Cancel is a chrome form (`built_render.chrome_forms`, `forms_host.rs`). Passing
  information is a toast or the activity capsule.

### Anatomy

```
 top bar (traffic lights and project tabs)
┌──────────────────────────────────────────────────────────────────────┐
│ title · tabs · verbs ……………………………………… menus · [⌕ search ⌘F] · ×  │ 52 pt
├──────────────┬────────────────────────────────────┬──────────────────┤
│ side list    │ content                            │ detail           │
│ CAPTION      │ Title  sub                         │ picture          │
│ scope    12  │ CAPTION                            │ heading  right   │
│  ▾ group     │ ▢ ▢ ▢ ▢ ▢  tiles (96 pt)           │ label     value  │
│    item   3  │ ▭ row  sub ………… dot · right        │ [fields]         │
│ CAPTION      │ [fields: label | control]          │ [Primary ↩]      │
│ 🗋 file   4  │                                    │ quiet · quiet    │
├──────────────┴────────────────────────────────────┴──────────────────┤
│ links · links ……………………………………………………………… key hints              │ 32 pt
└──────────────────────────────────────────────────────────────────────┘
 model controls retracted
```

- **Geometry.** Centre the card in the space below the project tabs. It takes
  84% of that height up to 760 pt. Horizontal margins are 6% of window width,
  clamped to 24–80 pt, with the provider's maximum width respected.
- **Chrome.** The rail, dock and bottom bar use the shared 160 ms Review
  retraction. The top bar retains the traffic lights and project tabs. On
  dismissal the window restores its previous Review or editing state.
  Paint's palette strip belongs to the model and returns with its controls.
- **Backdrop.** Sample the model across the full window. On a blank Start,
  use the last opened or saved file's cached preview when available, then
  other recent file previews, then the empty scene. Use a centred cover crop.
  The shared Kawase blur uses 1.75× tap offsets. A neutral 44% scrim and
  a soft vignette that attenuates edge light by up to 42% sit behind the card.
- **Surface.** The card and header sample the same blur through a 55% graphite
  tint. Lists and fields use the denser `FIELD` fill. Panes use `DIVIDER`
  hairlines, pictures use `WELL`, and link dots use feedback tokens.
  Buttons use the kit's sheet buttons and fields use the forms'
  widgets (`ui/dialog.rs`), so a setting looks the same in a form and a
  sheet. Blue only as the accent: the focused tile, the selected row, the
  primary button, links. No navy surfaces or washes.
- **Motion.** In and out in 160 ms: the dim fades, the sheet rises 14 pt
  and fades. Closing keeps the last content while it goes.

### Behaviour (the same on every sheet)

- **Input ownership.** While a sheet is up it takes the pointer everywhere
  under the top bar (the model is neither hovered nor picked) and every
  key, except the window's command shortcuts (⌘K, ⌘O, ⌘N, ⌘W, ⌘Q, ⌘,
  …): `ui.rs` `claims_key`, `views/chrome_sheet.py` `claims_key`. The top
  bar stays live. A question sheet, a chrome form or the command palette
  can open over it and take the keys while they are up.
- **Esc** closes the innermost thing: a menu, then a field being typed in,
  then the search text, then the sheet (when it can close: the start page
  with only a blank model behind it cannot). The model's view gets the
  keyboard back.
- **Search.** Typing anywhere (no field focused) types into the search;
  ⌘F focuses it. The provider filters: a search looks across every
  section, never only the one in front, and says where each hit is from.
- **Focus.** One focused item in the content (accent ring). Arrows walk the
  tiles and rows by their layout; Enter or a double-click *opens* it (the
  sheet's main verb: paint with it, open the file); a click focuses it and
  the detail pane shows it. Tab walks the side list. A right-click opens
  the item's menu.
- **Apply at once.** A sheet has no OK/Cancel: edits apply as they are
  made (sliders and number fields when they settle, one undo step), and a
  note says when one waits for the next start.

### Building one

The component is Rust (`ui/large_sheet.rs`): layout, state (open,
closing, search text, focused item, menus, the forms' typing), keys, the
pointer, the animation. What it shows is a `Spec` (serde, JSON from Python
today): `id`, `title`, `tabs`, `verbs`, `menus`, `search`, `closable`,
`side` items, `content` and `detail` panes of **blocks** (`caption`,
`text`, `tiles`, `rows`, `fields`, `picture`, `heading`, `kv`, `buttons`,
`separator`), `footer`, `focus`. Everything comes back as one action
string, `lsheet:<id>\x1f<verb>[\x1f<arg>…]` (verbs: `close`, `search`,
`side`, `fold`, `tab`, `verb`, `focus`, `open`, `menu`, `pick`,
`headmenu`, and `v`/`b` for fields).

From Python, a surface is a provider (`views/chrome_sheet.py`):

```python
class OneOffExport(chrome_sheet.Provider):
    id = "export"

    def build(self, ctx):
        s = chrome_sheet.Sheet(self.id, "Export")
        s.search("Search formats")
        for fmt in FORMATS:
            s.side_item(fmt.id, fmt.label, on=fmt.id == self.fmt)
        c = s.content(self.label(), self.scope_text())
        c.fields(self.options_form())          # chrome_forms.Form: applied at once
        d = s.detail()
        d.caption("Before it runs")
        d.rows([chrome_sheet.item("", c.text, dot=c.tone, fixed=True) for c in self.checks()])
        d.buttons([chrome_sheet.verb("keep", "Keep as delivery"),
                   chrome_sheet.verb("run", "Export", key="↩", primary=True)])
        return s

    def act(self, ctx, verb, args):
        if verb == "side":
            self.fmt = args[0]
        elif verb == "verb" and args[0] == "run":
            ...                                # a job in the activity capsule

chrome_sheet.open(win, OneOffExport())
```

`build` runs again after every action and whenever `signature(ctx)`
changes; only a changed sheet is sent. `closed(ctx)` runs when it goes
(`ctx.replaced` when another sheet took its place: the start page comes
back after Settings). The Qt host closes an undrawn sheet and reports a missing frame. Tests check every sheet they send against the Rust schema
with `chrome_sheet.check_wire`.

By the port rule of 2026-11-23 (`docs/rust-port-plan.md`), a **new**
sheet's view model goes in Rust: a Rust provider fills the same `Spec`
from the Rust document and the store, and only calls into Python where a
subsystem is still Python (`docs/python-glue.md`).

### The three sheets today

| Sheet | Side list | Content | Detail | Enter / double-click |
|---|---|---|---|---|
| Library (`views/wgpu_library.py`) | In this model, Recently used, My library, Bundled ▾ categories, *Linked files · read-only* | 96 pt tiles; a search groups hits by where they are | picture, heading, the material editor (base, colour/image, tile, PBR, opacity, rename) or read-only facts | paint with it / place a copy (closes the sheet) |
| Start page (`views/wgpu_start.py`) | — | Continue, Pinned, Recent, Projects, Templates as dense rows | picture, project, where it was left, problems, unfiled, linked files and their state | open, continue, new from template |
| Settings (`views/settings_sheet.py`) | Built · this Mac sections, This model sections | the section's fields; a search lists hits from every section | — (Keyboard: the command in focus) | — |

The Library's fast path is not the sheet: the **Paint strip** (the
model's materials, the recent ones, the current name, *Browse all…*) and
the dock's **inspector**, which edits the material Paint holds.
