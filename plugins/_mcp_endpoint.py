"""Private, per-user filesystem transport for native POSIX MCP sessions."""
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import socket
import stat
import sys


def _directory(create=False):
    # sockaddr_un is limited to 104 bytes on macOS. Long profile paths
    # select a stable endpoint without becoming part of the socket address.
    profile = Path(os.environ.get("BUILT_SETTINGS_DIR") or Path.home()).resolve()
    profile_id = hashlib.sha256(os.fsencode(profile)).hexdigest()[:24]
    directory = Path("/tmp") / f"built-mcp-{os.getuid()}-{profile_id}"
    if create:
        directory.mkdir(mode=0o700, exist_ok=True)
    info = directory.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
            or stat.S_IMODE(info.st_mode) != 0o700):
        raise PermissionError(f"MCP directory must be owned by the current user and mode 0700: {directory}")
    return directory


def _clear_stale_socket(path):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid():
        raise FileExistsError(f"Refusing to replace a non-owned socket or another file: {path}")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
        probe.settimeout(.5)
        try:
            probe.connect(str(path))
        except ConnectionRefusedError:
            path.unlink()
        else:
            raise FileExistsError(f"An MCP listener is already active: {path}")


@contextmanager
def listen():
    """Hold an exclusive lock through socket removal, including after crashes."""
    import fcntl
    directory = _directory(create=True)
    path = directory / "bridge.sock"
    with (directory / "lock").open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        _clear_stale_socket(path)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
            server.bind(str(path))
            try:
                path.chmod(0o600)
                server.listen(1)
                yield server
            finally:
                path.unlink(missing_ok=True)


def connect_private():
    """Connect only to the listener belonging to this user and profile."""
    path = _directory() / "bridge.sock"
    info = path.lstat()
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid():
        raise PermissionError(f"Invalid MCP socket: {path}")
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.settimeout(1900)
        sock.connect(str(path))
        return sock
    except BaseException:
        sock.close()
        raise


def native_private_transport():
    return os.name == "posix" and "built_shell" in sys.builtin_module_names


def connect_client(host, port):
    if native_private_transport():
        return connect_private()
    return socket.create_connection((host, port), timeout=1900)
