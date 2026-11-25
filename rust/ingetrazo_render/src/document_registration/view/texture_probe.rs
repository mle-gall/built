// SPDX-License-Identifier: GPL-3.0-or-later
//! Measure preview updates and completed frames on one textured placement.
use super::*;
use std::time::Instant;

fn first_textured<'py>(scene: &Obj<'py>, tool: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = scene.py();
    let placements = py.import("core.group")?.getattr("iter_placements")?;
    for root in scene.getattr("groups")?.try_iter()? {
        for placement in placements.call1((root?,))?.try_iter()? {
            let group = placement?.get_item(0)?;
            for face in group.getattr("mesh")?.getattr("faces")?.try_iter()? {
                let face = face?;
                if tool
                    .call_method1("side_texture", (&face, "front"))?
                    .is_truthy()?
                {
                    return Ok((group, face));
                }
            }
        }
    }
    Err(pyo3::exceptions::PyStopIteration::new_err(()))
}
#[pyfunction]
pub(super) fn measure_texture_position(py: Python<'_>, path: &Obj) -> PyResult<()> {
    let path = py.import("pathlib")?.call_method1("Path", (path,))?;
    let scene = py.import("core.scene")?.call_method0("Scene")?;
    py.import("formats.igz")?
        .call_method1("load_into", (&scene, &path))?;
    let tool = py
        .import("tools.texture_position")?
        .getattr("TexturePositionTool")?;
    let (group, face) = first_textured(&scene, &tool)?;
    scene.call_method1("begin_group_edit", (&group,))?;
    let [app, view] = unpack(
        &py.import("tools._classic")?
            .call_method1("measure_view", (&scene,))?,
    )?;
    let measure = || -> PyResult<()> {
        let camera = view.getattr("camera")?;
        camera.setattr("target", face.call_method0("centroid")?)?;
        camera.setattr("distance", 5)?;
        view.call_method0("show")?;
        app.call_method0("processEvents")?;
        let tool = tool.call0()?;
        tool.call_method1("begin", (&view, &face, face.call_method0("centroid")?))?;
        let preview = py
            .import("tools.texture_position_metal")?
            .call_method1("TexturePreview", (&view,))?;
        let frames = || -> PyResult<()> {
            let samples = PyList::empty(py);
            for _ in 0..10 {
                let map = tool.getattr("map")?;
                map.setattr("R", map.getattr("R")?.add(map.getattr("e_u")?.mul(0.01)?)?)?;
                let start = Instant::now();
                preview.call_method1("update", (&tool,))?;
                let upload = start.elapsed().as_secs_f64() * 1000.;
                view.call_method0("draw_frame")?;
                view.call_method1("pick_row", (0, 0))?;
                samples.append(fields!(py;"bake_upload_ms"=>upload,"through_frame_ms"=>start.elapsed().as_secs_f64()*1000.))?;
            }
            let result = fields!(py;"model"=>path.getattr("name")?,"group"=>group.getattr("name")?,
                "faces"=>group.getattr("mesh")?.getattr("faces")?.len()?,"placements"=>view.getattr("packed")?.getattr("instances")?.len()?,
                "overlay_only"=>preview.getattr("overlay_only")?,"samples"=>samples);
            let json = py.import("json")?.call_method(
                "dumps",
                (result,),
                Some(&fields!(py;"indent"=>2)),
            )?;
            py.import("builtins")?.call_method1("print", (json,))?;
            Ok(())
        };
        let result = frames();
        preview.call_method0("restore")?;
        result
    };
    let result = measure();
    view.call_method0("close")?;
    result
}
