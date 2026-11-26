# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Asistente IA: provider layer (IngePresupuestos convention) and the
in-app agent loop with a scripted fake model."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import json

import pytest

from core import ai


@pytest.fixture
def panel(native_host):
    from plugins.ai_assistant import create_panel
    from views.extension_api import ExtensionApp
    return create_panel(ExtensionApp(native_host.window, "ai_assistant"), native_host.viewport.update)


def wait_reply(panel):
    import time
    deadline = time.monotonic() + 5
    while panel.busy and time.monotonic() < deadline:
        panel._poll()
        time.sleep(.002)
    assert not panel.busy


def test_detect_provider_by_key_prefix():
    assert ai.detect_provider("gsk_xxx") == "groq"
    assert ai.detect_provider("sk-ant-xxx") == "anthropic"
    assert ai.detect_provider("AIzaXXX") == "gemini"
    assert ai.detect_provider("sk-or-xxx") == "openrouter"
    assert ai.detect_provider("oc_sk_xxx") == "opencode"
    assert ai.detect_provider("sk-xxx") == "openai"
    assert ai.detect_provider("") == "ollama"


def test_build_request_shapes():
    msgs = [{"role": "user", "text": "hola"},
            {"role": "assistant", "text": "```python\nprint(1)\n```"},
            {"role": "user", "text": "resultado", "image_png_b64": "QUJD"}]

    url, headers, payload = ai.build_request(
        "anthropic", "claude-sonnet-5", "sk-ant-k", "SYS", msgs)
    assert "api.anthropic.com/v1/messages" in url
    assert headers["x-api-key"] == "sk-ant-k"
    body = json.loads(payload)
    assert body["system"] == "SYS"
    blocks = body["messages"][2]["content"]
    assert blocks[0]["type"] == "image"
    assert blocks[0]["source"]["data"] == "QUJD"

    url, headers, payload = ai.build_request(
        "groq", "llama-3.3-70b-versatile", "gsk_k", "SYS", msgs)
    assert "api.groq.com/openai/v1/chat/completions" in url
    assert headers["Authorization"] == "Bearer gsk_k"
    body = json.loads(payload)
    assert body["messages"][0] == {"role": "system", "content": "SYS"}
    img = body["messages"][3]["content"][0]
    assert img["image_url"]["url"].startswith("data:image/png;base64,")

    url, _h, _p = ai.build_request("ollama", "llama3.2", "", "SYS",
                                   [{"role": "user", "text": "x"}],
                                   ollama_url="http://localhost:11434")
    assert url == "http://localhost:11434/v1/chat/completions"


def test_build_request_generic_image_mime():
    # User photos ride as JPEG via image_b64+image_mime; the legacy
    # image_png_b64 screenshot shorthand keeps working (previous test).
    msgs = [{"role": "user", "text": "recrea esto",
             "image_b64": "REVG", "image_mime": "image/jpeg"}]

    _u, _h, payload = ai.build_request(
        "anthropic", "claude-sonnet-5", "sk-ant-k", "SYS", msgs)
    block = json.loads(payload)["messages"][0]["content"][0]
    assert block["type"] == "image"
    assert block["source"]["media_type"] == "image/jpeg"
    assert block["source"]["data"] == "REVG"

    _u, _h, payload = ai.build_request(
        "openai", "gpt-4o", "sk-k", "SYS", msgs)
    img = json.loads(payload)["messages"][1]["content"][0]
    assert img["image_url"]["url"].startswith("data:image/jpeg;base64,REVG")


def test_parse_reply_and_extract_code():
    anth = json.dumps({"content": [{"type": "text", "text": "hola "},
                                   {"type": "text", "text": "mundo"}]})
    assert ai.parse_reply("anthropic", anth.encode()) == "hola mundo"
    oai = json.dumps({"choices": [{"message": {"content": "ok"}}]})
    assert ai.parse_reply("openai", oai.encode()) == "ok"

    text = "Voy a dibujar:\n```python\nmesh.add_edge(a, b)\n```\nlisto"
    assert ai.extract_code(text) == "mesh.add_edge(a, b)"
    assert ai.extract_code("sin código") is None


def test_compact_messages_stubs_old_recipes_keeps_recent():
    # Past recipes are dead weight (their effect is in the document): all
    # but the last 2 collapse to a stub, prose and feedback stay whole.
    def turn(i):
        return [{"role": "assistant",
                 "text": f"Paso {i}:\n```python\nprint({i})\n```\nsigo"},
                {"role": "user", "text": f"Resultado {i}"}]

    convo = [{"role": "user", "text": "modela la fuente"}]
    for i in range(4):
        convo += turn(i)
    convo.append({"role": "assistant", "text": "Listo, sin código."})

    slim = ai.compact_messages(convo)
    stubbed = [m for m in slim if ai.CODE_STUB in m["text"]]
    assert len(stubbed) == 2                       # turns 0 and 1
    assert "Paso 0:" in stubbed[0]["text"]         # prose kept
    assert "sigo" in stubbed[0]["text"]
    assert "print(0)" not in stubbed[0]["text"]    # code gone
    assert "print(2)" in slim[5]["text"]           # last two stay whole
    assert "print(3)" in slim[7]["text"]
    assert slim[2]["text"] == "Resultado 0"        # feedback untouched
    assert slim[-1]["text"] == "Listo, sin código."
    assert "print(0)" in convo[1]["text"]          # stored convo untouched


