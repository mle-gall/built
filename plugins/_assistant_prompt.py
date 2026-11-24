"""Prompt, provider settings and conversation limits shared by assistant panels."""
from core import ai, ai_recipes
from core.i18n import tr

MAX_ROUNDS = 12

#: Blocks in a row that raised before control goes back to the user. A
#: small local model (llama3.2 in Rafael's review 6, #461) fed its own
#: TypeError back for all 12 rounds — «toda la tarde» on a busy machine.
#: MAX_ROUNDS stays the cap for multi-step work that keeps succeeding.
MAX_FAILED_ROUNDS = 3

#: Reply budget per turn. 4096 was not enough for chatty coders (gemini
#: 2.5 flash got cut mid-```python and the loop ended with nothing drawn);
#: every provider we speak accepts 8192 output tokens.
MAX_TOKENS = 8192

#: Gemini 2.5 models bill their hidden "thinking" against the SAME
#: max_tokens — flash kept getting cut even at 8192 (seen live modeling a
#: fountain). Both 2.5 flash and pro accept 16384.
TOKENS_BY_PROVIDER = {"gemini": 16384}

#: The language a reply falls back to when the user's own cannot be told
#: (a bare «ok»), by interface language.
_LANGUAGE_NAMES = {
    "es": "español", "en": "English", "de": "Deutsch", "fr": "français",
    "id": "Bahasa Indonesia", "it": "italiano", "pl": "polski",
    "pt-BR": "português do Brasil", "ru": "русский", "zh-CN": "简体中文",
    "zh-TW": "繁體中文", "vi": "Tiếng Việt", "lo": "ລາວ", "th": "ไทย", "ja": "日本語", "ko": "한국어",
    "ar": "العربية", "bg": "български",
}


def system_prompt(lang: str | None = None) -> str:
    """The system prompt with the reply language: the one the user writes
    in, falling back to the interface's. The prompt said «Conversas en
    español», so it answered an English user in Spanish even after being
    told not to (#790)."""
    if lang is None:
        from core.i18n import current_language
        lang = current_language()
    name = _LANGUAGE_NAMES.get(lang, "English")
    return SYSTEM_PROMPT + (
        "\n\nIDIOMA: responde SIEMPRE en el idioma en que escribe el "
        f"usuario; si no se puede saber, en {name}. Vale también después "
        "de recibir resultados o capturas: este prompt y los resultados "
        "están en español, pero la conversación no.")


#: The assistant's own voice and its one-block-per-reply contract; the
#: modelling reference itself is shared with the MCP door (core.ai_recipes),
#: so a new helper is taught once and both doors learn it.
SYSTEM_PROMPT = "\n".join((
    "Eres el asistente de modelado de Built. " + ai_recipes.UNITS
    + " Conversas breve y claro.",
    """
Para ACTUAR sobre el modelo incluye EXACTAMENTE UN bloque ```python por \
respuesta. Tras cada bloque recibirás su resultado (stdout/errores y, si \
está disponible, una captura del viewport) — revísalo e itera. Cuando el \
pedido esté terminado, responde SIN bloque de código con un resumen corto. \
SIN bloque no se ejecuta nada: nunca describas como hecho lo que no has \
ejecutado.""".strip(),
    ai_recipes.SCOPE,
    ai_recipes.RECIPES,
    ai_recipes.HOW_IT_RUNS.format(unit="bloque"),
    """El código de tus recetas viejas se resume como "[receta ya \
ejecutada]"; su efecto sigue en el modelo.

Si el usuario adjunta una FOTO de un objeto (una fuente, un mueble, una \
fachada): identifica sus partes y proporciones y recréalo por partes, cada \
una como grupo con nombre. Una foto NO trae medidas: usa las que el usuario \
dé y declara como supuesto toda dimensión que estimes de la imagen. Para \
piezas torneadas (platos, columnas, jarrones) usa revolve(). Compara tus \
capturas contra la foto e itera hasta que la silueta calce.""",
))


_BUILD_WORDS = ("dibuj", "crea", "haz", "hac", "modela", "constru", "añad",
                "agreg", "pon", "gener", "diseñ", "levant", "arma", "traza",
                "draw", "build", "make", "create", "add")


def _asks_to_build(prompt: str) -> bool:
    """Whether the user's message asks for something to be modelled (so a
    reply without code is a model that did not do its job)."""
    low = (prompt or "").lower()
    return any(w in low for w in _BUILD_WORDS)


def key_placeholder(chosen, environ=None) -> str:
    """What an empty key field means for the provider combo's data: the
    local server only on Auto and Local — with Anthropic picked, «empty =
    local AI» was wrong (#998 review)."""
    if not chosen or chosen in ("auto", "ollama"):
        return tr("empty = local AI (Ollama, LM Studio)")
    env = ai.env_key(chosen, environ)
    if env:
        return tr("empty = {var} from the environment", var=env[0])
    return tr("paste your {name} key", name=ai.PROVIDER_INFO[chosen][0])


