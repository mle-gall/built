"""Public wire records and command defaults captured before native registration."""
import json
from pathlib import Path
from types import SimpleNamespace

from built_core import bridge, link_structure, material_commands
from built_core.protocol import commands, feed, keys, records


def observations():
    step = feed.Step(17, "undo", "Move")
    step.tags["modified"].append("Walls")
    group = SimpleNamespace(uid="g", ifc={"class": "IfcWall", "name": "W"},
                            locked=False, ext={})
    return {
        "commands": list(commands.COMMANDS),
        "step": step.to_json(),
        "keys": [keys.to_json_key(2**63+1), keys.from_json_key("42")],
        "occurrence": records.occurrence(group, {}),
        "linked": records.occurrence(group, {}, linked=True),
        "row": link_structure.row_id(["A", "B/é"]),
        "mount": link_structure.mount_key(["A", "B"]),
        "points": bridge.Profile([[1, 2], [3]], True).points,
        "paint": [material_commands.colour_name((0.5, -1, 2)),
                  material_commands.fresh_name({"A": 1, "A (2)": 1}, "A")],
    }


def test_modules_match_python():
    assert observations() == json.loads((Path(__file__).parent / "fixtures" /
                                         "commands_modules_python.json").read_text())


def test_first_import_can_be_the_tree_command_dispatcher():
    import subprocess
    import sys
    from textwrap import dedent

    subprocess.run([sys.executable, "-c", dedent("""
        from built_core.protocol.commands import COMMANDS, run
        from built_core.protocol import commands
        from built_core import command_protocol
        assert commands.__spec__.origin == 'built_core'
        assert run is command_protocol.run
        assert COMMANDS['set_tag_state'] is command_protocol.set_tag_state
        from built_core.history import History
        from built_core.scene import Scene
        assert run(History(Scene()), 'undo')['step'] is None
    """)], check=True)
