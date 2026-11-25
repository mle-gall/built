// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
const KEYS: [&str; 6] = ["text", "font", "bold", "italic", "height", "thickness"];
fn text_value(v: &Obj) -> PyResult<String> {
    v.str()?.extract()
}
fn font_value(v: &Obj) -> PyResult<String> {
    if v.is_truthy()? {
        text_value(v)
    } else {
        Ok(String::new())
    }
}
fn flag(v: &Obj) -> PyResult<bool> {
    v.is_truthy()
}
fn dimension(v: &Obj) -> PyResult<f64> {
    v.py()
        .import("builtins")?
        .call_method1("float", (v,))?
        .extract()
}
#[pyfunction]
#[pyo3(signature=(text,font=String::new(),bold=true,italic=false,height=0.25,thickness=0.05))]
fn text_params<'p>(
    py: Python<'p>,
    #[pyo3(from_py_with = text_value)] text: String,
    #[pyo3(from_py_with = font_value)] font: String,
    #[pyo3(from_py_with = flag)] bold: bool,
    #[pyo3(from_py_with = flag)] italic: bool,
    #[pyo3(from_py_with = dimension)] height: f64,
    #[pyo3(from_py_with = dimension)] thickness: f64,
) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("text", text)?;
    out.set_item("font", font)?;
    out.set_item("bold", bold)?;
    out.set_item("italic", italic)?;
    out.set_item("height", height)?;
    out.set_item("thickness", thickness)?;
    Ok(out)
}
fn letter_groups<'p>(params: &Obj<'p>, frame: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyList>> {
    let py = params.py();
    let group = py.import("core.group")?.getattr("Group")?;
    let matrix = py.import("built_core")?.getattr("Matrix4")?;
    let text = params.get_item("text")?.extract::<String>()?;
    let font = params
        .call_method1("get", ("font", ""))?
        .extract::<String>()?;
    let bold = params.call_method1("get", ("bold", true))?.is_truthy()?;
    let italic = params.call_method1("get", ("italic", false))?.is_truthy()?;
    let height = params.call_method1("get", ("height", 0.25))?.extract()?;
    let thickness = params.call_method1("get", ("thickness", 0.05))?.extract()?;
    let kids = PyList::empty(py);
    for (label, mesh) in build_text_letters(py, &text, &font, bold, italic, height, thickness)? {
        let kw = PyDict::new(py);
        kw.set_item("name", label)?;
        let g = group.call((mesh,), Some(&kw))?;
        let m = if let Some(frame) = frame {
            matrix.call1((frame,))?
        } else {
            matrix.call0()?
        };
        g.setattr("xform", m)?;
        kids.append(g)?;
    }
    Ok(kids)
}
// The text hash is serialized in saved drawings. Match Python's decimal rounding,
// JSON spacing, key order and Unicode escaping, including drawings from older versions.
#[pyfunction]
fn letters_fingerprint(children: &Obj<'_>) -> PyResult<String> {
    let py = children.py();
    let parts = PyList::empty(py);
    let builtin = py.import("builtins")?;
    let round = builtin.getattr("round")?;
    let dumps = py.import("json")?.getattr("dumps")?;
    let sort = PyDict::new(py);
    sort.set_item("sort_keys", true)?;
    for child in children.try_iter()? {
        let child = child?;
        let mesh = child.getattr("mesh")?;
        let cloud = PyList::empty(py);
        for v in mesh.getattr("vertices")?.try_iter()? {
            let p = xyz(&v?.getattr("position")?)?;
            cloud.append((
                round.call1((p.x, 4))?,
                round.call1((p.y, 4))?,
                round.call1((p.z, 4))?,
            ))?;
        }
        cloud.call_method0("sort")?;
        let faces = mesh.getattr("faces")?;
        let paint = PyList::empty(py);
        for f in faces.try_iter()? {
            let attrs = f?.getattr("attrs")?;
            let fields = PyDict::new(py);
            let color = attrs.call_method1("get", ("color",))?;
            if color.is_none() {
                fields.set_item("color", py.None())?;
            } else {
                let channels = PyList::empty(py);
                for c in color.try_iter()? {
                    channels.append(round.call1((builtin.call_method1("float", (c?,))?, 3))?)?;
                }
                fields.set_item("color", channels)?;
            }
            fields.set_item("mat", attrs.call_method1("get", ("mat",))?)?;
            for k in ["texture", "back"] {
                fields.set_item(k, !attrs.call_method1("get", (k,))?.is_none())?;
            }
            paint.append(dumps.call((fields,), Some(&sort))?)?;
        }
        paint.call_method0("sort")?;
        parts.append((
            child.getattr("name")?,
            faces.len()?,
            mesh.getattr("edges")?.len()?,
            cloud,
            paint,
        ))?;
    }
    let bytes = dumps.call1((parts,))?.call_method0("encode")?;
    let digest = py
        .import("hashlib")?
        .call_method1("sha1", (bytes,))?
        .call_method0("hexdigest")?
        .extract::<String>()?;
    Ok(digest[..16].to_owned())
}
#[pyfunction]
fn text_state<'p>(params: &Obj<'p>, children: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let state = PyDict::new(params.py());
    for key in KEYS {
        if params.contains(key)? {
            state.set_item(key, params.get_item(key)?)?;
        }
    }
    state.set_item("hash", letters_fingerprint(children)?)?;
    Ok(state)
}
fn children<'p>(g: &Obj<'p>) -> PyResult<Obj<'p>> {
    Ok(g.getattr_opt("children")?
        .filter(|k| !k.is_none())
        .unwrap_or_else(|| PyTuple::empty(g.py()).into_any()))
}
#[pyfunction]
fn text_frame<'p>(group: &Obj<'p>) -> PyResult<Obj<'p>> {
    let matrix = group.py().import("built_core")?.getattr("Matrix4")?;
    for kid in children(group)?.try_iter()? {
        let m = kid?.getattr("xform")?;
        if !m.is_none() {
            return matrix.call1((m,));
        }
    }
    matrix.call0()
}
#[pyfunction]
fn text_is_pristine(group: &Obj<'_>) -> PyResult<bool> {
    let Some(state) = group.getattr_opt("text3d")?.filter(|s| !s.is_none()) else {
        return Ok(true);
    };
    let hash = state.call_method1("get", ("hash",))?;
    if !hash.is_truthy()? {
        return Ok(true);
    }
    let kids = children(group)?;
    if hash.ne(letters_fingerprint(&kids)?)? {
        return Ok(false);
    }
    let frame = text_frame(group)?;
    let matrix = group.py().import("built_core")?.getattr("Matrix4")?;
    for kid in kids.try_iter()? {
        let mut m = kid?.getattr("xform")?;
        if m.is_none() {
            m = matrix.call0()?;
        }
        if m.ne(&frame)? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn make_text_group<'p>(params: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let py = params.py();
    let kids = letter_groups(params, None)?;
    if kids.is_empty() {
        return Ok(None);
    }
    let name = params
        .get_item("text")?
        .extract::<String>()?
        .trim()
        .chars()
        .take(24)
        .collect::<String>();
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    let group = py
        .import("core.group")?
        .call_method("Group", (), Some(&kw))?;
    group.call_method1("adopt", (&kids,))?;
    group.setattr("component", false)?;
    group.setattr("text3d", text_state(params, kids.as_any())?)?;
    Ok(Some(group))
}
#[pyfunction]
fn rebuild_text_group<'p>(group: &Obj<'p>, params: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let frame = text_frame(group)?;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for kid in children(group)?.try_iter()? {
        for v in kid?.getattr("mesh")?.getattr("vertices")?.try_iter()? {
            let y = xyz(&v?.getattr("position")?)?.y;
            low = low.min(y);
            high = high.max(y);
        }
    }
    if low > -1e-9 && high > 1e-9 {
        frame.call_method1("translate", (0., high, 0.))?;
    }
    letter_groups(params, Some(&frame))
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add("TEXT_KEYS", PyTuple::new(m.py(), KEYS)?)?;
    for f in [
        wrap_pyfunction!(text_params, m)?,
        wrap_pyfunction!(letters_fingerprint, m)?,
        wrap_pyfunction!(text_state, m)?,
        wrap_pyfunction!(text_frame, m)?,
        wrap_pyfunction!(text_is_pristine, m)?,
        wrap_pyfunction!(make_text_group, m)?,
        wrap_pyfunction!(rebuild_text_group, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