def test_strip_thoughts_removes_leaked_monologue():
    # Gemma leaks its chain of thought into the visible text (seen live);
    # resending it every turn is pure token burn.
    assert ai.strip_thoughts("<thought>hmm</thought>Listo.") == "Listo."
    assert ai.strip_thoughts("plan…</thought>La fuente está.") \
        == "La fuente está."                       # opener lost upstream
    assert ai.strip_thoughts("<thinking>a\nb") == ""   # unclosed: all gone
    assert ai.strip_thoughts("sin nada") == "sin nada"
    code = "ok\n```python\nprint(1)\n```"
    assert ai.strip_thoughts(code) == code


def test_recipe_helpers_build_correct_solids(native_host):
    # revolve/extrude are the one-line builders recipes should reach for:
    # a closed, outward-oriented, soft-edged group per call.
    win = native_host.window
    vp = win.viewport
    scope: dict = {"__name__": "__ai__"}
    r = ai.run_transactional(vp, (
        "g1 = revolve([(0.5, 0.0), (0.5, 1.0)], name='Cil',"
        " color=(1, 0, 0, 1))\n"
        "g2 = revolve([(0.6, 0.0), (0.7, 0.2), (0.6, 0.4), (0.4, 0.4),"
        " (0.4, 0.0)], closed=True, name='Aro')\n"
        "g3 = revolve([(0.4, 0.0), (0.5, 0.3), (0.5, 0.5)],"
        " scallop=(0.1, 8), name='Festón')\n"
        "g4 = extrude([(0, 0), (1, 0), (1, 1), (0, 1)], 0.0, 0.5,"
        " name='Caja', color=(0, 1, 0, 1))\n"
        "print(len(g1.mesh.faces), len(g4.mesh.faces))"), scope)
    assert r["error"] is None and r["changed"]
    names = [g.name for g in vp.scene.groups[-4:]]
    assert names == ["Cil", "Aro", "Festón", "Caja"]
    cil, aro, feston, caja = vp.scene.groups[-4:]
    assert len(cil.mesh.faces) == 32 + 2       # sides + two caps
    assert len(caja.mesh.faces) == 6
    assert all(f.attrs.get("color") == (1, 0, 0, 1)
               for f in cil.mesh.faces)
    # Every solid is watertight and the curved sides came out soft.
    from built_core.orient import is_closed
    for g in (cil, aro, feston, caja):
        assert is_closed(g.mesh)
    assert any(e.soft for e in cil.mesh.edges)
    # The scallop carves INTO the stone: nominal radius is the max.
    import math as m
    rmax = max(m.hypot(v.position.x(), v.position.y())
               for v in feston.mesh.vertices)
    assert abs(rmax - 0.5) < 1e-5
    # ONE undo step for the whole recipe.
    assert vp.history.undo_stack


def test_failed_recipe_leaves_no_orphan_groups(native_host):
    # A recipe that dies AFTER extrude/revolve appended groups must roll
    # back whole: History.execute only restored the mesh, so the groups
    # survived with no undo entry (45 orphan pieces in a live session).
    win = native_host.window
    vp = win.viewport
    scene = vp.scene
    n_groups = len(scene.groups)
    n_layers = len(scene.layers)
    depth = len(vp.history.undo_stack)
    scope: dict = {"__name__": "__ai__"}
    r = ai.run_transactional(vp, (
        "extrude([(0, 0), (1, 0), (1, 1), (0, 1)], 0.0, 0.5, name='Caja')\n"
        "from built_core.layers import Layer\n"
        "scene.layers.append(Layer('Huérfana'))\n"
        "raise ValueError('boom')"), scope)
    assert r["error"] is not None and not r["changed"]
    assert "ValueError: boom" in r["stderr"]
    assert len(scene.groups) == n_groups
    assert len(scene.layers) == n_layers
    assert len(vp.history.undo_stack) == depth


