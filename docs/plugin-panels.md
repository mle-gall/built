# Plugin panels

The native Built shell and the Qt app with the Metal chrome draw the same egui panels. Select **Plugins** in the right dock, then select a panel. A dock panel replaces the project tree. A utility panel opens above the bottom bar and has its own resize handle and Close button. The native Plugins menu opens the bundled panels directly. The Console and Blender render preview use these utilities wherever the native chrome is attached. The classic Blender settings dock still supplies its own light controls.

## Register a panel

Expose a `create_panel(app, invalidate)` factory in the discovered plugin module. Built calls it once per document view. `app` is an `ExtensionApp`. The object returned by the factory supplies the panel version, local ID, title, slot, descriptor and event handler.

```python
from core.plugin_panels import field, item, section


class Counter:
    api = 1
    id = "count"
    title = "Object count"
    slot = "dock"

    def __init__(self, app, invalidate):
        self.app = app
        self.count = 0

    def sections(self):
        return [section("objects", "Objects",
                        item("button", "count", "Count groups"),
                        field("result", "Groups", "number", self.count,
                              min=0, max=max(1, self.count), enabled=False))]

    def event(self, control, event, value):
        if (control, event) == ("count", "invoke"):
            self.count = len(self.app.scene.groups)


create_panel = Counter
```

Use `utility` for a console or other wide output. Section IDs must be unique in a panel. Control IDs are unique across all its sections. Reserve IDs beginning with an underscore for the host. A utility model can set `notify_close = True` to receive `event("_panel", "closed", None)` when its Close button is pressed. Every item accepts a plain-text label and an optional `enabled` flag.

## Controls in version 1

| Declaration | Data | Event and value |
| --- | --- | --- |
| Field with type `text` or `path` | String value | `commit` with a string on Enter or blur. A path also offers the host file picker. |
| Field with type `number` | Numeric value, required min and max, optional unit suffix | `commit` with a number |
| Field with type `toggle` | Boolean value | `commit` with a boolean |
| Field with type `choice` | String value and an array of allowed strings in `options` | `commit` with the selected string |
| Field with type `colour` | Three RGB bytes in an array | `commit` with three RGB bytes |
| `button` | Label | `invoke` with null |
| `list` or `table` | Column labels, rows with a unique string `key` and one string per column in `cells`, optional selected key in `selection` | `selection` with a row key |
| `log` | Plain-text `lines` array | Scrollable transcript |
| `console` | String array `history` in submission order | `console_submit` with source text. Run or Ctrl/⌘-Enter submits. Up and Down recall history at the prompt boundary. |
| `progress` | Label and `fraction` between 0 and 1, or null during indeterminate work | Display only |
| `image` | Local `path` to a completed RGB or RGBA PNG | Fit and actual-size preview |

Keep transcripts and history bounded. The renderer caches the preview texture by path. Give each finished image a new path. For document edits use the viewport history or the existing command API.

## Transport and lifetime

The bridge adds `api`, `plugin` (the extension filename stem), `panel`, `instance`, `document`, `revision`, `slot` and `title` to the sections for Rust. Events return the same identity and revision with a monotonic `event_id`, `control`, `event` and JSON `value`. The bridge checks the active document, instance, revision and event ID before invoking the handler. A file picker result is checked again after the picker returns. Schema errors and unsupported versions appear in the panel with Retry.

Built refreshes a descriptor after an event or a scene revision change. Call `invalidate()` on the application thread when asynchronous progress or results change. Use `ingetrazo_render.chrome_dialogs.Job(app.window, label, on_cancel=...)` for long-running work. Snapshot the active scene and camera before starting a worker. Update and end the job on the application thread. Closing the panel leaves the job in the activity capsule.

For a snapped point, call `app.pick_point(prompt, done)`. The host routes the next viewport click to `done(point, on_face)` for the original scene revision. Escape cancels and restores the Select tool.
