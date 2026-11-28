"""Dated handover issues, legacy records and independent writer outcomes."""
import datetime
import hashlib
import json
from pathlib import Path
import uuid
import xml.etree.ElementTree as ET
import zipfile

import built_core
from built_core import deliveries
from built_core.composition import Composicion
from built_core.history import History
import pytest
from tests.test_ifc_export import _tagged_wall_scene


def preset(destination, **fields):
    values = dict(name="Site A", format="package", scope="whole", destination=str(destination))
    return json.loads(built_core.delivery_record(json.dumps({**values, **fields})))


def test_legacy_delivery_migrates_without_changing_target(tmp_path):
    scene = _tagged_wall_scene()
    old = dict(name="Mesh", format="obj", destination=str(tmp_path / "model.obj"))
    scene.plugin_data["deliveries"] = {"old": old}
    assert deliveries.records(scene)["old"]["outputs"] == [
        dict(format="obj", scope="whole", options={}, destination=old["destination"])]
    h = History(scene)
    deliveries.edit_delivery(h, "old", {"scope": "visible"})
    assert scene.plugin_data["deliveries"]["old"]["outputs"][0]["scope"] == "visible"
    deliveries.run_delivery(h, "old")
    assert Path(old["destination"]).is_file()


def test_every_writer_and_dated_manifest(tmp_path, monkeypatch):
    scene = _tagged_wall_scene()
    scene.compositions = [Composicion(name="Plan"), Composicion(name="Roof")]
    topic = str(uuid.uuid4())
    scene.plugin_data["bcf"] = {"version": "3.0", "topics": [{
        "guid": topic, "title": "A & B", "topic_type": "Agent task", "topic_status": "Done",
        "creation_author": "Architect", "creation_date": "2026-01-02T01:00:00Z",
        "comments": [{"guid": str(uuid.uuid4()), "author": "Reviewer",
                      "date": "2026-01-03T01:00:00Z", "comment": "Issued"}]}]}
    h = History(scene)
    setup = preset("Deliveries", sheet_names="Plan", ids="rules.ids")
    key = deliveries.create_delivery(h, setup)["key"]
    revision = h.revision
    pdf_calls = []

    def pdf(path, sheets):
        pdf_calls.append([s.name for s in sheets])
        Path(path).write_bytes(b"%PDF-1.7")

    ids = [{"status": "checked", "specifications": [
        {"name": "Wall status", "status": False, "failed": 2, "passed": 0}]}]

    def check(path, files, directory):
        assert files == ["rules.ids"] and directory == tmp_path
        assert "IFC4" in path.read_text()
        path.with_suffix(".ids-report.json").write_text(json.dumps(ids))
        return ids

    monkeypatch.setattr("formats.ifc.export.check_ids", check)
    for _ in range(2):
        deliveries.run_delivery(h, key, project_directory=tmp_path, pdf_writer=pdf)
    first, second = deliveries.records(scene)[key]["runs"]
    folder = Path(first["folder"])
    assert folder == tmp_path.resolve() / "Deliveries" / f"{datetime.date.today()}_Site A"
    assert Path(second["folder"]) == folder.with_name(folder.name + "_2")
    assert pdf_calls == [["Plan"], ["Plan"]]
    assert first["ids_result"] == "1 failed rule" and first["result"] == "Needs review"
    assert first["outputs"][0] == {"format": "ifc", "result": "1 failed rule"}
    assert [r["result"] for r in first["outputs"][1:]] == ["Delivered"] * 5
    assert first["ids"] == json.loads((folder / "model.ids-report.json").read_text()) == ids
    manifest = json.loads((folder / "manifest.json").read_text())
    assert manifest["revision"] == first["revision"] == revision
    assert manifest["date"] == str(datetime.date.today()) and manifest["ids"] == ids
    inventory = {f["path"] for f in manifest["files"]}
    assert inventory == {"model.ifc", "model.ids-report.json", "sheets.pdf",
                         "drawings.dxf", "topics.bcfzip", "schedule.csv"}
    text = (folder / "manifest.txt").read_text()
    for f in manifest["files"]:
        payload = (folder / f["path"]).read_bytes()
        assert f["size"] == len(payload)
        assert f["sha256"] == hashlib.sha256(payload).hexdigest()
        assert f["path"] in text and f["sha256"] in text
    assert "IfcWall" in (folder / "schedule.csv").read_text()
    assert "EOF" in (folder / "drawings.dxf").read_text()
    with zipfile.ZipFile(folder / "topics.bcfzip") as archive:
        assert ET.fromstring(archive.read("bcf.version")).get("VersionId") == "3.0"
        markup = ET.fromstring(archive.read(topic + "/markup.bcf"))
        assert markup.findtext("Topic/Title") == "A & B"
        assert markup.findtext("Topic/Comments/Comment/Comment") == "Issued"