def test_failed_recipe_restores_deleted_groups_layers_and_history(native_host):

    win = native_host.window
    vp, scope = win.viewport, {"__name__": "__ai__"}
    for name in ("First", "Second", "Redo"):
        result = ai.run_transactional(
            vp, f"box((0, 0, 0), (1, 1, 1), name={name!r})", scope)
        assert result["error"] is None
    assert vp.history.undo()
    scene = vp.scene
    groups, layers = scene.groups, scene.layers
    before_groups, before_layers = list(groups), list(layers)
    undo, redo = list(vp.history.undo_stack), list(vp.history.redo_stack)

    result = ai.run_transactional(vp, (
        "groups.pop(0)\n"
        "groups.reverse()\n"
        "box((0, 0, 0), (2, 2, 2), name='Discarded')\n"
        "layers.clear()\n"
        "raise ValueError('rollback deletion')"), scope)

    assert result["error"] is not None and not result["changed"]
    assert scene.groups is groups and scope["groups"] is groups
    assert scene.layers is layers and scope["layers"] is layers
    assert groups == before_groups
    assert layers == before_layers
    assert vp.history.undo_stack == undo
    assert vp.history.redo_stack == redo
    assert vp.history.redo()
    assert [g.name for g in scene.groups][-3:] == ["First", "Second", "Redo"]


def test_supports_vision_by_provider_or_model():
    assert ai.supports_vision("anthropic", "claude-sonnet-5")
    assert ai.supports_vision("gemini", "gemini-2.5-flash")
    # Groq's DEFAULT model is text-only, but its free Llama 4 sees images.
    assert not ai.supports_vision("groq", "openai/gpt-oss-120b")
    assert ai.supports_vision(
        "groq", "meta-llama/llama-4-scout-17b-16e-instruct")
    assert ai.supports_vision("ollama", "llava:13b")
    assert ai.supports_vision("ollama", "qwen2.5-vl-7b")
    assert not ai.supports_vision("ollama", "llama3.2")
    assert not ai.supports_vision("groq", "llama-3.3-70b-versatile")
    # OpenCode Go, probed live 2026-10-20: flash takes images, plain
    # glm-5.3 does not — the tags must stay that specific.
    assert ai.supports_vision("opencode", "glm-5.3-flash")
    assert ai.supports_vision("opencode", "kimi-k3")
    assert not ai.supports_vision("opencode", "glm-5.3")
    assert not ai.supports_vision("opencode", "kimi-k2.6")


def test_slim_messages_keeps_photo_and_latest_screenshot_only():
    convo = [
        {"role": "user", "text": "recrea esto",
         "image_b64": "FOTO", "image_mime": "image/jpeg"},
        {"role": "assistant", "text": "```python\n...\n```"},
        {"role": "user", "text": "resultado 1", "image_png_b64": "SHOT1"},
        {"role": "assistant", "text": "```python\n...\n```"},
        {"role": "user", "text": "resultado 2", "image_png_b64": "SHOT2"},
    ]
    slim = ai.slim_messages(convo)
    assert slim[0]["image_b64"] == "FOTO"          # the reference stays
    assert "image_png_b64" not in slim[2]          # stale screenshot gone
    assert slim[4]["image_png_b64"] == "SHOT2"     # the latest stays
    assert slim[2]["text"] == "resultado 1"        # text untouched
    assert convo[2]["image_png_b64"] == "SHOT1"    # stored convo untouched

    # A text-only model gets NO image content at all (Groq rejects content
    # arrays with HTTP 400, and the poisoned convo made every retry fail).
    blind = ai.slim_messages(convo, vision=False)
    assert all(not any(k.startswith("image") for k in m) for m in blind)
    assert [m["text"] for m in blind] == [m["text"] for m in convo]
    assert convo[0]["image_b64"] == "FOTO"         # still stored


def test_truncated_code_detects_a_cut_reply():
    # A reply chopped by max_tokens opens the fence and never closes it
    # (seen live with gemini-2.5-flash) — that is NOT "no code".
    cut = "Empiezo:\n```python\nimport math\nm = Mesh()"
    assert ai.extract_code(cut) is None
    assert ai.truncated_code(cut)
    assert not ai.truncated_code("sin código")
    assert not ai.truncated_code("```python\nprint(1)\n```\nlisto")

    # The assistant asks for MORE room than the layer's default.
    _u, _h, payload = ai.build_request(
        "gemini", "gemini-2.5-flash", "AIzaK", "SYS",
        [{"role": "user", "text": "x"}], max_tokens=8192)
    assert json.loads(payload)["max_tokens"] == 8192


# ---- In-app agent loop ------------------------------------------------------

