# SPDX-License-Identifier: GPL-3.0-or-later
"""The link's frames, as Built writes them (rust/built_core/src/blender/link/wire.rs).

A frame is an 8-byte prefix (JSON length, payload length: big-endian u32),
the UTF-8 JSON object, then the payload. A payload larger than ``CHUNK``
travels as several frames sharing ``msg`` with ``part`` / ``parts``.
No Blender import: tested on its own.
"""
import json
import struct

PROTOCOL = 1
CHUNK = 1 << 20
MAX_HEADER = 1 << 20
MAX_FRAME_PAYLOAD = 4 << 20
MAX_MESSAGE = 512 << 20


class WireError(ValueError):
    pass


def _frame(header, payload):
    data = json.dumps(header, separators=(",", ":")).encode("utf-8")
    return struct.pack(">II", len(data), len(payload)) + data + payload


def encode(header, payload=b"", msg=1, chunk=CHUNK):
    """The bytes of one message (several frames when the payload is large)."""
    payload = bytes(payload or b"")
    chunk = max(1, min(chunk, MAX_FRAME_PAYLOAD))
    if len(payload) <= chunk:
        return _frame(header, payload)
    pieces = [payload[i:i + chunk] for i in range(0, len(payload), chunk)]
    out = []
    for i, piece in enumerate(pieces):
        h = dict(header) if i == 0 else {"type": header.get("type")}
        h.update(msg=msg, part=i, parts=len(pieces))
        out.append(_frame(h, piece))
    return b"".join(out)


def pairing(text):
    """``built:<port>:<token>`` → (port, token); ValueError when malformed."""
    parts = (text or "").strip().split(":")
    if len(parts) != 3 or parts[0] != "built" or not parts[1].isdigit() or not parts[2]:
        raise ValueError("A pairing code looks like built:47815:ABCD-EFGH-JKLM-NPQR")
    port = int(parts[1])
    if not 0 < port < 65536:
        raise ValueError("The pairing code names no port")
    return port, parts[2].upper()


class Assembler:
    """Bytes in, whole ``(header, payload)`` messages out."""

    def __init__(self):
        self.buffer = bytearray()
        self.partial = {}

    def pending(self):
        return len(self.buffer) + sum(len(p[3]) for p in self.partial.values())

    def feed(self, data):
        self.buffer.extend(data)
        out = []
        while len(self.buffer) >= 8:
            hlen, plen = struct.unpack(">II", bytes(self.buffer[:8]))
            if hlen > MAX_HEADER or plen > MAX_FRAME_PAYLOAD:
                raise WireError("frame too large")
            if len(self.buffer) < 8 + hlen + plen:
                break
            try:
                header = json.loads(bytes(self.buffer[8:8 + hlen]).decode("utf-8"))
            except ValueError as exc:
                raise WireError(f"bad frame header: {exc}") from None
            if not isinstance(header, dict):
                raise WireError("bad frame header: not an object")
            payload = bytes(self.buffer[8 + hlen:8 + hlen + plen])
            del self.buffer[:8 + hlen + plen]
            joined = self._join(header, payload)
            if joined is not None:
                out.append(joined)
        return out

    def _join(self, header, payload):
        parts = header.get("parts")
        if parts is None:
            return header, payload
        msg, part = header.pop("msg", None), header.pop("part", None)
        header.pop("parts", None)
        if not isinstance(parts, int) or parts <= 0 or parts > MAX_MESSAGE // CHUNK + 1:
            raise WireError("bad message parts")
        if part == 0:
            if msg in self.partial:
                raise WireError(f"message {msg} restarted")
            self.partial[msg] = [header, parts, 0, bytearray()]
        entry = self.partial.get(msg)
        if entry is None or entry[2] != part or entry[1] != parts:
            raise WireError(f"message {msg}: part {part} out of order")
        if len(entry[3]) + len(payload) > MAX_MESSAGE:
            raise WireError("message too large")
        entry[3].extend(payload)
        entry[2] += 1
        if entry[2] < entry[1]:
            return None
        del self.partial[msg]
        return entry[0], bytes(entry[3])