def test_writer_failure_and_cancellation_leave_a_consistent_folder(tmp_path):
    scene = _tagged_wall_scene()
    setup = preset(tmp_path)
    steps = deliveries.write_steps(scene, setup)
    assert next(steps) == (0., "Writing IFC")
    next(steps)
    steps.close()
    assert list(tmp_path.iterdir()) == []
    steps = deliveries.write_steps(scene, setup)
    list(steps)
    folder = Path(steps.summary["folder"])
    assert not (folder / "sheets.pdf").exists()
    assert steps.summary["result"] == "Needs review"
    assert steps.summary["outputs"][1]["result"].startswith("Failed")
    assert (folder / "schedule.csv").exists()
    assert len(list(tmp_path.iterdir())) == 1


def test_reject_empty_package(tmp_path):
    setup = preset(tmp_path)
    setup["outputs"] = []
    with pytest.raises(ValueError, match="Select at least one output"):
        deliveries.validate(setup)


def test_setup_sheet_run_and_review_share_delivery(native_host, tmp_path, monkeypatch):
    import built_render
    from built_render import chrome_sheet
    from unittest.mock import Mock

    win = native_host.window
    view = native_host.view
    monkeypatch.setattr(view, "renderer", Mock())
    scene = win.viewport.scene
    form = built_render.delivery_export_form(win, initial_format="package")
    form.set("destination", str(tmp_path / "output"))
    sheet = chrome_sheet
    sheet.before_frame(view, win)
    sheet.action(win, sheet.SEP.join(["delivery", "delivery-toggle", "csv"]))
    assert form.value("include_csv") is False
    form.press("run")
    for _ in range(30):
        if not win._delivery_queue:
            break
        win._host.idle()
    assert not win._delivery_queue
    key, delivery = next(iter(deliveries.records(scene).items()))
    assert "csv" not in [o["format"] for o in delivery["outputs"]]
    result = delivery["runs"][-1]
    assert result["result"] == "Needs review"
    assert result["folder"] == str(next((tmp_path / "output").iterdir()).resolve())
    built_render.delivery_action(win, view, f"review:{key}:0")
    assert view._delivery_review == result
    built_render.delivery_action(win, view, "back")
    assert view._delivery_review is None


def test_step_output_can_be_added_to_existing_package(tmp_path):
    from formats import step

    scene = _tagged_wall_scene()
    plan = preset(tmp_path, include_step=True)
    steps = deliveries.write_steps(scene, plan)
    list(steps)
    folder = Path(steps.summary["folder"])
    assert {"format": "step", "result": "Delivered"} in steps.summary["outputs"]
    assert step.prepare_step(folder / "model.step")["exact"] > 0


def test_amber_junction_topic_uses_exported_identity(tmp_path):
    from core.ifc_identity import junction_topics, component_guid
    from tests.test_live_junction import setup

    scene, history, main, _, uid = setup()
    assert junction_topics(scene) == []
    from tests.test_live_recipes import apply
    apply(history, "delete_placements", placements=[main.uid])
    topic, = junction_topics(scene)
    assert "lost" in topic["title"]
    selection, = topic["viewpoints"][0]["components"]["selection"]
    assert selection["authoring_tool_id"].startswith("built:")
    assert selection["ifc_guid"] == component_guid(scene, selection["authoring_tool_id"])
    assert topic["guid"] == junction_topics(scene)[0]["guid"]
    config = preset(str(tmp_path), include_pdf=False, include_ifc=False, include_dxf=False,
                    include_csv=False, include_manifest=False)
    key = deliveries.create_delivery(history, config)["key"]
    deliveries.run_delivery(history, key)
    folder = Path(deliveries.records(scene)[key]["runs"][-1]["folder"])
    with zipfile.ZipFile(folder / "topics.bcfzip") as archive:
        viewpoint = next(n for n in archive.namelist() if n.endswith(".bcfv"))
        component = ET.fromstring(archive.read(viewpoint)).find(".//Component")
        assert component.attrib["IfcGuid"] == selection["ifc_guid"]
        assert component.findtext("AuthoringToolId") == selection["authoring_tool_id"]