def test_assistant_loop_executes_recipes_transactionally(native_host, monkeypatch):
    import time
    from views.extension_api import ExtensionApp

    replies = iter([
        "Dibujo la arista:\n```python\n"
        "mesh.add_edge(QVector3D(0,0,0), QVector3D(3,0,0))\n"
        "print('arista lista')\n```",
        "Listo: dibujé la arista de 3 m.",
    ])
    seen: list = []

    def fake_chat(provider, model, key, system, messages, **kw):
        seen.append([dict(m) for m in messages])
        return next(replies)

    monkeypatch.setattr(ai, "chat", fake_chat)

    win = native_host.window
    vp = win.viewport
    panel = win._panel_factories["ai_assistant"](ExtensionApp(win, "ai_assistant"), vp.update)
    panel.event("provider", "commit", "ollama")
    panel.event("shots", "commit", False)
    edges0 = len(vp.scene.mesh.edges)
    depth0 = len(vp.history.undo_stack)

    panel.event("prompt", "commit", "dibuja una arista de 3 m")
    panel.event("send", "invoke", None)
    deadline = time.monotonic() + 5
    while panel.busy and time.monotonic() < deadline:
        native_host.timers.run_due()
        time.sleep(.01)
    assert not panel.busy

    assert len(vp.scene.mesh.edges) == edges0 + 1
    assert len(vp.history.undo_stack) == depth0 + 1   # ONE undo step
    # The model got the execution feedback on the second turn.
    assert any("arista lista" in m["text"]
               for m in seen[1] if m["role"] == "user")
    # The transcript keeps user → assistant → feedback → assistant.
    roles = [m["role"] for m in panel._convo]
    assert roles == ["user", "assistant", "user", "assistant"]


def test_photo_attaches_to_next_message_only(panel, monkeypatch, tmp_path):
    # The attached photo rides the NEXT user message as scaled JPEG and is
    # cleared afterwards — execution feedback turns must not re-send it.
    from built_render import SurveyRaster
    import base64
    photo = tmp_path / "fuente.jpg"
    photo.write_bytes(SurveyRaster(3000, 1500, bytes([128,128,128,255]) * (3000 * 1500)).jpeg())

    replies = iter([
        "Empiezo:\n```python\nprint('viendo la foto')\n```",
        "Listo.",
    ])
    seen: list = []

    def fake_chat(provider, model, key, system, messages, **kw):
        seen.append([dict(m) for m in messages])
        return next(replies)

    monkeypatch.setattr(ai, "chat", fake_chat)

    panel.event("provider", "commit", "auto")
    panel.key = "sk-ant-test"
    panel.shots = False
    assert panel.attachment is None
    panel.event("photo", "commit", str(photo))
    b64, mime, preview = panel.attachment
    assert mime == "image/jpeg" and panel.photo == "fuente.jpg"
    sent = SurveyRaster.decode(base64.b64decode(b64))
    assert (sent.width, sent.height) == (1280, 640)

    panel.event("prompt", "commit", "recrea la fuente; taza de 4 m")
    panel.event("send", "invoke", None)
    wait_reply(panel)

    first_user = seen[0][0]
    assert first_user["image_b64"] == b64
    assert first_user["image_mime"] == "image/jpeg"
    assert panel.attachment is None and not panel.photo
    # The feedback turn carries no photo (and no screenshot: shots off).
    feedback = seen[1][2]
    assert feedback["role"] == "user"
    assert "image_b64" not in feedback
    assert "image_png_b64" not in feedback


def test_cut_reply_gets_a_retry_not_a_silent_stop(panel, native_host, monkeypatch):
    # Live failure (gemini-2.5-flash): reply cut mid-```python → the loop
    # used to end silently with nothing drawn. Now it tells the model and
    # asks for a shorter complete block, and the retry still executes.
    from plugins.ai_assistant import TOKENS_BY_PROVIDER

    replies = iter([
        "Empiezo con el pedestal:\n```python\nimport math\nm = Mesh()",
        "Más corto:\n```python\n"
        "mesh.add_edge(QVector3D(0,0,0), QVector3D(1,0,0))\n```",
        "Listo.",
    ])
    seen: list = []
    budgets: list = []

    def fake_chat(provider, model, key, system, messages, **kw):
        seen.append([dict(m) for m in messages])
        budgets.append(kw.get("max_tokens"))
        return next(replies)

    monkeypatch.setattr(ai, "chat", fake_chat)

    win = native_host.window
    vp = win.viewport
    panel.event("provider", "commit", "auto")
    panel.key = "AIzaTest"
    panel.shots = False
    edges0 = len(vp.scene.mesh.edges)

    panel.event("prompt", "commit", "modela la fuente de la foto")
    panel.event("send", "invoke", None)
    wait_reply(panel)

    # The retry request went out, the shorter block ran, the loop ended
    # on the closing summary — and every turn asked for Gemini's budget
    # (its hidden thinking bills against the same max_tokens).
    assert any("se cortó a mitad del bloque" in m["text"]
               for m in seen[1] if m["role"] == "user")
    assert len(vp.scene.mesh.edges) == edges0 + 1
    assert budgets == [TOKENS_BY_PROVIDER["gemini"]] * 3


