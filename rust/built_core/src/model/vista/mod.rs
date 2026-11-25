//! Projected face fills for vector export. Geometry and visibility run in bulk;
//! only scene objects, progress callbacks and the plugin records cross Python.
use crate::{
    facesview::{self, Camera, FaceIn, FaceOut, Section},
    visible,
};
use numpy::{PyArray1, PyArrayMethods, PyReadonlyArray2};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
mod projection;
mod shadows;
use projection::*;
type Obj<'p> = Bound<'p, PyAny>;
const MIN_AREA: f64 = 0.01e-6;
fn array1<'p>(py: Python<'p>, v: &[f64]) -> PyResult<Obj<'p>> {
    Ok(PyArray1::from_vec(py, v.to_vec()).into_any())
}
fn array2<'p, const N: usize>(py: Python<'p>, v: &[[f64; N]]) -> PyResult<Obj<'p>> {
    Ok(PyArray1::from_vec(py, v.as_flattened().to_vec())
        .reshape([v.len(), N])?
        .into_any())
}
fn arrays<'p, const N: usize>(py: Python<'p>, v: &[Vec<[f64; N]>]) -> PyResult<Bound<'p, PyList>> {
    PyList::new(
        py,
        v.iter()
            .map(|r| array2(py, r))
            .collect::<PyResult<Vec<_>>>()?,
    )
}
fn read_rings(obj: &Obj<'_>) -> PyResult<Vec<visible::Ring>> {
    obj.try_iter()?
        .map(|ring| {
            let ring = ring?;
            if let Ok(array) = ring.extract::<PyReadonlyArray2<f64>>() {
                let view = array.as_array();
                if view.ncols() == 2 {
                    return Ok(view.rows().into_iter().map(|p| [p[0], p[1]]).collect());
                }
            }
            ring.extract()
        })
        .collect()
}
fn signed_area(r: &[[f64; 2]]) -> f64 {
    let (mut a, mut b) = (0., 0.);
    for (p, q) in r.iter().zip(r.iter().cycle().skip(1)) {
        a += p[0] * q[1];
        b += p[1] * q[0];
    }
    0.5 * (a - b)
}
#[pyfunction]
fn _area(r: Vec<[f64; 2]>) -> f64 {
    signed_area(&r)
}
fn light() -> P {
    unit([0.35, 0.25, 1.])
}
#[pyfunction]
fn shade_factor(normal: P) -> f64 {
    let len = dot(normal, normal).sqrt();
    if len < 1e-12 {
        return 0.84375;
    }
    ((0.62 + 0.38 * (dot(normal, light()).abs() / len)) * 32.).round_ties_even() / 32.
}
#[pyfunction]
fn shaded<'p>(py: Python<'p>, rgb: Vec<f64>, factor: f64) -> PyResult<Bound<'p, PyTuple>> {
    PyTuple::new(
        py,
        rgb.iter()
            .take(3)
            .map(|c| ((c * factor).min(1.) * 64.).round_ties_even() / 64.),
    )
}
#[pyfunction(signature=(path,cache=None))]
fn texture_average<'p>(path: &Obj<'p>, cache: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let py = path.py();
    let cache = match cache {
        Some(c) => c.clone(),
        None => py.import("built_core.vista_caras")?.getattr("_texture_cache")?,
    };
    if cache.contains(path)? {
        return cache.get_item(path);
    }
    let image = py
        .import("built_render")?
        .getattr("SurveyRaster")?
        .call_method1("open", (path,))?;
    let out = if image.is_none() {
        image
    } else {
        image.call_method0("mean_rgb")?
    };
    cache.set_item(path, &out)?;
    Ok(out)
}
type Item<'p> = (Obj<'p>, Obj<'p>, Obj<'p>, Obj<'p>);
#[pyfunction]
fn _faces_of<'p>(scene: &Obj<'p>) -> PyResult<Vec<Item<'p>>> {
    let py = scene.py();
    let tags = py.import("built_core")?.getattr("tag_styles")?;
    let layers = py.import("built_core.layers")?;
    let layer_of = layers.getattr("layer_of")?;
    let default = layers.getattr("DEFAULT_LAYER")?;
    let none = py.None().into_bound(py);
    let style = scene.getattr_opt("display_style")?.unwrap_or(none.clone());
    let by_layer = style
        .getattr_opt("color_by_layer")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(false);
    let paint = |owner: &Obj<'p>| {
        if by_layer {
            tags.call_method1("layer_paint", (scene, owner))
        } else {
            Ok(none.clone())
        }
    };
    let loose = paint(&none)?;
    let mut out = Vec::new();
    for f in scene.getattr("loose_mesh")?.getattr("faces")?.try_iter()? {
        let f = f?;
        if scene.call_method1("entity_visible", (&f,))?.is_truthy()? {
            let layer = layer_of.call1((&f,))?;
            out.push((f, none.clone(), loose.clone(), layer));
        }
    }
    for g in scene.getattr("groups")?.try_iter()? {
        let g = g?;
        if !scene.call_method1("entity_visible", (&g,))?.is_truthy()?
            || g.getattr_opt("billboard")?
                .map(|v| v.is_truthy())
                .transpose()?
                .unwrap_or(false)
        {
            continue;
        }
        let owner = tags.call_method1("object_layer", (&g,))?;
        let top = paint(&g)?;
        for row in py
            .import("built_core")?
            .call_method1("group_iter_painted_placements", (&g,))?
            .try_iter()?
        {
            let (pg, m, inherited): (Obj, Obj, Obj) = row?.extract()?;
            for f in pg.getattr("mesh")?.getattr("faces")?.try_iter()? {
                let f = f?;
                let own = layer_of.call1((&f,))?;
                let layer = if own.eq(&default)? {
                    owner.clone()
                } else {
                    own
                };
                out.push((
                    f,
                    m.clone(),
                    if by_layer {
                        top.clone()
                    } else {
                        inherited.clone()
                    },
                    layer,
                ));
            }
        }
    }
    Ok(out)
}
fn geometry(items: &[Item<'_>], proj: &Projection, sp: &Obj<'_>) -> PyResult<Vec<Option<FaceOut>>> {
    let mut inputs = Vec::with_capacity(items.len());
    let mut matrices = Vec::new();
    let mut seen = std::collections::HashMap::new();
    for (face, m, _, _) in items {
        let mut rings = vec![];
        let append =
            |arr: Obj<'_>| -> PyResult<Vec<P>> { arr.try_iter()?.map(|v| xyz(&v?)).collect() };
        rings.push(append(face.getattr("vertices")?)?);
        let holes = face.getattr("holes")?;
        if !holes.is_none() {
            for h in holes.try_iter()? {
                rings.push(append(h?)?);
            }
        }
        let xform = if m.is_none() {
            None
        } else {
            let key = m.as_ptr() as usize;
            let idx = match seen.get(&key) {
                Some(&idx) => idx,
                None => {
                    let idx = matrices.len();
                    matrices.push(matrix(m)?);
                    seen.insert(key, idx);
                    idx
                }
            };
            Some(idx)
        };
        inputs.push(FaceIn { rings, xform });
    }
    let section = if sp.is_none() {
        None
    } else {
        let n = xyz(&sp.getattr("normal")?)?;
        Some(Section {
            n,
            c: dot(n, xyz(&sp.getattr("point")?)?),
        })
    };
    Ok(sp
        .py()
        .detach(|| facesview::faces_view(&inputs, &matrices, &proj.cam, section.as_ref())))
}
#[pyfunction]
fn _face_geometry<'p>(
    py: Python<'p>,
    items: Vec<Item<'p>>,
    proj: PyRef<'p, Projection>,
    sp: &Obj<'p>,
) -> PyResult<Bound<'p, PyList>> {
    let out = PyList::empty(py);
    for row in geometry(&items, &proj, sp)? {
        if let Some(g) = row {
            let depths = PyList::new(
                py,
                g.depths
                    .iter()
                    .map(|v| array1(py, v))
                    .collect::<PyResult<Vec<_>>>()?,
            )?;
            out.append((
                arrays(py, &g.world)?,
                arrays(py, &g.drawn)?,
                depths,
                array1(py, &g.normal)?,
                g.back,
                (!g.depth.is_nan()).then_some((g.near[0], g.near[1], g.near[2])),
            ))?;
        } else {
            out.append(py.None())?
        }
    }
    Ok(out)
}
#[pyfunction]
fn _visible_parts<'p>(py: Python<'p>, faces: Vec<Obj<'p>>) -> PyResult<Vec<Obj<'p>>> {
    let mut inputs = Vec::with_capacity(faces.len());
    let mut depths = Vec::new();
    for f in &faces {
        inputs.push(visible::Face {
            rings: read_rings(&f.getattr("rings")?)?,
            near: f.getattr("extra")?.get_item("near")?.extract()?,
            opaque: f.getattr("opacity")?.extract::<f64>()? >= 0.999,
        });
        depths.push(f.getattr("depth")?.extract::<f64>()?);
    }
    visible_fills(py, &faces, &inputs, &depths)
}
fn visible_fills<'p>(
    py: Python<'p>,
    faces: &[Obj<'p>],
    inputs: &[visible::Face],
    depths: &[f64],
) -> PyResult<Vec<Obj<'p>>> {
    if faces.is_empty() {
        return Ok(Vec::new());
    }
    let setup = visible::Setup::new(inputs, MIN_AREA, visible::GRID);
    let tick = progress(py)?;
    let mut order = Vec::new();
    for i in (0..faces.len()).step_by(64) {
        tick.call1((i as f64 / faces.len() as f64,))?;
        for (j, r) in py
            .detach(|| setup.chunk(i..(i + 64).min(inputs.len())))
            .into_iter()
            .enumerate()
        {
            if !r.is_empty() {
                faces[i + j].setattr("rings", arrays(py, &r)?)?;
                order.push(i + j)
            }
        }
    }
    order.sort_by(|&a, &b| {
        inputs[b].opaque.cmp(&inputs[a].opaque).then_with(|| {
            depths[b]
                .partial_cmp(&depths[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    Ok(order.into_iter().map(|i| faces[i].clone()).collect())
}
fn progress(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("built_core")?
        .getattr("progress")?
        .getattr("tick")
}
fn rgb(obj: &Obj<'_>) -> PyResult<Vec<f64>> {
    obj.try_iter()?.take(3).map(|v| v?.extract()).collect()
}
fn get<'p>(obj: &Obj<'p>, key: &str) -> PyResult<Obj<'p>> {
    obj.call_method1("get", (key,))
}
#[pyfunction(signature=(scene,camera,as_view=true,textures=true,materials=true,shadows=false))]
fn view_faces<'p>(
    scene: &Obj<'p>,
    camera: &Obj<'p>,
    as_view: bool,
    textures: bool,
    materials: bool,
    shadows: bool,
) -> PyResult<Vec<Obj<'p>>> {
    let py = scene.py();
    let none = py.None().into_bound(py);
    let style = scene.getattr_opt("display_style")?.unwrap_or(none.clone());
    let mode: String = style
        .getattr_opt("face_mode")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or_else(|| "textures".into());
    let flat = matches!(mode.as_str(), "hidden_line" | "monochrome");
    let lit = as_view && mode != "hidden_line";
    let mut front = match style.getattr_opt("front_color")? {
        Some(v) if v.is_truthy()? => rgb(&v)?,
        _ => vec![1.; 3],
    };
    let styles = py.import("built_core.style")?;
    if !flat {
        for (c, d) in front
            .iter_mut()
            .zip(rgb(&styles.getattr("DEFAULT_FACE_COLOR")?)?)
        {
            *c *= d
        }
    }
    let back_color = rgb(&styles.call_method1("effective_back_color", (&style, scene))?)?;
    let proj = Projection::new(camera)?;
    if !proj.ok {
        return Ok(Vec::new());
    }
    let sp = if scene
        .getattr_opt("show_section_cuts")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(true)
    {
        match scene.getattr_opt("active_section")? {
            Some(f) => f.call0()?,
            None => none.clone(),
        }
    } else {
        none.clone()
    };
    let items = _faces_of(scene)?;
    let geo = geometry(&items, &proj, &sp)?;
    let mut raw = Vec::new();
    let mut visibility = Vec::new();
    let mut depths = Vec::new();
    let mut casters = Vec::new();
    let tick = progress(py)?;
    let mats = py.import("built_core.materials")?;
    let class = py.import("built_core.vista_caras")?.getattr("FaceFill")?;
    for ((face, m, paint, layer), g) in items.into_iter().zip(geo) {
        tick.call0()?;
        let Some(g) = g else { continue };
        let mut attrs = face.getattr("attrs")?;
        if !m.is_none() && attrs.is_truthy()? && get(&attrs, "texture")?.is_truthy()? {
            attrs = crate::model::group_mesh::transformed_attrs(&attrs, &m)?.into_any();
        }
        let mut eff = if materials {
            mats.call_method1("effective_attrs", (attrs, paint))?
        } else {
            none.clone()
        };
        if !eff.is_truthy()? {
            eff = PyDict::new(py).into_any()
        }
        let opacity = get(&eff, "opacity")?;
        let opacity = if opacity.is_truthy()? {
            opacity.extract()?
        } else {
            1.
        };
        let world = arrays(py, &g.world)?;
        let n = g.normal;
        let back = g.back;
        let mut texture = none.clone();
        let mut color = if back {
            back_color.clone()
        } else {
            front.clone()
        };
        if !flat
            && materials
            && !(back && mats.call_method1("back_is_default", (&eff,))?.is_truthy()?)
        {
            let mut side = eff;
            if back {
                let own = get(&side, "back")?;
                if own.is_instance_of::<PyDict>() {
                    side = own
                }
            }
            let own = get(&side, "color")?;
            texture = get(&side, "texture")?;
            color = if own.is_none() {
                front.clone()
            } else {
                rgb(&own)?
            };
            if !texture.is_none() {
                let path = get(&texture, "path")?;
                if !path.is_truthy()? {
                    texture = none.clone()
                } else if own.is_none() {
                    let avg = texture_average(&path, None)?;
                    if !avg.is_none() {
                        color = rgb(&avg)?
                    }
                }
            }
        }
        let factor = if lit { shade_factor(n) } else { 1. };
        if g.depth.is_finite() {
            let mut image = none.clone();
            if textures && mode == "textures" && !texture.is_none() {
                let vec = py
                    .import("built_core")?
                    .getattr("Vec3")?
                    .call1((n[0], n[1], n[2]))?;
                let axes = py
                    .import("built_core.texture")?
                    .call_method1("face_uv_axes", (&texture, vec))?;
                if let Some(pins) = pins(&g.world[0], n, &proj, &axes)? {
                    let t = PyDict::new(py);
                    t.set_item("path", texture.get_item("path")?)?;
                    t.set_item("shade", factor)?;
                    t.set_item("uv", PyTuple::new(py, pins)?)?;
                    image = t.into_any();
                }
            }
            let extra = PyDict::new(py);
            extra.set_item("near", (g.near[0], g.near[1], g.near[2]))?;
            extra.set_item("world", &world)?;
            extra.set_item(
                "seen_normal",
                array1(py, &n.map(|x| if back { -x } else { x }))?,
            )?;
            let c = if lit {
                shaded(py, color, factor)?
            } else {
                PyTuple::new(py, color)?
            };
            // Preserve NumPy's pairwise mean, including the sort order of tied fills.
            let depth = array1(py, &g.depths.iter().flatten().copied().collect::<Vec<_>>())?
                .call_method0("mean")?
                .extract::<f64>()?;
            depths.push(depth);
            visibility.push(visible::Face {
                rings: g.drawn,
                near: g.near,
                opaque: opacity >= 0.999,
            });
            raw.push(class.call1((
                PyList::empty(py),
                c,
                layer,
                opacity,
                image,
                back,
                depth,
                extra,
            ))?);
        }
        if shadows && opacity >= 0.7 {
            casters.push(shadows::Caster {
                world: g.world,
                identity: world.unbind(),
            })
        }
    }
    let mut out = visible_fills(py, &raw, &visibility, &depths)?;
    if shadows {
        let sun = py
            .import("built_core.sun")?
            .call_method1("scene_sun_direction", (scene,))?;
        if !sun.is_none() {
            let darkness = scene
                .getattr_opt("shadows")?
                .unwrap_or(none)
                .getattr_opt("darkness")?
                .map(|v| v.extract::<f64>())
                .transpose()?
                .unwrap_or(0.55);
            if let Some(ground) = shadows::cast(
                py,
                &out,
                &casters,
                sun.extract()?,
                &proj,
                1. - darkness,
                &class,
            )? {
                out.insert(0, ground)
            }
        }
    }
    Ok(out)
}
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let dc = py.import("dataclasses")?;
    let fields = PyList::empty(py);
    for (name, ty) in [("rings", "list"), ("rgb", "tuple"), ("layer", "str")] {
        fields.append((name, ty))?
    }
    let field = |v: Obj<'_>| {
        let kw = PyDict::new(py);
        kw.set_item("default", v)?;
        dc.call_method("field", (), Some(&kw))
    };
    fields.append((
        "opacity",
        "float",
        field(1_f64.into_pyobject(py)?.into_any())?,
    ))?;
    fields.append(("texture", "dict | None", field(py.None().into_bound(py))?))?;
    fields.append((
        "back",
        "bool",
        field(false.into_pyobject(py)?.to_owned().into_any())?,
    ))?;
    fields.append((
        "depth",
        "float",
        field(0_f64.into_pyobject(py)?.into_any())?,
    ))?;
    let kw = PyDict::new(py);
    kw.set_item("default_factory", py.get_type::<PyDict>())?;
    fields.append(("extra", "dict", dc.call_method("field", (), Some(&kw))?))?;
    fields.append(("shadow", "list | None", field(py.None().into_bound(py))?))?;
    fields.append((
        "shadow_alpha",
        "float",
        field(0_f64.into_pyobject(py)?.into_any())?,
    ))?;
    let cls = dc.call_method1("make_dataclass", ("FaceFill", fields))?;
    cls.setattr("__module__", "built_core.vista_caras")?;
    m.add("FaceFill", cls)?;
    m.add("MATTE_LIGHT", array1(py, &light())?)?;
    m.add("SHADOW_OPACITY_MIN", 0.7)?;
    m.add("GROUND_LAYER", "SUELO")?;
    m.add("_texture_cache", PyDict::new(py))?;
    m.add_function(wrap_pyfunction!(parallel, m)?)?;
    m.add_function(wrap_pyfunction!(perspective, m)?)?;
    m.add_function(wrap_pyfunction!(_area, m)?)?;
    m.add_function(wrap_pyfunction!(shade_factor, m)?)?;
    m.add_function(wrap_pyfunction!(shaded, m)?)?;
    m.add_function(wrap_pyfunction!(texture_average, m)?)?;
    m.add_function(wrap_pyfunction!(view_rect, m)?)?;
    m.add_function(wrap_pyfunction!(view_faces, m)?)?;
    m.add_function(wrap_pyfunction!(_faces_of, m)?)?;
    m.add_function(wrap_pyfunction!(_face_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(_visible_parts, m)?)?;
    Ok(())
}
