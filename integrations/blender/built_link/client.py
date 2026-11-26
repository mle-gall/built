# SPDX-License-Identifier: GPL-3.0-or-later
"""The add-on's end of the link: one loopback socket to Built.

A background thread connects, sends ``hello`` (pairing token, document,
what this .blend already holds), reads frames and queues whole messages.
Blender's main thread drains the queue from a timer and sends with
``send``. A dropped link is retried every few seconds until Built refuses
it (wrong token, another model) or the user unlinks. No Blender import.
"""
import queue
import socket
import threading
import time

from . import wire

RETRY_SECONDS = 2.0
HOST = "127.0.0.1"


class Client:
    def __init__(self):
        self.inbox = queue.Queue()
        self.state = "idle"        # idle, connecting, linked, waiting, refused
        self.message = ""
        self.welcome = {}
        self.hello = {}            # extra hello fields, refreshed by the main thread
        self.relink = False
        self._lock = threading.Lock()
        self._sock = None
        self._stop = threading.Event()
        self._thread = None
        self._msg = 1
        self.port = self.token = None

    # ------------------------------------------------------------ control

    def start(self, port, token, hello=None, relink=False):
        self.stop()
        self.port, self.token = int(port), token
        if hello is not None:
            self.hello = dict(hello)
        self.relink = relink
        self._stop = threading.Event()
        self.state, self.message = "connecting", ""
        self._thread = threading.Thread(target=self._run, args=(self._stop,), name="built-link", daemon=True)
        self._thread.start()

    def stop(self, say_bye=True):
        self._stop.set()
        with self._lock:
            sock, self._sock = self._sock, None
        if sock is not None:
            if say_bye:
                try:
                    sock.sendall(wire.encode({"type": "bye"}))
                except OSError:
                    pass
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            sock.close()
        if self._thread is not None and self._thread is not threading.current_thread():
            self._thread.join(timeout=2)
        self._thread = None
        if self.state != "refused":
            self.state = "idle"

    @property
    def linked(self):
        return self.state == "linked"

    def send(self, header, payload=b""):
        """Write one message now; False when not linked."""
        with self._lock:
            sock = self._sock
            if sock is None or self.state != "linked":
                return False
            data = wire.encode(header, payload, msg=self._msg)
            self._msg += 1
            try:
                sock.sendall(data)
            except OSError:
                return False
        return True

    def drain(self, limit=64):
        out = []
        while len(out) < limit:
            try:
                out.append(self.inbox.get_nowait())
            except queue.Empty:
                break
        return out

    # ------------------------------------------------------------ thread

    def _run(self, stop):
        while not stop.is_set():
            reason = self._session(stop)
            if stop.is_set() or self.state == "refused":
                return
            self.state, self.message = "waiting", reason
            self.inbox.put(({"type": "_closed", "reason": reason}, b""))
            stop.wait(RETRY_SECONDS)

    def _session(self, stop):
        try:
            sock = socket.create_connection((HOST, self.port), timeout=3)
        except OSError:
            return "Built is not listening"
        sock.settimeout(0.5)
        hello = {"type": "hello", "protocol": wire.PROTOCOL, "token": self.token,
                 "relink": self.relink, **self.hello}
        asm = wire.Assembler()
        try:
            sock.sendall(wire.encode(hello))
            first = self._read(sock, asm, stop, deadline=time.monotonic() + 10)
        except (OSError, wire.WireError):
            first = None
        if first is None:
            sock.close()
            return "Built did not answer"
        header, _ = first
        if header.get("type") == "refused":
            sock.close()
            self.state, self.message = "refused", header.get("message", "Built refused the link")
            self.inbox.put(({**header, "type": "_refused"}, b""))
            return self.message
        with self._lock:
            self._sock = sock
            self.welcome = header
            self.state = "linked"
            self.relink = False
        self.inbox.put(({**header, "type": "_linked"}, b""))
        while not stop.is_set():
            try:
                got = self._read(sock, asm, stop, deadline=None)
            except (OSError, wire.WireError) as exc:
                got, why = None, str(exc)
            else:
                why = "Built closed the link"
            if got is None:
                with self._lock:
                    if self._sock is sock:
                        self._sock = None
                try:
                    sock.close()
                except OSError:
                    pass
                return why
            self.inbox.put(got)
        return "unlinked"

    def _read(self, sock, asm, stop, deadline):
        pending = getattr(asm, "_ready", None)
        if pending:
            return pending.pop(0)
        while not stop.is_set():
            if deadline is not None and time.monotonic() > deadline:
                return None
            try:
                data = sock.recv(256 * 1024)
            except socket.timeout:
                continue
            if not data:
                return None
            msgs = asm.feed(data)
            if msgs:
                asm._ready = msgs[1:]
                return msgs[0]
        return None