def test_step_limit_announces_instead_of_silent_stop(panel, native_host, monkeypatch):
    # Second live failure mode: the model was still sending recipes when
    # MAX_ROUNDS ran out and the loop just went quiet. Now it says so and
    # points at "continue" (the convo survives, so continuing works).
    import plugins._assistant_panel as pa

    monkeypatch.setattr(pa, "MAX_ROUNDS", 1)
    replies = iter([
        "Paso 1:\n```python\nprint('a')\n```",
        "Paso 2:\n```python\n"
        "mesh.add_edge(QVector3D(0,0,0), QVector3D(1,0,0))\n```",
    ])
    monkeypatch.setattr(ai, "chat",
                        lambda *a, **k: next(replies))

    win = native_host.window
    vp = win.viewport
    panel.event("provider", "commit", "auto")
    panel.key = "AIzaTest"
    panel.shots = False
    edges0 = len(vp.scene.mesh.edges)

    panel.event("prompt", "commit", "haz dos pasos")
    panel.event("send", "invoke", None)
    wait_reply(panel)

    # The second recipe was NOT executed (limit hit) and the chat says
    # why instead of ending silently.
    assert len(vp.scene.mesh.edges) == edges0
    assert "Step limit reached" in "\n".join(panel.lines)


def test_consecutive_failures_hand_control_back(panel, monkeypatch):
    # #461: llama3.2 fed its own TypeError back for all 12 rounds. Three
    # failed blocks in a row stop the loop and say so; a success between
    # failures resets the count.
    import plugins._assistant_panel as pa

    sent = []
    replies = iter(["```python\n1/0\n```", "```python\nprint('ok')\n```"]
                   + ["```python\n'a' | 'b'\n```"] * 10)

    def fake_chat(*a, **k):
        sent.append(1)
        return next(replies)

    monkeypatch.setattr(ai, "chat", fake_chat)
    panel.event("provider", "commit", "auto")
    panel.key = "AIzaTest"
    panel.shots = False
    panel.event("prompt", "commit", "dibuja un cubo")
    panel.event("send", "invoke", None)
    wait_reply(panel)
    # fail, ok, fail ×3 → stop: five blocks, not twelve.
    assert len(sent) == 5
    assert "3 attempts in a row failed" in "\n".join(panel.lines)


def test_small_local_models_are_flagged():
    assert ai.small_local_model("ollama", "llama3.2")
    assert ai.small_local_model("ollama", "qwen2.5-coder:1.5b")
    assert not ai.small_local_model("ollama", ai.DEFAULT_MODELS["ollama"])
    assert not ai.small_local_model("ollama", "llama3.1:8b")
    assert not ai.small_local_model("groq", "llama3.2")


def test_box_helper_builds_a_closed_box(native_host):
    from built_core.orient import is_closed
    win = native_host.window
    vp = win.viewport
    r = ai.run_transactional(
        vp, "g = box((1, 2, 0), (6, 6, 6), name='Cubo')", {"__name__": "__ai__"})
    assert r["error"] is None and r["changed"]
    g = vp.scene.groups[-1]
    assert g.name == "Cubo" and len(g.mesh.faces) == 6
    assert is_closed(g.mesh)
    xs = sorted({round(v.position.x(), 6) for v in g.mesh.vertices})
    zs = sorted({round(v.position.z(), 6) for v in g.mesh.vertices})
    assert xs == [1.0, 7.0] and zs == [0.0, 6.0]
    # A model that appends what the helpers already added (live with
    # qwen3.5): no duplicate, and house()'s LIST is flattened, not
    # left in scene.groups to crash the next paint.
    n0 = len(vp.scene.groups)
    r = ai.run_transactional(vp, (
        "b = box((20, 0, 0), (1, 1, 1))\ngroups.append(b)\n"
        "gs = house(6, 4, origin=(30, 0))\ngroups.append(gs)"),
        {"__name__": "__ai__"})
    assert r["error"] is None
    from built_core.group import Group
    added = vp.scene.groups[n0:]
    assert all(isinstance(x, Group) for x in vp.scene.groups)
    # box + house walls, floor, roof (no openings → no carpentry).
    assert len(added) == len({id(x) for x in added}) == 1 + 3
    # The recipe book teaches it to both doors.
    from core import ai_recipes
    assert "box((x,y,z), (dx,dy,dz)" in ai_recipes.RECIPES


def test_encode_photo_rejects_garbage(panel, tmp_path):
    bad = tmp_path / "no-es-imagen.jpg"
    bad.write_bytes(b"not an image at all")
    panel.event("photo", "commit", str(bad))
    assert panel.attachment is None
    assert "Could not read the image." in str(panel.sections())


