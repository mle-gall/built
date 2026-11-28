// SPDX-License-Identifier: GPL-3.0-or-later
use super::{decode, label, Obj, Path, PyResult, PyValueError, Value};
use pyo3::{prelude::*, types::PyDict};
use std::collections::BTreeSet;

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn element(xml: &mut String, tag: &str, text: &str) {
    if !text.is_empty() {
        xml.push_str(&format!("<{tag}>{}</{tag}>", escape(text)));
    }
}
fn guid(py: Python<'_>, v: &Value) -> PyResult<String> {
    Ok(py
        .import("uuid")?
        .call_method1("UUID", (label(v, "guid"),))?
        .str()?
        .to_string())
}
pub(super) fn write(scene: &Obj<'_>, path: &Path, selected: &Value) -> PyResult<()> {
    let py = scene.py();
    let bcf = scene
        .getattr("plugin_data")?
        .call_method1("get", ("bcf", PyDict::new(py)))?;
    let bcf = decode(&bcf)?;
    let problems = decode(&py.import("core.ifc_identity")?.call_method1("junction_topics", (scene,))?)?;
    let mut missing: BTreeSet<_> = selected
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let filter = !missing.is_empty();
    let zip = py
        .import("zipfile")?
        .call_method1("ZipFile", (path, "w", 8))?;
    let write = |name: &str, xml: String| zip.call_method1("writestr", (name, xml)).map(|_| ());
    let result = (|| -> PyResult<()> {
        write("bcf.version", r#"<Version VersionId="3.0"/>"#.into())?;
        let mut types = BTreeSet::new();
        let mut statuses = BTreeSet::new();
        for t in bcf["topics"].as_array().into_iter().flatten().chain(problems.as_array().into_iter().flatten()) {
            if filter && !missing.remove(t["guid"].as_str().unwrap_or("")) {
                continue;
            }
            let id = guid(py, t)?;
            let kind = label(t, "topic_type");
            let status = label(t, "topic_status");
            let mut xml = format!(
                r#"<Markup><Topic Guid="{id}" TopicType="{}" TopicStatus="{}">"#,
                escape(&kind),
                escape(&status)
            );
            types.insert(kind);
            statuses.insert(status);
            element(&mut xml, "Title", &label(t, "title"));
            for (tag, field) in [
                ("CreationDate", "creation_date"),
                ("CreationAuthor", "creation_author"),
                ("ModifiedDate", "modified_date"),
                ("ModifiedAuthor", "modified_author"),
                ("AssignedTo", "assigned_to"),
                ("Description", "description"),
            ] {
                element(&mut xml, tag, &label(t, field));
            }
            if let Some(comments) = t["comments"].as_array().filter(|c| !c.is_empty()) {
                xml.push_str("<Comments>");
                for c in comments {
                    xml.push_str(&format!(r#"<Comment Guid="{}">"#, guid(py, c)?));
                    for (tag, key) in [
                        ("Date", "date"),
                        ("Author", "author"),
                        ("Comment", "comment"),
                    ] {
                        element(&mut xml, tag, &label(c, key));
                    }
                    xml.push_str("</Comment>");
                }
                xml.push_str("</Comments>");
            }
            if let Some(views) = t["viewpoints"].as_array().filter(|v| !v.is_empty()) {
                xml.push_str("<Viewpoints>");
                for v in views {
                    let vid = guid(py, v)?;
                    let mut vis = format!(r#"<VisualizationInfo Guid="{vid}">"#);
                    if let Some(selection) = v["components"]["selection"]
                        .as_array()
                        .filter(|s| !s.is_empty())
                    {
                        vis.push_str("<Components><Selection>");
                        for item in selection {
                            // BCF's IfcGuid: the placement's GlobalId, the one
                            // the IFC export writes (docs/persistent-ids.md §7.2).
                            let mut ifc = label(item, "ifc_guid");
                            let tool = label(item, "authoring_tool_id");
                            if ifc.is_empty() && !tool.is_empty() {
                                ifc = py
                                    .import("core.ifc_identity")
                                    .and_then(|m| m.call_method1("component_guid", (scene, tool.as_str())))
                                    .and_then(|v| v.extract::<Option<String>>())
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default();
                            }
                            if ifc.is_empty() {
                                vis.push_str("<Component>");
                            } else {
                                vis.push_str(&format!(r#"<Component IfcGuid="{}">"#, escape(&ifc)));
                            }
                            element(
                                &mut vis,
                                "OriginatingSystem",
                                &label(item, "originating_system"),
                            );
                            element(
                                &mut vis,
                                "AuthoringToolId",
                                &label(item, "authoring_tool_id"),
                            );
                            vis.push_str("</Component>");
                        }
                        vis.push_str("</Selection></Components>");
                    }
                    let ortho = v["orthogonal_camera"].is_object();
                    let (cam, tag) = if ortho {
                        (&v["orthogonal_camera"], "OrthogonalCamera")
                    } else {
                        (&v["perspective_camera"], "PerspectiveCamera")
                    };
                    vis.push_str(&format!("<{tag}>"));
                    for (tag, k) in [
                        ("CameraViewPoint", "camera_view_point"),
                        ("CameraDirection", "camera_direction"),
                        ("CameraUpVector", "camera_up_vector"),
                    ] {
                        vis.push_str(&format!("<{tag}>"));
                        for axis in ["x", "y", "z"] {
                            element(&mut vis, &axis.to_uppercase(), &cam[k][axis].to_string());
                        }
                        vis.push_str(&format!("</{tag}>"));
                    }
                    if ortho {
                        element(
                            &mut vis,
                            "ViewToWorldScale",
                            &cam["view_to_world_scale"].to_string(),
                        );
                    } else {
                        element(&mut vis, "FieldOfView", &cam["field_of_view"].to_string());
                    }
                    element(&mut vis, "AspectRatio", &cam["aspect_ratio"].to_string());
                    vis.push_str(&format!("</{tag}></VisualizationInfo>"));
                    write(&format!("{id}/{vid}.bcfv"), vis)?;
                    xml.push_str(&format!(
                        r#"<ViewPoint Guid="{vid}"><Viewpoint>{vid}.bcfv</Viewpoint></ViewPoint>"#
                    ));
                }
                xml.push_str("</Viewpoints>");
            }
            xml.push_str("</Topic></Markup>");
            write(&format!("{id}/markup.bcf"), xml)?;
        }
        if !missing.is_empty() {
            return Err(PyValueError::new_err("Selected BCF topic is missing"));
        }
        let mut ext = String::from("<Extensions>");
        for (tag, values) in [("TopicType", types), ("TopicStatus", statuses)] {
            if values.is_empty() {
                continue;
            }
            let plural = if tag == "TopicType" {
                "TopicTypes"
            } else {
                "TopicStatuses"
            };
            ext.push_str(&format!("<{plural}>"));
            for value in values {
                element(&mut ext, tag, &value);
            }
            ext.push_str(&format!("</{plural}>"));
        }
        ext.push_str("</Extensions>");
        write("extensions.xml", ext)
    })();
    zip.call_method0("close")?;
    if result.is_err() {
        std::fs::remove_file(path)?;
    }
    result
}
