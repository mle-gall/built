// SPDX-License-Identifier: GPL-3.0-or-later
//! Indexed OBJ records in first-corner order and named Wavefront material sidecars.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    collections::HashMap,
    fmt::Write,
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
#[derive(Default)]
struct Index<const N: usize> {
    keys: HashMap<[u64; N], usize>,
    rows: Vec<[f64; N]>,
}
impl<const N: usize> Index<N> {
    fn insert(&mut self, row: [f64; N], digits: usize) -> usize {
        let key = row.map(|v| {
            let v = if v.fract() == 0. {
                v
            } else {
                format!("{v:.digits$}").parse().unwrap_or(v)
            };
            if v == 0. {
                0
            } else {
                v.to_bits()
            }
        });
        *self.keys.entry(key).or_insert_with(|| {
            self.rows.push(row);
            self.rows.len()
        })
    }
}
struct Material<'p> {
    key: Obj<'p>,
    info: Bound<'p, PyDict>,
    color: [f64; 3],
    image: Option<PathBuf>,
    faces: String,
}
fn get<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    o.call_method1("get", (k,))
}
fn tile(tex: &Obj<'_>, key: &str) -> PyResult<f64> {
    let v = tex.call_method1("get", (key, 1.))?;
    if v.is_truthy()? {
        v.extract()
    } else {
        Ok(1.)
    }
}
fn uv_for<'p>(
    tex: &Obj<'p>,
    texture: &Bound<'p, PyModule>,
    flat: &Obj<'p>,
    pts: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let uvw = get(tex, "uvw")?;
    if uvw.is_truthy()? {
        crate::model::module(texture.py(), "built_core")?.call_method1("texture_affine_uv", (uvw, pts))
    } else {
        crate::model::module(texture.py(), "built_core")?.call_method1(
            "texture_planar_uv",
            (
                flat,
                pts,
                tile(tex, "sw")?,
                tile(tex, "sh")?,
                tex.call_method1("get", ("rot", 0.))?,
            ),
        )
    }
}
fn copy_texture(src: &Path, path: &Path, texture: &Bound<'_, PyModule>) {
    let Some(name) = src.file_name() else { return };
    let dest = path.with_file_name(name);
    if dest.canonicalize().ok() == src.canonicalize().ok() {
        return;
    }
    if texture
        .call_method1("is_image_file", (src,))
        .and_then(|o| o.is_truthy())
        .unwrap_or(false)
    {
        let _ = std::fs::copy(src, dest);
    }
}
#[pyfunction]
fn obj_save(py: Python<'_>, scene: &Obj<'_>, path: PathBuf) -> PyResult<()> {
    let helpers = py.import("built_core.formats.meshexport")?;
    let texture = py.import("built_core.texture")?;
    let smooth = helpers.call_method0("SmoothNormals")?;
    let ids = PyDict::new(py);
    let mut materials: Vec<Material> = Vec::new();
    let mut order = vec![];
    let kw = PyDict::new(py);
    kw.set_item("with_opacity", false)?;
    let (mut positions, mut normals, mut uvs) =
        (Index::default(), Index::default(), Index::default());
    for pair in helpers
        .call_method1("world_faces_painted", (scene,))?
        .try_iter()?
    {
        let (face, attrs): (Obj<'_>, Obj<'_>) = pair?.extract()?;
        let key = helpers.call_method("material_key", (&attrs,), Some(&kw))?;
        let tex = get(&attrs, "texture")?;
        let image = if tex.is_truthy()? {
            get(&tex, "path")?
        } else {
            py.None().into_bound(py)
        };
        let textured = image.is_truthy()?;
        let idx = if let Some(v) = ids.get_item(&key)? {
            v.extract::<usize>()?
        } else {
            let color = if textured {
                [1.; 3]
            } else {
                let col = get(&attrs, "color")?;
                if col.is_truthy()? {
                    [
                        col.get_item(0)?.extract()?,
                        col.get_item(1)?.extract()?,
                        col.get_item(2)?.extract()?,
                    ]
                } else {
                    [0.96, 0.95, 0.925]
                }
            };
            let index = materials.len();
            ids.set_item(&key, index)?;
            materials.push(Material {
                key,
                info: PyDict::new(py),
                color,
                image: if textured {
                    Some(image.extract()?)
                } else {
                    None
                },
                faces: String::new(),
            });
            index
        };
        let m = &mut materials[idx];
        let name = get(&attrs, "mat")?;
        if name.is_truthy()? && !m.info.contains("mat")? {
            m.info.set_item("mat", name)?;
        }
        let flat = smooth.call_method1("flat", (&face,))?;
        let corner = smooth.call_method1("corners", (&face,))?;
        let flat_normal = crate::math::xyz(&flat)?.to_array();
        let smooth_face = corner.is_truthy()?;
        for tri in helpers
            .call_method1("export_triangles", (&face,))?
            .try_iter()?
        {
            let tri = tri?;
            let uv = if textured {
                Some(uv_for(
                    &tex,
                    &texture,
                    &flat,
                    PyList::new(py, tri.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.as_any(),
                )?)
            } else {
                None
            };
            if m.faces.is_empty() {
                order.push(idx);
            }
            m.faces.push('f');
            for (i, p) in tri.try_iter()?.enumerate() {
                let p = p?;
                let vi = positions.insert(crate::math::xyz(&p)?.to_array(), 6);
                let n = if smooth_face {
                    crate::math::xyz(
                        &corner.call_method1("get", (p.call_method0("toTuple")?, &flat))?,
                    )?
                    .to_array()
                } else {
                    flat_normal
                };
                let ni = normals.insert(n, 5);
                if let Some(uv) = &uv {
                    let ti = uvs.insert(uv.get_item(i)?.extract::<[f64; 2]>()?, 6);
                    write!(m.faces, " {vi}/{ti}/{ni}").unwrap();
                } else {
                    write!(m.faces, " {vi}//{ni}").unwrap();
                }
            }
            m.faces.push('\n');
        }
    }
    let used = PyDict::new(py);
    for &i in &order {
        used.set_item(&materials[i].key, &materials[i].info)?;
    }
    let names = helpers.call_method1("export_names", (used,))?;
    let mtl_path = path.with_extension("mtl");
    let mut obj = format!(
        "# Built OBJ export\nmtllib {}\n",
        mtl_path.file_name().unwrap_or_default().to_string_lossy()
    );
    for [x, y, z] in positions.rows {
        writeln!(obj, "v {x:.6} {y:.6} {z:.6}").unwrap();
    }
    for [u, v] in uvs.rows {
        writeln!(obj, "vt {u:.6} {v:.6}").unwrap();
    }
    for [x, y, z] in normals.rows {
        writeln!(obj, "vn {x:.6} {y:.6} {z:.6}").unwrap();
    }
    let mut mtl = String::new();
    for i in order {
        let m = &materials[i];
        let name = names.get_item(&m.key)?.extract::<String>()?;
        let [r, g, b] = m.color;
        writeln!(mtl, "newmtl {name}\nKd {r:.4} {g:.4} {b:.4}").unwrap();
        if let Some(src) = &m.image {
            copy_texture(src, &path, &texture);
            writeln!(
                mtl,
                "map_Kd {}",
                src.file_name().unwrap_or_default().to_string_lossy()
            )
            .unwrap();
        }
        mtl.push('\n');
        writeln!(obj, "usemtl {name}").unwrap();
        obj.push_str(&m.faces);
    }
    py.detach(|| {
        std::fs::write(mtl_path, mtl)?;
        std::fs::write(path, obj)
    })?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(obj_save, m)?)
}
