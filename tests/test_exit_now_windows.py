"""A completed windowless command returns its exit status to the caller."""
from tests.launcher_binary import run


def test_success_exits_cleanly():
    result = run("--check")
    assert result.returncode == 0, result.stderr
    assert "[built] check ok" in result.stdout


def test_invalid_arguments_keep_the_error_status():
    assert run("-p").returncode == 2
