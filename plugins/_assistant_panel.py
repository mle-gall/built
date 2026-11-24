"""Assistant conversations hosted in the native dock, one per document."""
import base64
import mimetypes
from pathlib import Path
from queue import Empty, Queue
from threading import Thread
from uuid import uuid4

from core import ai
from core.i18n import tr
from core.plugin_panels import field, item, on_document_close, section
from core.settings import settings
from plugins._assistant_prompt import (
    MAX_ROUNDS, MAX_TOKENS, TOKENS_BY_PROVIDER, env_key_note, handle_reply, system_prompt,
)


class AssistantPanel:
    id, title, slot = "assistant", tr("AI Assistant"), "dock"

    def __init__(self, app, invalidate):
        self.app, self.invalidate = app, invalidate
        self._viewport = app.viewport
        self.scene = app.scene
        self._scope = {"__name__": "__ai__"}
        self._session = uuid4().hex
        self._convo, self.lines, self.models = [], [], []
        self.store = settings()
        self.provider = self.store.value("ia/proveedor", "auto") or "auto"
        self.model = self.store.value("ia/modelo", "") or ""
        self.key = self.store.value("ia/api_key", "") or ""
        self.url = self.store.value("ia/ollama_url", "http://localhost:11434")
        self.shots = str(self.store.value("ia/capturas", "1")) != "0"
        self.prompt = self.photo = ""
        self.generation, self.busy = 0, False
        self.inbox = Queue()
        self.timer = app.window._host.timers.timer(self._poll, 100, True)
        on_document_close(app, self._stop)

    def _config(self):
        provider = ai.detect_provider(self.key) if self.provider == "auto" else self.provider
        env = ai.env_key(provider) if self.provider != "auto" and not self.key else None
        return provider, self.model or ai.DEFAULT_MODELS[provider], self.key or (env[1] if env else ""), self.url

    def sections(self):
        config = [field("provider", tr("Provider"), "choice", self.provider, options=["auto", *ai.PROVIDERS]),
                  item("button", "key", tr("API key…")),
                  field("model", tr("Model"), "text", self.model),
                  item("button", "models", tr("Models")), item("button", "test", tr("Test connection"))]
        if self.models:
            config.append(field("choose_model", tr("Available models"), "choice",
                                self.model if self.model in self.models else self.models[0], options=self.models))
        if self._config()[0] == "ollama":
            config.append(field("url", tr("Local server"), "text", self.url))
        note = env_key_note(self.provider, self.key)
        if note:
            config.append(item("log", "env", lines=[note]))
        return [section("configuration", tr("Settings"), *config),
                section("conversation", self.title, item("log", "chat", lines=self.lines),
                        field("prompt", tr("Message"), "text", self.prompt),
                        field("photo", tr("Photo"), "path", self.photo),
                        field("shots", tr("Viewport screenshots"), "toggle", self.shots),
                        item("button", "stop" if self.busy else "send", tr("Stop" if self.busy else "Send")))]

    def event(self, control, event, value):
        if control == "stop":
            self._stop()
        elif self.busy:
            return
        elif control == "key":
            from views.chrome_forms import Field, Form, show
            def save(answer, values):
                if answer == "ok":
                    self.key = values["credential"].strip()
                    self.store.setValue("ia/api_key", self.key)
                    self.store.setValue("ia/claves/" + self._config()[0], self.key)
                    self.invalidate()
            show(self.app.window, Form(tr("API key"), [Field("credential", "edit", tr("API key"),
                                                               self.key, password=True)], on_done=save))
        elif control == "provider":
            self.provider = value
            if value != "auto":
                self.key = self.store.value(f"ia/claves/{value}", "") or ""
                self.model = self.store.value(f"ia/modelos/{value}", "") or ""
            self.models = []
            self.store.setValue("ia/proveedor", value)
            self.store.setValue("ia/api_key", self.key)
            self.store.setValue("ia/modelo", self.model)
        elif control in ("choose_model", "model"):
            self.model = value.strip()
            self.store.setValue("ia/modelo", self.model)
            self.store.setValue("ia/modelos/" + self._config()[0], self.model)
        elif control in ("prompt", "photo", "url", "shots"):
            setattr(self, control, value)
            if control == "url":
                self.store.setValue("ia/ollama_url", value)
            elif control == "shots":
                self.store.setValue("ia/capturas", int(value))
        elif control in ("models", "test", "send"):
            provider, model, key, url = self._config()
            if not key and provider != "ollama":
                self._append(tr("Enter an API key for {name}", name=provider), "error")
                return
            if control == "models":
                self._start("models", lambda: ai.list_models(provider, key, ollama_url=url))
            elif control == "test":
                self._start("test", lambda: ai.probar_conexion(provider, model, key, ollama_url=url))
            elif self.prompt.strip():
                message = dict(role="user", text=self.prompt.strip())
                if self.photo:
                    path = Path(self.photo)
                    mime = mimetypes.guess_type(path.name)[0]
                    if mime not in ("image/png", "image/jpeg", "image/webp") or path.stat().st_size > 16_000_000:
                        raise ValueError(tr("Choose a PNG, JPEG or WebP photo under 16 MB"))
                    message.update(image_b64=base64.b64encode(path.read_bytes()).decode(), image_mime=mime)
                self._last_prompt = message["text"]
                self._append(self._last_prompt, "user")
                self._convo.append(message)
                self.prompt = self.photo = ""
                self._round = self._failed = 0
                self._nudged = False
                self._next_turn()

    def _next_turn(self):
        provider, model, key, url = self._config()
        conversation = ai.compact_messages(ai.slim_messages(self._convo, vision=ai.supports_vision(provider, model)))
        prompt = system_prompt()
        self._start("chat", lambda: ai.chat(provider, model, key, prompt, conversation,
                                            ollama_url=url, session_id=self._session,
                                            max_tokens=TOKENS_BY_PROVIDER.get(provider, MAX_TOKENS)))

    def _start(self, kind, request):
        self.busy = True
        generation = self.generation

        def work():
            try:
                result = dict(ok=True, text=request())
            except Exception as exc:
                result = dict(ok=False, error=str(exc))
            self.inbox.put((generation, kind, result))
        Thread(target=work, daemon=True).start()
        self.timer.start()
        self.invalidate()

    def _poll(self):
        try:
            generation, kind, result = self.inbox.get_nowait()
        except Empty:
            return
        if generation != self.generation:
            return
        self._finish()
        if self.app.scene is not self.scene or self._viewport.scene is not self.scene:
            return
        if kind == "chat":
            handle_reply(self, result, MAX_ROUNDS, self.shots)
        elif not result["ok"]:
            self._append(result["error"], "error")
        elif kind == "models":
            self.models = list(result["text"])
        else:
            ok, message = result["text"]
            self._append(message, "status" if ok else "error")
        self.invalidate()

    def _stop(self, *_):
        self.generation += 1
        self._finish()

    def _finish(self):
        self.busy = False
        self.timer.stop()
        self.invalidate()

    def _append(self, text, role):
        self.lines.extend([f"{role} · {text}"])
        self.lines = self.lines[-200:]

    def _small_model_hint(self, model):
        return tr("Try a larger local model for code generation.")

    def _screenshot_b64(self):
        from ingetrazo_render import archive_picture_png
        render = getattr(self.app.window.view, "render_pixels", None)
        if not render:
            return None
        try:
            png = archive_picture_png(*render(640, 427))
        except (ValueError, RuntimeError, OSError):
            return None
        return base64.b64encode(png).decode() if png else None


create_panel = AssistantPanel
