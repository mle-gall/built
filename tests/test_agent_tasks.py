# SPDX-License-Identifier: GPL-3.0-or-later
"""Agent tasks: each request to the built-in assistant or an MCP agent is a
task kept in the document as a BCF 3.0 topic; the edits made for it are
attributed to it, reviewed, accepted or reverted as one undo step; agents
reach it over MCP only while Settings › AI allows it."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import json
import socket
import threading

import pytest

import built_render


def _tasks(win):
    return json.loads(built_render.task_list(win, True))


def _task(win, guid):
    return json.loads(built_render.task_get(win, guid))


def _groups(vp, *names):
    """Groups made as the user (not attributed to any task)."""
    from core.ai import run_transactional
    code = "\n".join(f"groups.append(Group(Mesh(), name={n!r}))" for n in names)
    assert run_transactional(vp, code, {})["changed"]
    return [next(g for g in vp.scene.groups if g.name == n) for n in names]


def _agent_edit(win, guid, code):
    from core.ai import run_transactional
    marker = built_render.task_edit_begin(win)
    result = run_transactional(win.viewport, code, {})
    built_render.task_edit_end(win, guid, marker)
    assert result["error"] is None, result
    return result


def test_tasks_live_in_the_document_as_bcf_topics(native_host, tmp_path):
    from formats import igz
    from built_core.scene import Scene
    win = native_host.window
    scene = win.viewport.scene
    guid = built_render.task_new(win, "Align the backrest slats", "Align them with the frame",
                                 "Codex", "agent", "", False, True)
    assert built_render.task_report(win, guid, "question", "From the backrest or the rear posts?") == "question"
    bcf = scene.plugin_data["bcf"]
    assert bcf["version"] == "3.0"
    topic = bcf["topics"][0]
    assert (topic["guid"], topic["topic_status"], topic["assigned_to"]) == (guid, "Question", "Codex")
    assert topic["comments"][0]["comment"] == "From the backrest or the rear posts?"
    camera = topic["viewpoints"][0]
    assert "perspective_camera" in camera or "orthogonal_camera" in camera
    # An unsaved change: the document asks to be saved.
    assert scene.version != win._saved_version
    path = tmp_path / "tasks.igz"
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert reopened.plugin_data["bcf"] == bcf
    # A step that restores plugin_data on undo does not take the tasks along.
    _groups(win.viewport, "Bench")
    win.viewport.history.undo()
    built_render.tasks_before_frame(native_host.viewport, win)
    assert scene.plugin_data["bcf"]["topics"][0]["topic_status"] == "Question"


def test_a_task_without_net_edits_settles_to_done(native_host):
    win = native_host.window
    guid = built_render.task_new(win, "Check the clearance", "", "Codex", "agent", "", False, True)
    marker = built_render.task_edit_begin(win)
    assert built_render.task_edit_end(win, guid, marker) == 0
    assert built_render.task_finish(win, guid, "Clearance is 0.62 m") == "done"
    # Added then removed by the same task: no net edit either.
    guid = built_render.task_new(win, "Try a post", "", "Codex", "agent", "", False, True)
    _agent_edit(win, guid, "groups.append(Group(Mesh(), name='Trial post'))")
    _agent_edit(win, guid, "groups.remove(next(g for g in groups if g.name == 'Trial post'))")
    assert _task(win, guid)["changed"] == 0
    assert built_render.task_finish(win, guid, "") == "done"


def test_edits_are_attributed_and_revert_is_one_undo_step(native_host):
    win = native_host.window
    vp = win.viewport
    history = vp.history
    slat, post = _groups(vp, "Slat 01", "Post")
    guid = built_render.task_new(win, "Align the backrest slats", "", "Codex", "agent", "", False, True)
    _agent_edit(win, guid, "next(g for g in groups if g.name == 'Slat 01').name = 'Slat 01 aligned'")
    assert built_render.task_finish(win, guid, "Aligned one slat") == "review"
    task = _task(win, guid)
    assert task["built"]["changes"] == [{"key": slat.uid, "label": "Slat 01 aligned", "change": "modified"}]
    assert task["needs_attention"] and task["topic_status"] == "To review"
    # Unrelated work after the task is left alone.
    from built_core.history import NativeCommand
    history.execute(NativeCommand.rename(post, "Rear post"))
    depth = len(history.undo_stack)
    built_render.task_revert(win, guid)
    assert len(history.undo_stack) == depth + 1
    assert (slat.name, post.name) == ("Slat 01", "Rear post")
    assert _task(win, guid)["state"] == "reverted"
    # One undo brings the task's edits and its review back.
    assert history.undo()
    assert (slat.name, post.name) == ("Slat 01 aligned", "Rear post")
    assert _task(win, guid)["state"] == "review"
    assert history.redo()
    assert slat.name == "Slat 01" and _task(win, guid)["state"] == "reverted"


def test_revert_on_top_of_the_history_uses_the_steps_own_inverse(native_host):
    win = native_host.window
    vp = win.viewport
    guid = built_render.task_new(win, "Add a seat and a line", "", "Codex", "agent", "", False, True)
    edges = len(vp.scene.mesh.edges)
    _agent_edit(win, guid, "groups.append(Group(Mesh(), name='Seat'))\n"
                           "mesh.add_edge(QVector3D(0,0,0), QVector3D(2,0,0))")
    assert built_render.task_finish(win, guid, "") == "review"
    labels = {c["label"] for c in _task(win, guid)["built"]["changes"]}
    assert labels == {"Seat"}
    built_render.task_revert(win, guid)
    assert not any(g.name == "Seat" for g in vp.scene.groups)
    assert len(vp.scene.mesh.edges) == edges
    vp.history.undo()
    assert any(g.name == "Seat" for g in vp.scene.groups) and len(vp.scene.mesh.edges) == edges + 1


def test_revert_refuses_when_later_work_changed_the_same_elements(native_host):
    win = native_host.window
    vp = win.viewport
    (slat,) = _groups(vp, "Slat 02")
    guid = built_render.task_new(win, "Rename the slat", "", "Codex", "agent", "", False, True)
    _agent_edit(win, guid, "next(g for g in groups if g.name == 'Slat 02').name = 'Slat B'")
    built_render.task_finish(win, guid, "")
    from built_core.history import NativeCommand
    vp.history.execute(NativeCommand.rename(slat, "Slat C"))
    with pytest.raises(ValueError, match="Later edits changed Slat C"):
        built_render.task_revert(win, guid)
    assert slat.name == "Slat C" and _task(win, guid)["state"] == "review"
    # The Tasks verb keeps the reason on the task instead of raising.
    built_render.task_action(win, f"revert:{guid}")
    built_render.task_action(win, f"select:{guid}")
    built_render.task_action(win, f"revert:{guid}")
    built_render.tasks_before_frame(native_host.viewport, win)


def test_questions_replies_and_requests_for_changes(native_host):
    win = native_host.window
    guid = built_render.task_new(win, "Check the clearance", "", "Claude Code", "agent", "", False, True)
    built_render.task_report(win, guid, "question", "From the backrest or the rear posts?")
    built_render.task_action(win, f"reply:{guid}\x1fThe rear posts")
    task = _task(win, guid)
    assert task["state"] == "working" and task["comments"][-1]["comment"] == "The rear posts"
    built_render.task_report(win, guid, "done", "0.62 m")
    built_render.task_action(win, f"accept:{guid}")       # nothing to review: Done already
    assert _task(win, guid)["state"] == "done"
    built_render.task_action(win, f"changes:{guid}\x1fMeasure to the posts' faces")
    assert _task(win, guid)["state"] == "working"


def test_a_new_request_from_tasks_runs_the_assistant_as_a_task(native_host, monkeypatch):
    import time
    from core import ai
    replies = iter(["```python\ngroups.append(Group(Mesh(), name='Bench'))\n```",
                    "Added the bench."])
    monkeypatch.setattr(ai, "chat", lambda *a, **k: next(replies))
    from built_core.settings import settings
    settings().setValue("ia/proveedor", "ollama")
    settings().setValue("ia/capturas", 0)
    win = native_host.window
    vp = win.viewport
    vp.scene.selection.clear()
    built_render.task_action(win, "new:Built-in assistant\x1f0\x1fAdd a bench")
    guid = _tasks(win)[0]["guid"]
    panel = vp.scene._task_assistant
    deadline = time.monotonic() + 5
    while panel.busy and time.monotonic() < deadline:
        native_host.timers.run_due()
        time.sleep(.01)
    task = _task(win, guid)
    assert task["assigned_to"] == "Built-in assistant" and task["built"]["source"] == "assistant"
    assert task["state"] == "review", task
    assert [c["label"] for c in task["built"]["changes"]] == ["Bench"]
    assert any(c["comment"] == "Added the bench." for c in task["comments"])
    built_render.task_action(win, f"accept:{guid}")
    assert _task(win, guid)["state"] == "done"


# ---- MCP: a fake client on the bridge's socket -------------------------------------

def _call(bridge, tool, args=None, client="Codex", sock=None):
    from plugins.ai_bridge import UI_CALL_TIMEOUT
    result = {}

    def worker(conn):
        conn.settimeout(UI_CALL_TIMEOUT + 10)
        conn.sendall((json.dumps({"id": 1, "tool": tool, "args": args or {}, "client": client}) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            chunk = conn.recv(65536)
            assert chunk, "bridge closed"
            buf += chunk
        result["reply"] = json.loads(buf.split(b"\n", 1)[0])

    thread = threading.Thread(target=worker, args=(sock,))
    thread.start()
    while thread.is_alive():
        bridge.poll()
        thread.join(timeout=.01)
    return result["reply"]


@pytest.fixture
def agent(native_host, monkeypatch):
    """Model access on: the window listens; one MCP client connected."""
    monkeypatch.setenv("BUILT_AI_PORT", "0")
    win = native_host.window
    built_render.set_agent_access(win, True)
    bridge = win._ai_bridge
    assert bridge.running and bridge.port
    sock = socket.create_connection(("127.0.0.1", bridge.port), timeout=10)
    yield win, bridge, sock
    sock.close()
    built_render.set_agent_access(win, False)


def test_mcp_tools_create_and_update_tasks_and_their_edits_belong_to_them(agent):
    win, bridge, sock = agent
    reply = _call(bridge, "task_create", {"title": "Add a bench", "description": "A 2 m bench"}, sock=sock)
    assert reply["ok"], reply
    guid = reply["result"]["guid"]
    assert bridge.clients() == ["Codex"]
    reply = _call(bridge, "run_python", {"code": "groups.append(Group(Mesh(), name='Bench'))"}, sock=sock)
    assert reply["ok"] and [c["label"] for c in _task(win, guid)["built"]["changes"]] == ["Bench"]
    # A call that changes nothing opens no task and adds no change.
    assert _call(bridge, "run_python", {"code": "x = 1"}, sock=sock)["ok"]
    assert len(_tasks(win)) == 1
    reply = _call(bridge, "task_update", {"guid": guid, "state": "question", "message": "Oak or pine?"}, sock=sock)
    assert reply["result"]["state"] == "question" and reply["result"]["built"]["question"] == "Oak or pine?"
    built_render.task_reply(win, guid, "Oak")
    reply = _call(bridge, "task_list", sock=sock)
    listed = reply["result"]["tasks"][0]
    assert listed["state"] == "working" and listed["comments"][-1]["comment"] == "Oak"
    reply = _call(bridge, "task_update", {"guid": guid, "state": "done", "message": "Added an oak bench"}, sock=sock)
    task = reply["result"]
    assert task["state"] == "review" and task["assigned_to"] == "Codex"
    assert [c["label"] for c in task["built"]["changes"]] == ["Bench"]
    # An edit with no task open opens one, so every external edit is reviewable.
    reply = _call(bridge, "run_python", {"code": "groups.append(Group(Mesh(), name='Table'))"}, sock=sock)
    other = _tasks(win)[0]["guid"]
    assert other != guid and _task(win, other)["title"] == "Changes from Codex"
    assert [c["label"] for c in _task(win, other)["built"]["changes"]] == ["Table"]
    assert json.loads(built_render.task_list(win, False))[0]["guid"] == other


def test_turning_access_off_refuses_agents_and_stops_listening(native_host, monkeypatch):
    from plugins.ai_bridge import _Bridge
    monkeypatch.setenv("BUILT_AI_PORT", "0")
    win = native_host.window
    vp = win.viewport
    assert not built_render.agent_access(win)            # off until turned on
    bridge = _Bridge(vp)
    bridge.start()
    try:
        with socket.create_connection(("127.0.0.1", bridge.port), timeout=10) as sock:
            groups = len(vp.scene.groups)
            reply = _call(bridge, "run_python", {"code": "groups.append(Group(Mesh(), name='X'))"}, sock=sock)
            assert not reply["ok"] and "Allow agents to edit this model" in reply["error"]
            assert len(vp.scene.groups) == groups and not _tasks(win)
            reply = _call(bridge, "task_create", {"title": "Sneak"}, sock=sock)
            assert not reply["ok"]
            assert _call(bridge, "list_tools", sock=sock)["ok"]
    finally:
        bridge.stop()
    built_render.set_agent_access(win, True)
    assert win._ai_bridge.running
    built_render.set_agent_access(win, False)
    assert not win._ai_bridge.running


def test_settings_ai_holds_the_access_switch_and_the_setup(native_host, monkeypatch):
    from tests.preferences_chrome import Controls
    monkeypatch.setenv("BUILT_AI_PORT", "0")
    win = native_host.window
    c = Controls(win, "ai")
    assert c.value("agent_access") is False
    assert {"ia/proveedor", "ia/api_key", "agent_access"} <= {r.id for _, _, rows in c.sheet.groups(c.env, "ai") for r in rows}
    c.set("agent_access", True)
    try:
        assert built_render.agent_access(win) and win._ai_bridge.running
        page = json.dumps(c.wire())
        assert "Active" in page and "Connect an agent" in page
        c.sheet.act(c.ctx, "verb", ["ai_connect"])
        page = json.dumps(c.wire())
        assert "mcp add built" in page and "Copy setup" in page
        from plugins.ai_bridge import setup_command
        assert setup_command("Codex")[0].startswith("codex mcp add built -- ")
        c.sheet.act(c.ctx, "verb", ["ai_connect_close"])
    finally:
        c.set("agent_access", False)
    assert not built_render.agent_access(win)


def test_the_assistant_and_bridge_are_not_plugin_panels():
    from plugins._native import discover_plugins
    plugins, _ = discover_plugins(native_panels={"render_blender": "plugins._blender_panel",
                                                 "python_console": "plugins.python_console"})
    stems = {p.stem for p in plugins}
    assert not stems & {"ai_assistant", "ai_bridge"}
