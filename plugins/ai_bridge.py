# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Execute MCP requests on the main thread of the active document.

Native POSIX panels open a user-private Unix socket on explicit request.
The legacy client protocol uses newline-delimited JSON over loopback TCP.
"""
from __future__ import annotations

import json
import os
import socket
import sys
import tempfile
import threading
from pathlib import Path

from queue import Empty, Queue

from core.i18n import tr

DEFAULT_PORT = 4763
_REFUSED = b'{"id": null, "ok": false, "error": "not a bridge request; closing"}\n'


#: How long a request waits for its turn on the UI thread. The first frame
#: of a four-million-face model, or its IFC export, runs for minutes on that
#: thread; a short limit only turned a slow success into a reported failure.
UI_CALL_TIMEOUT = 1800.0


from core.agent_inspection import InspectionTools
from core.agent_records import RecordTools
from core.agent_facets import FacetTools
from core.agent_structure import StructureTools
from core.agent_protocol import ProtocolTools


class _Bridge(InspectionTools, RecordTools, FacetTools, StructureTools,
              ProtocolTools):
    """The in-app half: accepts one agent connection and executes its tool
    calls on the document host thread."""

    @property
    def _viewport(self):
        """The viewport of the project in front (projects in tabs, #1066):
        the one handed in at setup may be a tab in the back, or closed."""
        return self._live_viewport()

    def __init__(self, viewport) -> None:
        # The window's, not the viewport's: a project tab closed must not
        # take the bridge with it (#1066).
        self._endpoint = None
        self._jobs = Queue()
        self._timer = None
        from views.documents import live_viewport
        self._live_viewport = live_viewport(viewport)
        self._server: socket.socket | None = None
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self._scope: dict = {"__name__": "__ai__"}
        self.port: int | None = None
        self._busy = False
        from core.agent_tools import BUILTIN_TOOLS, extension_registry, register
        self.registry = {}
        self._extension_tools = extension_registry(viewport.window() or viewport)
        for spec in BUILTIN_TOOLS:
            register(self.registry, spec["name"], getattr(self, "_tool_" + spec["name"]),
                     description=spec["description"], input_schema=spec["inputSchema"],
                     mutates=spec.get("mutates", spec["name"] in {"run_python", "undo", "redo"}))
        register(self.registry, "list_tools", self._tool_list_tools,
                 description="List currently available agent tools.")

    def poll(self):
        try:
            job = self._jobs.get_nowait()
        except Empty:
            return
        self._run_on_main(job)

    # ---- Lifecycle ----------------------------------------------------------
    def start(self, *, private=False):
        if self.running:
            return self.port
        if private:
            from plugins._mcp_endpoint import listen
            endpoint = listen()
            srv = endpoint.__enter__()
            self._endpoint = endpoint
        else:
            port = int(os.environ.get("INGETRAZO_AI_PORT", DEFAULT_PORT))
            srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            try:
                srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                srv.bind(("127.0.0.1", port))
                srv.listen(1)
            except OSError:
                srv.close()
                raise
        srv.settimeout(0.5)
        self._server = srv
        self.port = None if private else srv.getsockname()[1]
        self._stop = threading.Event()
        self._jobs = Queue()
        self._thread = threading.Thread(
            target=self._serve, args=(srv, self._stop), name="ingetrazo-ai-bridge", daemon=True)
        try:
            self._timer = self._viewport.window()._host.timers.timer(self.poll, 20, True)
            self._timer.start()
            self._thread.start()
        except Exception:
            self.stop()
            raise
        return self.port

    def stop(self) -> None:
        self._stop.set()
        if self._timer is not None:
            self._timer.stop()
        if self._server is not None:
            try:
                self._server.close()
            except OSError:
                pass
            self._server = None
        if self._thread is not None and self._thread.is_alive():
            self._thread.join(timeout=1.5)
            self._thread = None
        if self._endpoint is not None:
            self._endpoint.__exit__(None, None, None)
            self._endpoint = None
        self.port = None

    @property
    def running(self) -> bool:
        return self._server is not None

    # ---- Worker side ---------------------------------------------------------
    def _serve(self, srv, stop) -> None:
        while not stop.is_set():
            try:
                conn, _addr = srv.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            with conn:
                conn.settimeout(0.5)
                buf = b""
                while not stop.is_set():
                    try:
                        chunk = conn.recv(65536)
                    except socket.timeout:
                        continue
                    except OSError:
                        break
                    if not chunk:
                        break
                    buf += chunk
                    if not self._drain(conn, buf):
                        break
                    buf = buf[buf.rfind(b"\n") + 1:]

    def _drain(self, conn, buf: bytes) -> bool:
        """Answer every complete line in ``buf``. False when the connection
        must close: a line that is not a JSON request ends it, and nothing
        after it runs.

        Loopback is not a wall against the browser on this machine. A web page
        can ``fetch("http://127.0.0.1:4763", {method: "POST", mode: "no-cors",
        body})`` — a text/plain POST needs no CORS preflight, so the browser
        sends it and only hides the reply. Read line by line, its request line
        and headers failed as bad JSON and its body, a tool call, ran. The
        first line of any HTTP request is ``POST / HTTP/1.1``, so refusing the
        connection there stops the body from ever being read. The MCP client
        sends nothing but JSON lines and never meets this."""
        lines = buf.split(b"\n")[:-1]
        for line in lines:
            if not line.strip():
                continue
            reply = self._handle_line(line)
            try:
                conn.sendall(reply if reply is not None else _REFUSED)
            except OSError:
                return False
            if reply is None:
                return False
        return True

    def _handle_line(self, line: bytes) -> bytes | None:
        """The reply to one request line, or None if it is not a request."""
        try:
            req = json.loads(line)
        except ValueError:
            return None
        if not isinstance(req, dict):
            return None
        job = {"req": req, "done": threading.Event(), "reply": None}
        self._jobs.put(job)
        import time
        deadline = time.monotonic() + UI_CALL_TIMEOUT
        while not job["done"].wait(.1):
            if self._stop.is_set() or time.monotonic() >= deadline:
                break
        reply = job["reply"] or {"id": req.get("id"), "ok": False,
                                 "error": f"timed out on the UI thread after "
                                          f"{UI_CALL_TIMEOUT:.0f} s (the call may "
                                          f"still finish; the next call waits for it)"}
        return (json.dumps(reply) + "\n").encode()

    # ---- Main-thread side ----------------------------------------------------
    def _run_on_main(self, job: dict) -> None:
        req = job["req"]
        tool = req.get("tool", "")
        args = req.get("args") or {}
        if self._busy:
            # A call delivered by an event loop nested in the previous one
            # (something it ran is still waiting): never run inside it.
            job["reply"] = {"id": req.get("id"), "ok": False, "error":
                            "Built is still running the previous call"}
            job["done"].set()
            return
        self._busy = True
        from views import chrome_dialogs, chrome_forms
        win = self._viewport.window()
        previous_forms = tuple(chrome_forms.forms(win))
        previous_sheets = tuple(chrome_dialogs._sheets(win))
        refused = []

        def cancel_new_dialogs():
            for form in reversed(tuple(chrome_forms.forms(win))):
                if form not in previous_forms:
                    refused.append(form.title)
                    form.close()
            sheet = chrome_dialogs.current_sheet(win)
            if sheet is not None and sheet not in previous_sheets:
                refused.append(sheet.title)
                if sheet.cancel is not None:
                    chrome_dialogs.answer(win, sheet.cancel)
                elif sheet.blocking:
                    sheet.event_loop.quit()
                else:
                    chrome_dialogs._pop(win, sheet)
                    if sheet.done:
                        sheet.done(None)

        # Synchronous questions pump native window timers while their caller
        # waits. Cancel them inside that loop so the request can return.
        guard = win._host.timers.timer(cancel_new_dialogs, 20, True)
        try:
            guard.start()
            entry = self.registry.get(tool) or self._extension_tools.get(tool)
            if entry is None:
                raise ValueError(f"unknown tool {tool!r}")
            if (entry['mutates'] and tool not in ('set_active_file', 'open_project')
                    and not (tool == 'bootstrap_structure' and not args.get('apply'))
                    and getattr(self._viewport.scene, 'worksession_read_only', False)):
                raise ValueError('Choose this file in Worksession before editing it')
            from core.agent_tools import json_value
            if entry["mutates"] and tool not in self.registry:
                from core.ai import run_callable_transactional
                result = run_callable_transactional(self._viewport, lambda: entry["fn"](**args))
            else:
                result = json_value(entry["fn"](**args))
            cancel_new_dialogs()
            if refused and isinstance(result, dict):
                result["dialogs_cancelled"] = refused
                result["stderr"] = (result.get("stderr") or "") + "".join(
                    f"Dialog cancelled: {title}\n" for title in refused)
            job["reply"] = {"id": req.get("id"), "ok": True, "result": result}
        except Exception as exc:  # noqa: BLE001 — reported to the agent
            job["reply"] = {"id": req.get("id"), "ok": False,
                            "error": f"{type(exc).__name__}: {exc}"}
        finally:
            guard.stop()
            self._busy = False
            job["done"].set()

    def _tool_list_tools(self):
        return {"tools": [dict(name=name, description=e["description"],
                               inputSchema=e["input_schema"],
                               annotations={"readOnlyHint": not e["mutates"]})
                          for name, e in (self.registry | self._extension_tools).items()]}

    # ---- Tools ---------------------------------------------------------------
    def _tool_run_python(self, code: str = "") -> dict:
        """Execute ``code`` via the shared transactional executor (core.ai):
        one undoable step, whole-rollback on error, no undo entry when
        nothing changed."""
        from core.ai import run_transactional
        return run_transactional(self._viewport, code, self._scope)

    def _tool_query_model(self, include_bounds: bool = True) -> dict:
        vp = self._viewport
        scene = vp.scene
        lo, hi = scene.bounds() if include_bounds else (None, None)
        insts = sum(1 for g in scene.groups
                    if getattr(g, "xform", None) is not None)
        return {
            "version": scene.version,
            "faces": len(scene.mesh.faces),
            "edges": len(scene.mesh.edges),
            "groups": len(scene.groups) - insts,
            "component_instances": insts,
            "group_names": [g.name for g in scene.groups][:100],
            "materials": sorted(getattr(scene, "materials", {}) or {})[:100],
            # Keys are what g.layer takes; labels may repeat.
            "layers": [ly.key for ly in scene.layers],
            "layer_labels": {ly.key: ly.name for ly in scene.layers
                             if ly.name != ly.key},
            "dimensions": len(getattr(scene, "dimensions", []) or []),
            "section_planes": len(getattr(scene, "section_planes", []) or []),
            "selection": len(scene.selection),
            "bounds": (None if lo is None else
                       {"min": [lo.x(), lo.y(), lo.z()],
                        "max": [hi.x(), hi.y(), hi.z()]}),
        }

    def _tool_query_groups(self, name: str = "", offset: int = 0,
                           limit: int = 100) -> dict:
        """Read nested group metadata without snapshots, bakes or bounds.

        Return detached JSON values, never live objects or executable Python.
        Paths contain stable uids so identically named groups are distinct.
        """
        if not isinstance(name, str):
            raise ValueError("name must be a string")
        if type(offset) is not int or offset < 0:
            raise ValueError("offset must be a non-negative integer")
        if type(limit) is not int or not 1 <= limit <= 1000:
            raise ValueError("limit must be between 1 and 1000")
        scene = self._viewport.scene
        stack = [(g, []) for g in reversed(scene.groups)]
        rows = []
        matched = 0
        while stack:
            group, parents = stack.pop()
            path = parents + [group.uid]
            stack.extend((g, path) for g in reversed(group.children or []))
            if name.casefold() not in (group.name or "").casefold():
                continue
            if matched >= offset:
                if len(rows) == limit:
                    return {"groups": rows, "next_offset": offset + limit,
                            "version": scene.version}
                rows.append({"uid": group.uid, "name": group.name,
                             "path": path, "layer": group.layer,
                             "hidden": group.hidden,
                             "children": len(group.children or []),
                             "component": group.component})
            matched += 1
        return {"groups": rows, "next_offset": None, "version": scene.version}

    def _tool_query_changes(self, since_revision: int = 0) -> dict:
        """Read the current document's bounded command journal."""
        return self._viewport.history.changes_since(since_revision)

    def _tool_query_status(self, name: str) -> dict:
        """Read JSON status published in run_python's persistent scope.

        A recipe can assign ``job_status = {'done': True}``. Reading it
        never executes code, touches history or schedules a redraw.
        """
        if not isinstance(name, str) or name.startswith("_"):
            raise ValueError("name must be a public scope variable")
        if name not in self._scope:
            return {"found": False, "value": None}
        # Restrict to plain JSON containers/scalars; do not invoke custom
        # encoders, properties or arbitrary object's iteration methods.
        def copy(value, depth=0):
            if depth > 32:
                raise ValueError("status is too deeply nested")
            if type(value) in (str, int, float, bool, type(None)):
                return value
            from core.doc_model import JSON_DICTS, JSON_LISTS
            if type(value) in JSON_LISTS:
                return [copy(v, depth + 1) for v in value]
            if type(value) in JSON_DICTS and all(type(k) is str for k in value):
                return {k: copy(v, depth + 1) for k, v in value.items()}
            raise ValueError("status must contain only JSON values")
        return {"found": True, "value": copy(self._scope[name])}

    def _tool_screenshot(self, width: int = 1024, height: int = 768, uids=None) -> dict:
        """Capture the shown view. On Metal the ride-model agent framed the
        shared camera but captured the hidden GL widget (2026-11-09)."""
        if uids is not None:
            self._tool_zoom_to(uids)
        width = max(64, min(int(width), 4096))
        height = max(64, min(int(height), 4096))
        window = self._viewport.window()
        shown = getattr(window, "_shown_viewport", None)
        view = shown() if shown is not None else self._viewport
        from app_shell import native
        if native.available():
            from ingetrazo_render import SurveyRaster
            frame = window.view.render_pixels(width, height)
            image = SurveyRaster(*frame) if frame else None
        else:
            image = view.render_image(width, height)
        if not hasattr(self, "_captures"):
            self._captures = tempfile.TemporaryDirectory(prefix="built-mcp-capture-")
        out = Path(self._captures.name) / "viewport.png"
        if image is None or not image.save(str(out)):
            raise RuntimeError("Could not save the viewport screenshot")
        return {"path": str(out), "width": width, "height": height,
                "camera": self._tool_query_camera()}

    def _tool_undo(self) -> dict:
        ok = self._viewport.history.undo()
        self._viewport.update()
        return {"ok": bool(ok)}

    def _tool_redo(self) -> dict:
        ok = self._viewport.history.redo()
        self._viewport.update()
        return {"ok": bool(ok)}


def mcp_command(platform: str | None = None, frozen: bool | None = None,
                executable: str | None = None, root: Path | None = None,
                env: dict | None = None) -> list[str]:
    """The command an MCP client must run to reach this IngeTrazo — the
    packaged app carries the server, so nobody needs Python installed:
    ``ingetrazo-mcp.exe`` beside the app on Windows, ``<ingetrazo> --mcp``
    for the Linux/macOS packages, and the script itself from a checkout.

    The Linux packages need the command that is valid FROM THE HOST and
    OUTLIVES this run, not ``sys.executable``: an AppImage mounts itself
    under a fresh ``/tmp/.mount_*`` each launch (the path the dialog used
    to print died with the session), a Flatpak's executable lives inside
    the sandbox (``flatpak run <id> --mcp`` is the door), and a snap is
    reached through ``/snap/bin``. Marco found the gap setting Antigravity
    up for the tutorial (2026-09-21)."""
    from pathlib import PurePosixPath, PureWindowsPath
    platform = platform or sys.platform
    frozen = bool(getattr(sys, "frozen", False)) if frozen is None else frozen
    executable = executable or sys.executable
    env = os.environ if env is None else env
    windows = platform.startswith("win")
    PathOf = PureWindowsPath if windows else PurePosixPath   # host-agnostic
    if not windows:
        if env.get("APPIMAGE"):
            return [env["APPIMAGE"], "--mcp"]
        if env.get("FLATPAK_ID"):
            return ["flatpak", "run", env["FLATPAK_ID"], "--mcp"]
        if env.get("SNAP_NAME"):
            return [f"/snap/bin/{env['SNAP_NAME']}", "--mcp"]
    if frozen:
        exe = PathOf(executable)
        if windows:
            return [str(exe.with_name("ingetrazo-mcp.exe"))]
        return [str(exe), "--mcp"]
    root = PathOf(str(root or Path(__file__).resolve().parents[1]))
    python = "python" if windows else "python3"
    return [python, str(root / "scripts" / "ingetrazo_mcp.py")]


def desktop_config_path(platform: str | None = None) -> str:
    """Where Claude Desktop reads its MCP servers on this platform."""
    platform = platform or sys.platform
    if platform.startswith("win"):
        return r"%APPDATA%\\Claude\\claude_desktop_config.json"
    if platform == "darwin":
        return "~/Library/Application Support/Claude/claude_desktop_config.json"
    return "~/.config/Claude/claude_desktop_config.json"


#: Where the common MCP clients read a ``mcpServers`` block like the one the
#: dialog prints (the same JSON, or the equivalent, works in every one:
#: MCP is an open standard, not a Claude feature).
OTHER_CLIENTS = (
    ("Cursor", "~/.cursor/mcp.json"),
    ("VS Code (Copilot)", ".vscode/mcp.json  (key \"servers\")"),
    ("Windsurf", "~/.codeium/windsurf/mcp_config.json"),
    # Google retired Gemini CLI's free Google-account login on 2026-06-18;
    # its successor for individuals is Antigravity CLI (`agy`), which reads
    # this file (Marco hit the shutdown notice on 2026-09-21).
    ("Antigravity CLI (Google)", "~/.gemini/config/mcp_config.json"),
    ("Codex CLI", "~/.codex/config.toml  ([mcp_servers.ingetrazo])"),
)


def connect_instructions(port: int, platform: str | None = None, **kw) -> str:
    """Copy-and-paste text for the dialog the Extensions entry shows once
    the bridge is up: Claude Code's one-liner, the JSON every other MCP
    client takes, and where each of them reads it."""
    cmd = mcp_command(platform, **kw)
    quoted = " ".join(f'"{c}"' if " " in c else c for c in cmd)
    config = {"mcpServers": {"ingetrazo": {"command": cmd[0], "args": cmd[1:]}}}
    others = "\n".join(f"    {name}: {path}" for name, path in OTHER_CLIENTS)
    return (
        tr("The AI bridge is listening on 127.0.0.1:{port}. Connect ANY MCP "
           "client — MCP is an open standard: Claude Code, Claude Desktop, "
           "Cursor, VS Code, Windsurf, Antigravity CLI, Codex CLI…", port=port)
        + "\n\n"
        + tr("Claude Code (in a terminal):") + "\n"
        + f"    claude mcp add ingetrazo -- {quoted}\n\n"
        + tr("Antigravity CLI (Google; in a terminal):") + "\n"
        + f"    agy mcp add ingetrazo -- {quoted}\n\n"
        + tr("Claude Desktop: add this to {path} and restart Claude Desktop:",
             path=desktop_config_path(platform)) + "\n"
        + json.dumps(config, indent=2) + "\n\n"
        + tr("Other clients take the same block in their own file:") + "\n"
        + others + "\n\n"
        + tr("Keep Built open with the bridge on; the tools answer only while it runs.")
    )


def _bridge_of(viewport) -> _Bridge:
    window = viewport.window()
    bridge = getattr(window, "_ai_bridge", None)
    if bridge is None:
        bridge = _Bridge(viewport)
        window._ai_bridge = bridge
        window._extension_closed.connect(bridge.stop)
    return bridge


class BridgeSection:
    """Native descriptor for the window's MCP listener."""
    id, title, slot = "bridge", tr("AI Bridge (MCP)"), "dock"

    def __init__(self, viewport, invalidate=lambda: None):
        from views.documents import live_viewport
        self._live_viewport = live_viewport(viewport)
        self.invalidate = invalidate

    @property
    def _viewport(self):
        return self._live_viewport()

    @property
    def running(self):
        bridge = getattr(self._viewport.window(), "_ai_bridge", None)
        return bridge is not None and bridge.running

    def stop(self):
        if self.running:
            _bridge_of(self._viewport).stop()
        self.invalidate()

    def sections(self):
        from core.plugin_panels import item, section
        controls = [item("button", "toggle", tr("Stop bridge") if self.running else tr("Start bridge"))]
        if self.running:
            controls.append(item("log", "instructions", lines=[
                tr("The private bridge is listening. Configure your MCP client to run:"),
                f'"{sys.executable}" --mcp',
            ]))
        return [section("connection", self.title, *controls)]

    def event(self, control, event, value):
        if (control, event) == ("toggle", "invoke"):
            if self.running:
                self.stop()
            else:
                _bridge_of(self._viewport).start(private=True)
                self.invalidate()


def create_panel(app, invalidate):
    return BridgeSection(app.viewport, invalidate)