def test_send_becomes_stop_and_a_stopped_turn_is_discarded(panel, monkeypatch):
    import threading
    gate = threading.Event()
    monkeypatch.setattr(ai, "chat", lambda *a, **kw: gate.wait(5) and "late answer")
    panel.event("provider", "commit", "ollama")
    panel.event("prompt", "commit", "hello")
    panel.event("send", "invoke", None)
    try:
        assert panel.busy and "stop" in str(panel.sections())
        generation = panel.generation
        panel.event("stop", "invoke", None)
        assert not panel.busy
        panel.inbox.put((generation, "chat", dict(ok=True, text="late answer")))
        panel._poll()
        assert len(panel._convo) == 1
    finally:
        gate.set()


def test_pasted_image_attaches_with_thumbnail(panel, monkeypatch):
    from built_render import SurveyRaster
    from built_render import shell_services as native
    data = SurveyRaster(10, 10, bytes([255, 0, 0, 255]) * 100).png()
    monkeypatch.setattr(native, "pasteboard_read", lambda kind: data)
    panel.event("paste_photo", "invoke", None)
    assert panel.attachment and "preview" in str(panel.sections())
    panel.event("remove_photo", "invoke", None)
    assert panel.attachment is None and "preview" not in str(panel.sections())


def test_sent_photo_opens_its_preview(panel, tmp_path, monkeypatch):
    from tests.plugin_images import write_png
    from built_core import desktop_open
    photo = tmp_path / "photo.png"
    write_png(photo, 64, 32)
    panel.event("photo", "commit", str(photo))
    preview = panel.attachment[2]
    monkeypatch.setattr(panel, "_next_turn", lambda: None)
    panel.event("provider", "commit", "ollama")
    panel.event("prompt", "commit", "describe this")
    panel.event("send", "invoke", None)
    opened = []
    monkeypatch.setattr(desktop_open, "open_url", opened.append)
    panel.event("view_photo", "invoke", None)
    assert opened == [preview]


def test_user_agent_defeats_cloudflare_and_headers_carry_it():
    # Groq sits behind Cloudflare, which rejects urllib's default agent with
    # HTTP 403 error 1010 (the user's first real key hit it).
    for provider in ("groq", "anthropic", "openai"):
        _u, headers, _p = ai.build_request(
            provider, "m", "k", "SYS", [{"role": "user", "text": "x"}])
        assert headers["User-Agent"].startswith("Built/")


def test_provider_selector_overrides_autodetect(panel, monkeypatch):
    panel.key = "sk-ant-algo"
    panel.provider = "auto"
    assert panel._config()[0] == "anthropic"
    panel.event("provider", "commit", "groq")
    assert panel._config()[0] == "groq"
    assert "console.groq.com" in str(panel.sections())
    panel.key = "gsk_test"
    monkeypatch.setattr(ai, "probar_conexion", lambda *a, **kw: (True, "Connection OK"))
    panel.event("test", "invoke", None)
    wait_reply(panel)
    assert "Connection OK" in str(panel.lines)
    monkeypatch.setattr(ai, "list_models", lambda *a, **kw: ["model-one", "model-two"])
    panel.event("models", "invoke", None)
    wait_reply(panel)
    assert panel.models == ["model-one", "model-two"]


def test_list_models_filters_and_strips(monkeypatch):
    # Groq retired llama-3.3-70b-versatile (2026-06-17, free tiers) — the
    # picker offers the LIVE list, minus non-chat models, "models/" bare.
    calls = {}

    def fake_urlopen(url, headers, payload=None, timeout=180.0):
        calls["url"], calls["headers"] = url, headers
        return json.dumps({"data": [
            {"id": "models/gemini-2.5-flash"},
            {"id": "openai/gpt-oss-120b"},
            {"id": "whisper-large-v3"},
            {"id": "playai-tts"},
            {"id": "meta-llama/llama-guard-4-12b"},
            {"id": "text-embedding-004"},
        ]}).encode()

    monkeypatch.setattr(ai, "_urlopen", fake_urlopen)
    models = ai.list_models("groq", "gsk_k")
    assert models == ["gemini-2.5-flash", "openai/gpt-oss-120b"]
    assert calls["url"] == "https://api.groq.com/openai/v1/models"
    assert calls["headers"]["Authorization"] == "Bearer gsk_k"
    assert calls["headers"]["User-Agent"].startswith("Built/")

    ai.list_models("anthropic", "sk-ant-k")
    assert "api.anthropic.com/v1/models" in calls["url"]
    assert calls["headers"]["x-api-key"] == "sk-ant-k"


def test_panel_uses_the_renderers_chat_and_prompt_controls(panel):
    fields = {i["id"]: i for section in panel.sections() for i in section["items"]}
    assert fields["chat"]["kind"] == "log"
    assert fields["prompt"]["type"] == "text"


def test_chat_colours_are_owned_by_the_native_theme(panel):
    panel._append("response", "assistant")
    chat = next(i for section in panel.sections() for i in section["items"] if i["id"] == "chat")
    assert chat["lines"] == ["assistant · response"]
    assert "color" not in chat


