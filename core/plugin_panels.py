"""Bridge for versioned plugin data. Widget state and layout live in egui."""
import json
import uuid

from views.extension_api import ExtensionApp


def item(kind, id, label="", **data):
    return dict(kind=kind, id=id, label=label, **data)


def field(id, label, type, value, **data):
    return item("field", id, label, type=type, value=value, **data)


def section(id, title, *items):
    return dict(id=id, title=title, items=list(items))


def install(win, plugins):
    win._panel_factories = {p.stem: p.panel for p in plugins if p.panel}


def on_document_close(app, close):
    """Release document resources when its tab or owning window closes."""
    scene = app.scene
    window = app.window

    def finished(closed_scene=None):
        if closed_scene is not None and closed_scene is not scene:
            return
        window._doc_close_hooks.remove(finished)
        window._extension_closed.disconnect(finished)
        close()

    app.on_document_closed(finished)
    window._extension_closed.connect(finished)


def open_panel(win, key):
    from ingetrazo_render.chrome_dialogs import view_of
    view = view_of(win)
    if not getattr(view, "_ui_attached", False) or not hasattr(view.renderer, "ui_plugin_open"):
        return False
    sync(view, win)
    for binding in getattr(view, "_plugin_bindings", {}).values():
        if binding.key == key:
            opened = getattr(binding.model, "on_open", None)
            if opened is not None:
                opened()
                binding.invalidate()
                binding.push()
            view.renderer.ui_plugin_open(binding.instance)
            view.update()
            return True
    return False


def show_utility(win, key, model):
    """Publish or replace one utility for this document, keeping its zoom and position."""
    from ingetrazo_render.chrome_dialogs import view_of
    view = view_of(win)
    if not getattr(view, "_ui_attached", False):
        return None
    sync(view, win)
    bindings = getattr(view, "_plugin_bindings", None)
    if bindings is None or view._plugin_scene is not win.viewport.scene:
        return None
    binding = next((b for b in bindings.values() if b.key == key), None)
    if binding is None:
        binding = Binding(win, key, lambda *_: model, view)
        bindings[binding.instance] = binding
    else:
        binding.model = model
    binding.invalidate()
    binding.push()
    view.renderer.ui_plugin_open(binding.instance)
    return binding


class Binding:
    def __init__(self, win, key, factory, view):
        self.key = key
        self.instance = uuid.uuid4().hex
        self.document = view._plugin_document
        self.last_sections = "[]"
        self.last_event = 0
        self.scene = win.viewport.scene
        self.dirty = True
        self.sent_revision = None
        self.error = ""
        self.view = view
        self.model = factory(ExtensionApp(win, key), self.invalidate)
        self.id = self.model.id
        self.title = self.model.title
        self.slot = self.model.slot
        if not all(isinstance(v, str) and v.strip() for v in (self.id, self.title)):
            raise ValueError("A panel requires a nonempty string ID and title")

    def invalidate(self):
        self.dirty = True
        try:
            self.view.update()
        except RuntimeError:
            # A render may finish after its document view was closed.
            pass

    def push(self):
        revision = self.scene.version
        if not self.dirty and self.sent_revision == revision:
            return
        self.dirty = False
        self.sent_revision = revision
        model = self.model
        envelope = dict(api=getattr(model, "api", 1), plugin=self.key, panel=self.id,
                        title=self.title, slot=self.slot, instance=self.instance, document=self.document, revision=revision)
        try:
            sections = model.sections()
            clean_sections = json.dumps(sections, allow_nan=False)
            if self.error:
                sections = [*sections, section("_error", "Plugin error",
                            item("log", "_exception", lines=[self.error]))]
            self.view.renderer.ui_plugin_panel(json.dumps(dict(envelope, sections=sections), allow_nan=False))
            self.last_sections = clean_sections
        except (Exception, SystemExit) as exc:
            envelope.update(api=1, slot=self.slot if self.slot in ("dock", "utility") else "dock",
                            sections=[*json.loads(self.last_sections), section("_error", "Panel unavailable",
                                      item("log", "_exception", lines=[str(exc)]),
                                      item("button", "_retry", "Retry"))])
            try:
                self.view.renderer.ui_plugin_panel(json.dumps(envelope))
            except ValueError:
                # Even an extension using the reserved error IDs is isolated.
                envelope["sections"] = envelope["sections"][-1:]
                self.view.renderer.ui_plugin_panel(json.dumps(envelope))