def env_key_note(chosen, key: str, environ=None) -> str:
    """The line under an EMPTY key field when the environment has a key
    (#998), "" otherwise. ``chosen`` is the provider combo's data: a
    provider by name uses its variable; on Auto the field stays the local
    server, so the note only says which provider to pick. Shared with the
    Preferences dialog."""
    if key:
        return ""
    if chosen and chosen != "auto":
        env = ai.env_key(chosen, environ)
        return (tr("Using {var} from the environment", var=env[0])
                if env else "")
    for prov in ai.PROVIDERS:
        env = ai.env_key(prov, environ)
        if env:
            return tr("{var} is set in the environment — choose {name} as "
                      "the provider to use it", var=env[0],
                      name=ai.PROVIDER_INFO[prov][0])
    return ""


def handle_reply(self, msg, round_limit, screenshots):
    """Apply one provider reply to the document on the application thread."""
    if not msg.get("ok"):
        self._append(tr("Error: {err}", err=msg.get("error")), "err")
        self._finish()
        return
    self._local_warm = True
    text = ai.strip_thoughts(msg["text"])
    self._convo.append({"role": "assistant", "text": text})
    self._append(text, "ai")
    code = ai.extract_code(text)
    if code is None and ai.truncated_code(text):
        # Cut by max_tokens mid-recipe: half a block must neither run
        # nor end the loop silently — ask for a smaller, complete one.
        if self._round < round_limit:
            self._round += 1
            self._append(tr("The reply was cut off mid-code — asking "
                            "for a shorter, complete block."), "muted")
            self._convo.append({"role": "user", "text":
                "Tu respuesta se cortó a mitad del bloque ```python "
                "(límite de tokens). NO continúes donde quedaste: "
                "reenvía UN bloque completo y más corto, avanzando "
                "solo la primera parte del trabajo; el resto irá en "
                "turnos siguientes."})
            self._next_turn()
            return
        self._append(tr('The reply was cut off and the step limit is '
                        'used up — type "continue" to keep going.'),
                     "err")
        self._finish()
        return
    if code is not None and self._round >= round_limit:
        # A recipe arrived but the budget for ONE request is spent:
        # never swallow it silently — the convo survives, so any new
        # message (e.g. "continúa") picks up exactly here.
        self._append(tr('Step limit reached for one request '
                        '({n}) — type "continue" to keep going.',
                        n=round_limit), "muted")
        self._finish()
        return
    if code is None:
        if self._round == 0 and not self._nudged and _asks_to_build(
                self._last_prompt) and "?" not in text[-80:]:
            # A model that narrates a build it never sent (Groq's
            # compound answered «se añadió una cumbrera…» with no
            # block, Marco 2026-09-15): one push, then let it be.
            self._nudged = True
            self._append(tr("No code came back — asking for the recipe."),
                         "muted")
            self._convo.append({"role": "user", "text":
                "No incluiste ningún bloque ```python: NADA se ejecutó "
                "y el modelo no cambió. Escribe ahora la receta completa "
                "en UN bloque ```python (sin describirla antes)."})
            self._next_turn()
            return
        self._finish()
        return
    self._round += 1
    result = ai.run_transactional(self._viewport, code, self._scope)
    summary = []
    if result["stdout"]:
        out = result["stdout"].rstrip()
        if len(out) > 1500:      # a print-happy recipe rides EVERY turn
            out = out[:700] + "\n… (recortado) …\n" + out[-700:]
        summary.append(out)
    self._failed = self._failed + 1 if result["error"] else 0
    if result["error"]:
        summary.append("ERROR (todo revertido): " + str(result["error"]))
        if result["stderr"]:
            summary.append(result["stderr"].rstrip()[-800:])
    summary.append(f"(cambió el modelo: {result['changed']})")
    feedback = "Resultado de la ejecución:\n" + "\n".join(summary)
    self._append(feedback, "muted")
    provider, model = self._config()[:2]
    shot = None
    # Only shoot when the model CHANGED: after an error or an
    # inspect-only block the previous screenshot is still accurate
    # (slim_messages keeps the latest one in the convo).
    if (screenshots and result["changed"]
            and ai.supports_vision(provider, model)):
        shot = self._screenshot_b64()
    self._convo.append({"role": "user", "text": feedback,
                        **({"image_png_b64": shot} if shot else {})})
    if self._failed >= MAX_FAILED_ROUNDS:
        # The error is already in the convo: «continúa» (or a clearer
        # request) picks up from here with the model knowing why.
        self._append(tr('{n} attempts in a row failed — stopping here. '
                        'Rephrase the request or type "continue" to '
                        'let it try again.', n=self._failed), "err")
        if ai.small_local_model(provider, model):
            self._append(self._small_model_hint(model), "muted")
        self._finish()
        return
    self._next_turn()
