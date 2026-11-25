"""Assistant conversations hosted in the native dock, one per document."""
import base64
from tempfile import TemporaryDirectory
from pathlib import Path
from queue import Empty, Queue
from threading import Thread
from uuid import uuid4

from core import ai
from built_core.i18n import tr
from built_core.plugin_panels import field, item, on_document_close, section
from built_core.settings import settings
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
        self.images = TemporaryDirectory(prefix="built-assistant-")
        self.attachment = self.last_photo = None
        self.generation, self.busy = 0, False
        self.inbox = Queue()
        self.timer = app.window._host.timers.timer(self._poll, 100, True)
        on_document_close(app, self.close)

    def _config(self):
        provider = ai.detect_provider(self.key) if self.provider == "auto" else self.provider
        env = ai.env_key(provider) if self.provider != "auto" and not self.key else None
        return provider, self.model or ai.DEFAULT_MODELS[provider], self.key or (env[1] if env else ""), self.url

    def sections(self):
        config = [field("provider", tr("Provider"), "choice", self.provider, options=["auto", *ai.PROVIDERS]),
                  item("button", "key", tr("API key…")),
                  field("model", tr("Model"), "text", self.model),
                  item("button", "models", tr("Models")), item("button", "test", tr("Test connection"))]
        if self.provider in ai.PROVIDER_INFO and ai.PROVIDER_INFO[self.provider][1]:
            config.append(item("log", "key_source", lines=[ai.PROVIDER_INFO[self.provider][1]]))
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
                        item("button", "paste_photo", tr("Paste photo")),
                        *([item("image", "preview", path=self.attachment[2]),
                           item("button", "remove_photo", tr("Remove photo"))] if self.attachment else []),
                        *([item("button", "view_photo", tr("View sent photo"))] if self.last_photo else []),
                        field("shots", tr("Viewport screenshots"), "toggle", self.shots),
                        item("button", "stop" if self.busy else "send", tr("Stop" if self.busy else "Send")))]

    def event(self, control, event, value):
        if control == "stop":
            self._stop()
        elif self.busy:
            return
        elif control == "key":
            from built_render.chrome_forms import Field, Form, show
            def save(answer, values):
                if answer == "ok":
                    self.key = values["credential"].strip()
                    self.store.setValue("ia/api_key", self.key)
                    self.store.setValue("ia/claves/" + self._config()[0], self.key)
                    self.invalidate()
            show(self.app.window, Form(tr("API key"), [Field("credential", "edit", tr("API key"),
                                                               self.key, password=True)], on_done=save))
        elif control == "photo":
            self.attach_photo(value)
        elif control == "paste_photo":
            from built_render.shell_services import pasteboard_read
            data = pasteboard_read("public.png") or pasteboard_read("public.tiff")
            if data:
                self.attach_photo(data)
        elif control == "remove_photo":
            self.clear_photo()
        elif control == "view_photo" and self.last_photo:
            from built_core.desktop_open import open_url
            open_url(self.last_photo)
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
        elif control in ("prompt", "url", "shots"):
            setattr(self, control, value)
            if control == "url":
                self.store.setValue("ia/ollama_url", value)
            elif control == "shots":
                self.store.setValue("ia/capturas", int(value))
        elif control in ("models", "test", "send"):
            provider, model, key, url = self._config()
            if not key and provider != "ollama":
                self._append(self._missing_key_hint(provider, key), "error")
                return
            if control == "models":
                self._start("models", lambda: ai.list_models(provider, key, ollama_url=url))
            elif control == "test":
                self._start("test", lambda: ai.probar_conexion(provider, model, key, ollama_url=url))
            elif self.prompt.strip():
                message = dict(role="user", text=self.prompt.strip())
                if self.attachment:
                    if self.last_photo:
                        Path(self.last_photo).unlink(missing_ok=True)
                    encoded, mime, self.last_photo = self.attachment
                    message.update(image_b64=encoded, image_mime=mime)
                    self.attachment = None
                self._last_prompt = message["text"]
                self._append(self._last_prompt, "user")
                self._convo.append(message)
                self.prompt = self.photo = ""
                self._round = self._failed = 0
                self._nudged = False
                self._next_turn()

    def clear_photo(self):
        if self.attachment:
            Path(self.attachment[2]).unlink(missing_ok=True)
        self.photo, self.attachment = "", None

    def attach_photo(self, source):
        from built_render import SurveyRaster
        self.clear_photo()
        if not source:
            return
        if isinstance(source, bytes):
            data = source
            name = tr("Clipboard")
        else:
            path = Path(source)
            name = path.name
            try:
                with path.open("rb") as stream:
                    data = stream.read(16_000_001)
            except OSError:
                data = b""
        raster = SurveyRaster.decode(data) if len(data) <= 16_000_000 else None
        if raster is None:
            self._append(tr("Could not read the image."), "error")
            return
        scale = min(1, 1280 / max(raster.width, raster.height))
        if scale < 1:
            raster = raster.resized(max(1, round(raster.width * scale)), max(1, round(raster.height * scale)))
        preview = Path(self.images.name) / (uuid4().hex + ".png")
        preview.write_bytes(raster.png())
        self.photo = name
        self.attachment = base64.b64encode(raster.jpeg()).decode(), "image/jpeg", str(preview)

    def close(self, *_):
        self._stop()
        self.images.cleanup()

    def _missing_key_hint(self, provider: str, key: str) -> str | None:
        """The plain-words message for a hosted provider with no key, or
        None when there is nothing to say. Rafael picked «Groq (gratis)»,
        left the key empty and Test connection answered with Groq's raw
        ``HTTP 401 {"error":{"message":"Invalid API Key"…`` (Revisión 3,
        2026-09-20). A missing key is known BEFORE any network round trip,
        and the fix is one sentence, not a JSON blob."""
        if key or provider == "ollama":
            return None
        label, url = ai.PROVIDER_INFO[provider]
        hint = tr(
            "{name} needs an API key — the quota is free, the key is not "
            "optional. Create one at {url} (the link under the key field "
            "opens it) and paste it in the \"API key\" field — it is saved "
            "in your user profile, never in the document.", name=label,
            url=url)
        env_vars = ai.ENV_KEYS.get(provider)
        if env_vars:
            hint += tr(" Or define {var} in the environment and restart "
                       "Built.", var=env_vars[0])
        return hint

    @staticmethod
    def _bad_key_hint(provider: str, err: str) -> str | None:
        """A rejected key (HTTP 401/403, or the providers' own wording),
        translated into what to check."""
        low = (err or "").lower()
        if not ("401" in low or "403" in low or "invalid api key" in low
                or "invalid_api_key" in low or "incorrect api key" in low
                or "invalid x-api-key" in low or "api key not valid" in low
                or "authentication" in low):
            return None
        label, url = ai.PROVIDER_INFO.get(provider, ("", ""))
        prefixes = {"groq": "gsk_", "anthropic": "sk-ant-", "gemini": "AIza",
                    "openrouter": "sk-or-", "opencode": "oc_sk_",
                    "openai": "sk-"}
        pre = prefixes.get(provider)
        tail = (tr(" A {name} key starts with «{prefix}».", name=label,
                   prefix=pre) if pre else "")
        return tr(
            "{name} rejected the key. Check it was pasted whole, with no "
            "spaces, and that it belongs to this provider — or create a new "
            "one at {url}.", name=label, url=url) + tail

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
            self._append(self._bad_key_hint(self._config()[0], result["error"]) or result["error"], "error")
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
        from built_render import archive_picture_png
        render = getattr(self.app.window.view, "render_pixels", None)
        if not render:
            return None
        try:
            png = archive_picture_png(*render(640, 427))
        except (ValueError, RuntimeError, OSError):
            return None
        return base64.b64encode(png).decode() if png else None


create_panel = AssistantPanel