def test_each_provider_remembers_its_own_key_and_model(panel, monkeypatch):
    from built_render import chrome_forms
    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda _, form: forms.append(form))
    for provider, key in [("groq", "gsk_AAA"), ("gemini", "AIzaBBB")]:
        panel.event("provider", "commit", provider)
        panel.event("key", "invoke", None)
        forms[-1].receive("credential", key)
        forms[-1].press("ok")
        panel.event("model", "commit", provider + "-model")
    for provider, key in [("groq", "gsk_AAA"), ("gemini", "AIzaBBB")]:
        panel.event("provider", "commit", provider)
        assert (panel.key, panel.model) == (key, provider + "-model")


def test_a_busy_provider_is_retried_with_backoff(monkeypatch):
    import pytest
    """Gemini's 503 «high demand» cut Marco's house short at the walls
    (2026-09-15): the call is retried after a growing pause, telling the
    caller each time; a real error (401) is not retried."""
    import io
    import urllib.error
    from core import ai

    calls = []
    fails = [503, 503]

    class _Resp(io.BytesIO):
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    def fake_urlopen(req, timeout=0, context=None):
        calls.append(req.full_url)
        if fails:
            code = fails.pop(0)
            raise urllib.error.HTTPError(req.full_url, code, "busy", {},
                                         io.BytesIO(b'{"error": "high demand"}'))
        return _Resp(b'{"choices": [{"message": {"content": "ok"}}]}')

    monkeypatch.setattr(ai.urllib.request, "urlopen", fake_urlopen)
    waits, told = [], []
    out = ai._urlopen("https://x/v1/chat", {}, b"{}", retries=4,
                      on_retry=lambda n, t, w, r: told.append((n, t, r[:8])),
                      sleep=waits.append)
    assert out.startswith(b'{"choices"')
    assert len(calls) == 3 and waits == [2.0, 4.0]
    assert told == [(1, 4, "HTTP 503"), (2, 4, "HTTP 503")]
    # Out of retries: the error says how many tries it took.
    fails[:] = [503] * 5
    calls.clear()
    with pytest.raises(RuntimeError, match="after 3 attempts"):
        ai._urlopen("https://x/v1/chat", {}, b"{}", retries=2, sleep=lambda s: None)
    assert len(calls) == 3
    # A 401 is final at once.
    fails[:] = [401]
    calls.clear()
    with pytest.raises(RuntimeError, match="HTTP 401"):
        ai._urlopen("https://x/v1/chat", {}, b"{}", retries=4, sleep=lambda s: None)
    assert len(calls) == 1


def test_house_wall_and_prism_helpers_build_closed_solids(native_host):
    """Marco (2026-09-15): «se puede optimizar para que me dibuje algo
    mejor?». One call builds a house with walls of real thickness, a door
    leaf, glass panes and a roof — every solid watertight."""
    from built_core.orient import is_closed
    win = native_host.window
    vp = win.viewport
    scope: dict = {"__name__": "__ai__"}
    r = ai.run_transactional(vp, (
        "gs = house(6, 4, doors=[('S', 2.5, 0.9, 2.1)],"
        " windows=[('S', 0.6, 1.0, 1.2, 1.0), ('E', 1.5, 1.0, 1.2, 1.0),"
        " ('N', 1.0, 1.0, 1.2, 1.0)])\n"
        "w = wall((10, 0), (14, 0), 3, 0.2, openings=[(1, 0, 0.9, 2.1), (2.5, 1, 1, 1)],"
        " peak=(2, 1), name='Muro')\n"
        "p = prism([(20, 0, 0), (22, 0, 0), (22, 1, 0), (20, 1, 0)], (0, 0, 0.3),"
        " holes=[[(20.5, 0.3, 0), (21.5, 0.3, 0), (21.5, 0.7, 0), (20.5, 0.7, 0)]],"
        " name='Losa')\n"
        "print(len(gs))"), scope)
    assert r["error"] is None, r["error"]
    assert r["stdout"].strip() == "4"
    names = [g.name for g in vp.scene.groups[-6:]]
    assert names == ["House · Walls", "House · Floor", "House · Joinery",
                     "House · Roof", "Muro", "Losa"]
    walls, floor, carp, roof, muro, losa = vp.scene.groups[-6:]
    for g in (walls, floor, roof, muro, losa):
        assert is_closed(g.mesh), g.name
    # The front wall has a notch for the door and a hole for the window;
    # the gable ends rise to the ridge.
    zs = [v.position.z() for v in walls.mesh.vertices]
    assert max(zs) > 3.0 + 1.0
    assert any(len(f.hole_loops) == 1 for f in walls.mesh.faces)
    # Carpentry: one door leaf (a solid) and three panes (faces with opacity).
    panes = [f for f in carp.mesh.faces if f.attrs.get("opacity")]
    assert len(panes) == 3
    assert len(carp.mesh.faces) == 3 + 6
    # The roof overhangs the walls on every side.
    xs = [v.position.x() for v in roof.mesh.vertices]
    assert min(xs) < 0 and max(xs) > 6
    # The slab kept its hole.
    assert sum(1 for f in losa.mesh.faces if f.hole_loops) == 2
    # Hip and flat roofs close too.
    r = ai.run_transactional(vp, (
        "a = house(5, 8, roof='hip', origin=(30, 0), name='H')\n"
        "b = house(5, 5, roof='flat', origin=(40, 0), name='F')"), scope)
    assert r["error"] is None, r["error"]
    for g in vp.scene.groups[-6:]:
        assert is_closed(g.mesh), g.name