def sync(view, win):
    if "_panel_factories" not in vars(win):
        return
    if view is not getattr(getattr(win, "_doc", None), "stand_in", None) or not hasattr(view.renderer, "ui_plugin_panel"):
        return
    if getattr(view, "_plugin_scene", None) is not win.viewport.scene:
        view._plugin_scene = win.viewport.scene
        view._plugin_document = uuid.uuid4().hex
        view._plugin_bindings = {}
        view._plugin_factories_seen = set()
        view._ui_plugins_sent = False
    for key, factory in win._panel_factories.items():
        if key in view._plugin_factories_seen:
            continue
        view._plugin_factories_seen.add(key)
        try:
            binding = Binding(win, key, factory, view)
        except (Exception, SystemExit):
            import logging
            logging.getLogger(__name__).exception("Plugin panel %s failed to load", key)
            continue
        view._plugin_bindings[binding.instance] = binding
    if not getattr(view, "_ui_plugins_sent", False):
        live = [scope for doc in win._documents
                if (scope := getattr(getattr(doc, "stand_in", None), "_plugin_document", None))]
        view.renderer.ui_plugin_document(view._plugin_document, live)
        for binding in view._plugin_bindings.values():
            binding.dirty = True
            binding.last_event = 0
        view._ui_plugins_sent = True
    for binding in view._plugin_bindings.values():
        binding.push()


def dispatch(win, raw):
    from ingetrazo_render.chrome_dialogs import view_of
    view = view_of(win)
    event = json.loads(raw)
    binding = getattr(view, "_plugin_bindings", {}).get(event.get("instance"))
    if (binding is None or binding.scene is not win.viewport.scene
            or event.get("revision") != binding.scene.version or event.get("api") != 1
            or event.get("document") != binding.document
            or event.get("plugin") != binding.key or event.get("panel") != binding.id
            or event["event_id"] <= binding.last_event):
        return
    binding.last_event = event["event_id"]
    if (event["control"] == "_panel" and event["event"] == "closed"
            and not getattr(binding.model, "notify_close", False)):
        return
    try:
        if event["control"] == "_retry":
            binding.invalidate()
            return
        if event["event"] == "browse":
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getOpenFileName(win, "Choose file")
            if (not path or binding.scene is not win.viewport.scene
                    or event["revision"] != binding.scene.version):
                return
            event.update(event="commit", value=path)
        binding.model.event(event["control"], event["event"], event["value"])
        binding.error = ""
    except (Exception, SystemExit) as exc:
        binding.error = str(exc)
    binding.invalidate()


def overlay_primitives(view, viewport):
    """Collect document panel overlays for this viewport in logical pixels."""
    result = []
    for binding in getattr(view, "_plugin_bindings", {}).values():
        overlay = getattr(binding.model, "overlay", None)
        if overlay is None:
            continue
        try:
            result.extend(list(overlay(viewport)))
        except (Exception, SystemExit) as exc:
            if binding.error != str(exc):
                binding.error = str(exc)
                binding.invalidate()
    return result


def draw_tool_feedback(painter, tools, layout):
    from views.wgpu_annotations import draw_tool_feedback as draw_builtin_feedback
    from views.wgpu_overlay import paint_layer
    draw_builtin_feedback(painter, tools, layout)
    if hasattr(painter, "draw_primitives"):
        primitives = overlay_primitives(tools.view, tools)
        if primitives:
            painter.save()
            try:
                paint_layer(painter, primitives)
            finally:
                painter.restore()