def test_code_less_claims_and_agentic_models_are_caught():
    from plugins.ai_assistant import _asks_to_build
    assert _asks_to_build("dibuja una casa de 2 aguas 6x4")
    assert _asks_to_build("Crea un poste")
    assert not _asks_to_build("¿qué medidas tiene el modelo?")
    # Groq's compound systems are not offered as chat models.
    assert any(tag in "groq/compound" for tag in ai._NON_CHAT)
    # A JPEG screenshot is announced with its real mime.
    _b64, mime = ai._message_image({"image_png_b64": "/9j/4AAQ"})
    assert mime == "image/jpeg"
    _b64, mime = ai._message_image({"image_png_b64": "iVBORw0KGgo"})
    assert mime == "image/png"


# ---- Missing / rejected key, in plain words (Rafael, Revisión 3) -----------

def test_missing_key_is_caught_before_any_network_call():
    """Rafael picked «Groq (gratis)», left the key empty and Test connection
    showed Groq's raw HTTP 401 JSON. A hosted provider with no key is known
    locally; Ollama and a pasted key say nothing."""
    from plugins.ai_assistant import AsistenteDialog
    hint = AsistenteDialog._missing_key_hint(None, "groq", "")
    assert hint is not None
    assert "console.groq.com" in hint
    assert AsistenteDialog._missing_key_hint(None, "ollama", "") is None
    assert AsistenteDialog._missing_key_hint(None, "groq", "gsk_x") is None
    assert "gratis, con clave" in ai.PROVIDER_INFO["groq"][0]


def test_rejected_key_names_the_prefix_to_check():
    from plugins.ai_assistant import AsistenteDialog
    err = ('HTTP 401: {"error":{"message":"Invalid API Key","type":'
           '"invalid_request_error","code":"invalid_api_key"}}')
    hint = AsistenteDialog._bad_key_hint("groq", err)
    assert hint is not None and "gsk_" in hint
    assert AsistenteDialog._bad_key_hint("groq", "HTTP 503: busy") is None


def test_openai_asks_for_max_completion_tokens():
    """OpenAI's newer models reject ``max_tokens`` with HTTP 400 (#517);
    ``max_completion_tokens`` works for all of them. The other
    OpenAI-compatible services keep ``max_tokens``, the name they accept."""
    msgs = [{"role": "user", "text": "x"}]
    _u, _h, payload = ai.build_request("openai", "gpt-5", "sk-k", "SYS",
                                       msgs, max_tokens=8192)
    body = json.loads(payload)
    assert body["max_completion_tokens"] == 8192
    assert "max_tokens" not in body
    for provider in ("groq", "gemini", "openrouter", "deepseek", "opencode",
                     "ollama"):
        _u, _h, payload = ai.build_request(provider, "m", "k", "SYS", msgs,
                                           max_tokens=8192)
        body = json.loads(payload)
        assert body["max_tokens"] == 8192, provider
        assert "max_completion_tokens" not in body, provider
    _u, _h, payload = ai.build_request("anthropic", "claude-sonnet-5", "k",
                                       "SYS", msgs, max_tokens=8192)
    assert json.loads(payload)["max_tokens"] == 8192


def test_test_connection_to_openai_sends_max_completion_tokens(monkeypatch):
    """#1032 reported the 400 «Use 'max_completion_tokens' instead» from
    Test connection: that button goes through probar_conexion → chat, the
    same request builder, so it must carry the new name too."""
    sent = []

    def fake_urlopen(url, headers, payload, timeout, **_kw):
        sent.append((url, json.loads(payload)))
        return json.dumps({"choices": [{"message": {"content": "OK"}}]}).encode()

    monkeypatch.setattr(ai, "_urlopen", fake_urlopen)
    ok, msg = ai.probar_conexion("openai", "gpt-6-sol", "sk-proj-k")
    assert ok and msg == "OK"
    url, body = sent[0]
    assert url == "https://api.openai.com/v1/chat/completions"
    assert "max_completion_tokens" in body and "max_tokens" not in body
