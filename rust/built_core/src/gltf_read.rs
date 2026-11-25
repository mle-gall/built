// SPDX-License-Identifier: GPL-3.0-or-later
//! glTF container, scene traversal and typed triangle accessors.
use glam::{DMat4, DQuat, DVec3};
use numpy::{
    ndarray::{Array2, Array3},
    IntoPyArray,
};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple},
};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

type Obj<'py> = Bound<'py, PyAny>;
fn invalid(msg: impl Into<String>) -> PyErr {
    PyValueError::new_err(msg.into())
}
fn items(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn index(v: &Value) -> PyResult<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("invalid glTF index"))
}
fn at<'a>(arr: &'a Value, i: &Value) -> PyResult<&'a Value> {
    arr.get(index(i)?)
        .ok_or_else(|| invalid("glTF index out of range"))
}
fn numbers<const N: usize>(v: &Value) -> PyResult<[f64; N]> {
    serde_json::from_value::<Vec<f64>>(v.clone())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| invalid("invalid glTF vector"))
}
fn pyvalue<'py>(py: Python<'py>, v: &Value) -> PyResult<Obj<'py>> {
    match v {
        Value::Null => Ok(py.None().into_bound(py)),
        Value::Bool(b) => Ok(b.into_pyobject(py)?.to_owned().into_any()),
        Value::String(s) => Ok(s.into_pyobject(py)?.into_any()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_pyobject(py)?.into_any())
            } else if let Some(u) = n.as_u64() {
                Ok(u.into_pyobject(py)?.into_any())
            } else {
                Ok(n.as_f64()
                    .ok_or_else(|| invalid("invalid glTF number"))?
                    .into_pyobject(py)?
                    .into_any())
            }
        }
        Value::Array(arr) => {
            let out = PyList::empty(py);
            for item in arr {
                out.append(pyvalue(py, item)?)?
            }
            Ok(out.into_any())
        }
        Value::Object(map) => {
            let out = PyDict::new(py);
            for (k, item) in map {
                out.set_item(k, pyvalue(py, item)?)?
            }
            Ok(out.into_any())
        }
    }
}
fn json_value(obj: &Obj<'_>) -> PyResult<Value> {
    let text: String = obj
        .py()
        .import("json")?
        .call_method1("dumps", (obj,))?
        .extract()?;
    serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))
}
fn uri(py: Python<'_>, path: &Path, uri: &str) -> PyResult<Vec<u8>> {
    if uri.starts_with("data:") {
        let (_, data) = uri
            .split_once(',')
            .ok_or_else(|| invalid("invalid data URI"))?;
        Ok(py
            .import("base64")?
            .call_method1("b64decode", (data,))?
            .cast::<PyBytes>()?
            .as_bytes()
            .to_vec())
    } else {
        Ok(py.detach(|| fs::read(path.with_file_name(uri)))?)
    }
}
struct Document {
    js: Value,
    buffers: Vec<Vec<u8>>,
}
fn chunks(data: &[u8]) -> PyResult<(&[u8], &[u8])> {
    if !data.starts_with(b"glTF") {
        return Ok((data, &[]));
    }
    let mut rem = data
        .get(12..)
        .ok_or_else(|| invalid("truncated GLB header"))?;
    let (mut json, mut bin) = (None, &[][..]);
    while let Some(h) = rem.get(..8) {
        let n = u32::from_le_bytes(h[..4].try_into().unwrap()) as usize;
        let chunk = rem
            .get(8..8 + n)
            .ok_or_else(|| invalid("truncated GLB chunk"))?;
        match &h[4..8] {
            b"JSON" => json = Some(chunk),
            b"BIN\0" => bin = chunk,
            _ => {}
        }
        rem = rem.get(8 + n.next_multiple_of(4)..).unwrap_or_default();
    }
    Ok((
        json.ok_or_else(|| invalid("GLB without a JSON chunk"))?,
        bin,
    ))
}
impl Document {
    fn open(py: Python<'_>, path: &Path) -> PyResult<Self> {
        let (js, bin) = py.detach(|| -> PyResult<_> {
            let bytes = fs::read(path)?;
            let (text, bin) = chunks(&bytes)?;
            let js: Value = serde_json::from_slice(text).map_err(|e| invalid(e.to_string()))?;
            Ok((js, bin.to_vec()))
        })?;
        let mut buffers = Vec::new();
        for buf in items(&js["buffers"]) {
            buffers.push(if let Some(u) = buf["uri"].as_str() {
                uri(py, path, u)?
            } else {
                bin.clone()
            })
        }
        Ok(Self { js, buffers })
    }
    fn view(&self, id: &Value) -> PyResult<(&Value, &[u8])> {
        let view = at(&self.js["bufferViews"], id)?;
        let blob = self
            .buffers
            .get(view["buffer"].as_u64().unwrap_or(0) as usize)
            .ok_or_else(|| invalid("missing glTF buffer"))?;
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = index(&view["byteLength"])?;
        let slice = start
            .checked_add(len)
            .and_then(|end| blob.get(start..end))
            .ok_or_else(|| invalid("invalid glTF buffer view"))?;
        Ok((view, slice))
    }
    fn accessor(&self, id: &Value) -> PyResult<Accessor<'_>> {
        let a = at(&self.js["accessors"], id)?;
        if a.get("sparse").is_some() {
            return Err(invalid("sparse accessors are not supported"));
        }
        let component = index(&a["componentType"])?;
        let width = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(invalid("unknown glTF component type")),
        };
        let n = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT4") => 16,
            _ => return Err(invalid("unknown glTF accessor type")),
        };
        let (view, buf) = self.view(&a["bufferView"])?;
        let stride = view["byteStride"]
            .as_u64()
            .filter(|s| *s != 0)
            .map_or(n * width, |s| s as usize);
        let data = buf
            .get(a["byteOffset"].as_u64().unwrap_or(0) as usize..)
            .ok_or_else(|| invalid("invalid glTF accessor offset"))?;
        let count = index(&a["count"])?;
        if stride < n * width
            || (count > 0
                && (count - 1)
                    .checked_mul(stride)
                    .and_then(|s| s.checked_add(n * width))
                    .is_none_or(|s| s > data.len()))
        {
            return Err(invalid("invalid glTF accessor range"));
        }
        let normalized = a["normalized"].as_bool() == Some(true);
        let divisor = if normalized {
            match component {
                5120 => 127.,
                5121 if n != 1 => 255.,
                5122 => 32767.,
                5123 if n != 1 => 65535.,
                _ => 1.,
            }
        } else {
            1.
        };
        Ok(Accessor {
            data,
            component,
            width,
            n,
            count,
            stride,
            divisor,
        })
    }
    fn images(&self, py: Python<'_>, path: &Path) -> PyResult<Vec<Option<String>>> {
        let mut paths = Vec::new();
        if items(&self.js["images"]).is_empty() {
            return Ok(paths);
        }
        let metadata = fs::metadata(path)?;
        let ns = metadata
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|t| t.as_nanos() as i128)
            .unwrap_or_else(|e| -(e.duration().as_nanos() as i128));
        let tag = format!("{}|{}|{}", path.display(), metadata.len(), ns);
        let sha: String = py
            .import("hashlib")?
            .call_method1("sha1", (PyBytes::new(py, tag.as_bytes()),))?
            .call_method0("hexdigest")?
            .extract()?;
        let stem: String = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .take(40)
            .collect();
        let root: PathBuf = py
            .import("core.texture")?
            .call_method0("texture_cache_root")?
            .extract()?;
        let dir = root.join("glb").join(format!("{stem}-{}", &sha[..10]));
        for (i, img) in items(&self.js["images"]).iter().enumerate() {
            let write = || -> PyResult<String> {
                let external;
                let bytes = if let Some(id) = img.get("bufferView") {
                    self.view(id)?.1
                } else {
                    external = uri(
                        py,
                        path,
                        img["uri"]
                            .as_str()
                            .ok_or_else(|| invalid("missing glTF image"))?,
                    )?;
                    &external
                };
                let ext = if bytes.starts_with(b"\x89PNG") {
                    "png"
                } else {
                    "jpg"
                };
                let fp = dir.join(format!("img_{i}.{ext}"));
                fs::create_dir_all(&dir)?;
                if !fp.exists() {
                    fs::write(&fp, bytes)?
                }
                Ok(fp.to_string_lossy().into_owned())
            };
            paths.push(write().ok())
        }
        Ok(paths)
    }
}
struct Accessor<'a> {
    data: &'a [u8],
    component: usize,
    width: usize,
    n: usize,
    count: usize,
    stride: usize,
    divisor: f64,
}
impl Accessor<'_> {
    fn value(&self, i: usize, j: usize) -> PyResult<f64> {
        if i >= self.count || j >= self.n {
            return Err(invalid("glTF accessor index out of range"));
        }
        let s = &self.data[i * self.stride + j * self.width..][..self.width];
        let v = match self.component {
            5120 => s[0] as i8 as f64,
            5121 => s[0] as f64,
            5122 => i16::from_le_bytes(s.try_into().unwrap()) as f64,
            5123 => u16::from_le_bytes(s.try_into().unwrap()) as f64,
            5125 => u32::from_le_bytes(s.try_into().unwrap()) as f64,
            _ => f32::from_le_bytes(s.try_into().unwrap()) as f64,
        };
        Ok(v / self.divisor)
    }
}
fn local(node: &Value) -> PyResult<DMat4> {
    if let Some(v) = node.get("matrix") {
        return Ok(DMat4::from_cols_array(&numbers(v)?));
    }
    let vec = |key: &str, default: DVec3| -> PyResult<DVec3> {
        node.get(key)
            .map(|v| numbers(v).map(DVec3::from_array))
            .unwrap_or(Ok(default))
    };
    let rotation = node
        .get("rotation")
        .map(|q| numbers(q).map(DQuat::from_array))
        .unwrap_or(Ok(DQuat::IDENTITY))?;
    Ok(DMat4::from_scale_rotation_translation(
        vec("scale", DVec3::ONE)?,
        rotation,
        vec("translation", DVec3::ZERO)?,
    ))
}
fn transforms(js: &Value) -> PyResult<Vec<(usize, DMat4)>> {
    let nodes = items(&js["nodes"]);
    let children = |n: &Value| {
        items(&n["nodes"])
            .iter()
            .map(index)
            .collect::<PyResult<Vec<_>>>()
    };
    let mut roots = Vec::new();
    for scene in items(&js["scenes"]) {
        roots.extend(children(scene)?)
    }
    if roots.is_empty() {
        let mut held = HashSet::new();
        for node in nodes {
            for c in items(&node["children"]) {
                held.insert(index(c)?);
            }
        }
        roots.extend((0..nodes.len()).filter(|i| !held.contains(i)));
        roots.extend((0..nodes.len()).filter(|i| held.contains(i)));
    }
    let mut stack: Vec<_> = roots
        .into_iter()
        .rev()
        .map(|i| (i, DMat4::IDENTITY))
        .collect();
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    while let Some((i, parent)) = stack.pop() {
        if !visited.insert(i) {
            continue;
        }
        let node = nodes.get(i).ok_or_else(|| invalid("invalid glTF node"))?;
        let world = parent * local(node)?;
        result.push((i, world));
        for c in items(&node["children"]).iter().rev() {
            stack.push((index(c)?, world));
        }
    }
    Ok(result)
}
#[pyfunction]
fn gltf_node_transforms<'py>(py: Python<'py>, js: &Obj<'py>) -> PyResult<Obj<'py>> {
    let out = PyList::empty(py);
    for (i, m) in transforms(&json_value(js)?)? {
        let a = m.to_cols_array();
        out.append((
            i,
            Array2::from_shape_fn((4, 4), |(r, c)| a[c * 4 + r]).into_pyarray(py),
        ))?
    }
    Ok(out.into_any())
}
#[pyfunction]
fn gltf_parse_container(py: Python<'_>, path: PathBuf) -> PyResult<(Obj<'_>, Obj<'_>)> {
    let doc = Document::open(py, &path)?;
    let buffers = PyList::new(py, doc.buffers.iter().map(|b| PyBytes::new(py, b)))?;
    Ok((pyvalue(py, &doc.js)?, buffers.into_any()))
}
struct Recipe<'a> {
    texture: Option<&'a str>,
    color: &'a Value,
    opacity: Option<f64>,
    set: usize,
    offset: [f64; 2],
    scale: [f64; 2],
    rotation: f64,
}
impl<'a> Recipe<'a> {
    fn new(js: &'a Value, material: &'a Value, paths: &'a [Option<String>]) -> PyResult<Self> {
        let mr = &material["pbrMetallicRoughness"];
        let sg = &material["extensions"]["KHR_materials_pbrSpecularGlossiness"];
        let (tex, color) = if sg.is_null() {
            (&mr["baseColorTexture"], &mr["baseColorFactor"])
        } else {
            (&sg["diffuseTexture"], &sg["diffuseFactor"])
        };
        let mapping = [
            tex,
            &material["normalTexture"],
            &material["occlusionTexture"],
            &mr["metallicRoughnessTexture"],
        ]
        .into_iter()
        .find(|v| v.as_object().is_some_and(|m| !m.is_empty()))
        .unwrap_or(&Value::Null);
        let tt = &mapping["extensions"]["KHR_texture_transform"];
        let texture = if tex.is_null() {
            None
        } else {
            let t = at(&js["textures"], &tex["index"])?;
            t["source"]
                .as_u64()
                .and_then(|i| paths.get(i as usize))
                .and_then(|s| s.as_deref())
        };
        let a = color[3].as_f64().unwrap_or(1.);
        let opacity = if material["alphaMode"] == "BLEND" {
            if a < 0.999 {
                Some(a)
            } else if texture.is_none() {
                Some(0.999)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            texture,
            color,
            opacity,
            set: tt["texCoord"]
                .as_u64()
                .or(mapping["texCoord"].as_u64())
                .unwrap_or(0) as usize,
            offset: tt
                .get("offset")
                .map(numbers)
                .transpose()?
                .unwrap_or([0., 0.]),
            scale: tt
                .get("scale")
                .map(numbers)
                .transpose()?
                .unwrap_or([1., 1.]),
            rotation: tt["rotation"].as_f64().unwrap_or(0.),
        })
    }
}
#[pyfunction]
#[pyo3(signature=(path,*,materials=None))]
pub(crate) fn gltf_read_triangles<'py>(
    py: Python<'py>,
    path: PathBuf,
    materials: Option<&Obj<'py>>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let doc = Document::open(py, &path)?;
    let js = &doc.js;
    if items(&js["extensionsRequired"]).iter().any(|v| {
        v.as_str()
            .is_some_and(|s| s.to_lowercase().contains("draco"))
    }) {
        return Err(invalid(
            "Draco-compressed GLB — re-export/download uncompressed",
        ));
    }
    let paths = doc.images(py, &path)?;
    let recipes = PyDict::new(py);
    let batches = PyList::empty(py);
    for (i, world) in transforms(js)? {
        let node = &js["nodes"][i];
        let Some(mesh) = node["mesh"]
            .as_u64()
            .and_then(|m| js["meshes"].get(m as usize))
        else {
            continue;
        };
        for primitive in items(&mesh["primitives"]) {
            let attr = &primitive["attributes"];
            if primitive["mode"].as_u64().unwrap_or(4) != 4 || attr.get("POSITION").is_none() {
                continue;
            }
            let pos = doc.accessor(&attr["POSITION"])?;
            let indices = primitive
                .get("indices")
                .map(|id| doc.accessor(id))
                .transpose()?;
            let mid = primitive["material"].as_u64().map(|i| i as usize);
            let mat = mid
                .and_then(|m| js["materials"].get(m))
                .unwrap_or(&Value::Null);
            let r = Recipe::new(js, mat, &paths)?;
            if let (Some(reg), Some(id)) = (materials, mid) {
                if !mat.is_null()
                    && !recipes.contains(id)?
                    && mat["extensions"]["KHR_materials_pbrSpecularGlossiness"].is_null()
                {
                    let module = py.import("built_core.materials")?;
                    let pbr = crate::gltf_maps::gltf_import_recipe(
                        py, &pyvalue(py, mat)?, &pyvalue(py, &js["textures"])?, PyList::new(py, &paths)?.as_any(),
                    )?;
                    let kw = PyDict::new(py);
                    kw.set_item("pbr", &pbr)?;
                    let rgb = items(r.color)
                        .iter()
                        .take(3)
                        .map(|v| pyvalue(py, v))
                        .collect::<PyResult<Vec<_>>>()?;
                    let rgb = if rgb.is_empty() {
                        PyTuple::new(py, [1, 1, 1])?
                    } else {
                        PyTuple::new(py, rgb)?
                    };
                    let texture = if let Some(path) = r.texture {
                        let d = PyDict::new(py);
                        d.set_item("path", path)?;
                        Some(d)
                    } else {
                        None
                    };
                    let name = mat["name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Material {}", id + 1));
                    let material = module
                        .getattr("Material")?
                        .call((name, rgb, texture, r.opacity), Some(&kw))?;
                    recipes
                        .set_item(id, (module.call_method1("register", (reg, material))?, pbr))?;
                }
            }
            let recipe = mid.map(|k| recipes.get_item(k)).transpose()?.flatten();
            let has_maps = if let Some(rec) = &recipe {
                rec.get_item(1)?
                    .cast::<PyDict>()?
                    .keys()
                    .iter()
                    .any(|k| k.extract::<String>().is_ok_and(|s| s.ends_with("_map")))
            } else {
                false
            };
            let uv = attr
                .get(format!("TEXCOORD_{}", r.set))
                .filter(|_| r.texture.is_some() || has_maps)
                .map(|id| doc.accessor(id))
                .transpose()?;
            let count = indices.as_ref().map_or(pos.count, |a| a.count * a.n) / 3;
            let (points, coords) = py.detach(|| -> PyResult<_> {
                let mut points = Vec::with_capacity(count * 9);
                let mut coords = Vec::new();
                let (s, c) = r.rotation.sin_cos();
                for j in 0..count * 3 {
                    let k = match &indices {
                        Some(a) => {
                            let n = a.value(j / a.n, j % a.n)?;
                            if n < 0. || n >= pos.count as f64 {
                                return Err(invalid("invalid glTF vertex index"));
                            }
                            n as usize
                        }
                        None => j,
                    };
                    let p = world.transform_point3(DVec3::new(
                        pos.value(k, 0)?,
                        pos.value(k, 1)?,
                        pos.value(k, 2)?,
                    ));
                    points.extend([p.x, -p.z, p.y]);
                    if let Some(uv) = &uv {
                        let x = uv.value(k, 0)? * r.scale[0];
                        let y = uv.value(k, 1)? * r.scale[1];
                        coords.extend([x * c - y * s + r.offset[0], x * s + y * c + r.offset[1]])
                    }
                }
                Ok((points, coords))
            })?;
            let tris = Array3::from_shape_vec((count, 3, 3), points)
                .unwrap()
                .into_pyarray(py);
            let uvs = uv.as_ref().map(|_| {
                Array3::from_shape_vec((count, 3, 2), coords)
                    .unwrap()
                    .into_pyarray(py)
            });
            let mut values = vec![
                r.texture
                    .filter(|_| uvs.is_some())
                    .into_pyobject(py)?
                    .into_any(),
                pyvalue(py, r.color)?,
                r.opacity.into_pyobject(py)?.into_any(),
            ];
            if let Some(recipe) = recipe {
                values.push(recipe)
            }
            batches.append((tris, uvs, PyTuple::new(py, values)?))?;
        }
    }
    Ok((batches.into_any(), pyvalue(py, js)?))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(gltf_read_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(gltf_parse_container, m)?)?;
    m.add_function(wrap_pyfunction!(gltf_node_transforms, m)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_painted_triangle_is_read_in_file_coordinates() {
        let (text, bin) = chunks(include_bytes!(
            "../../../tests/fixtures/gltf/painted-triangle.glb"
        ))
        .unwrap();
        let doc = Document {
            js: serde_json::from_slice(text).unwrap(),
            buffers: vec![bin.to_vec()],
        };
        let primitive = &doc.js["meshes"][0]["primitives"][0];
        let a = doc.accessor(&primitive["attributes"]["POSITION"]).unwrap();
        assert_eq!((a.count, a.n), (3, 3));
        let mut points = (0..3)
            .map(|i| {
                [
                    a.value(i, 0).unwrap(),
                    a.value(i, 1).unwrap(),
                    a.value(i, 2).unwrap(),
                ]
            })
            .collect::<Vec<_>>();
        points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert_eq!(points, [[-1.25, 0., 0.], [0., 2., -1.], [1., 0., 0.]]);
        assert_eq!(
            doc.js["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
            serde_json::json!([0.1, 0.3, 0.9, 1.0])
        );
    }

    #[test]
    fn normalized_strided_uvs_respect_view_boundaries() {
        let mut doc = Document {
            js: serde_json::json!({
                "bufferViews":[{"buffer":0,"byteOffset":1,"byteLength":6,"byteStride":4}],
                "accessors":[{"bufferView":0,"count":2,"type":"VEC2","componentType":5121,"normalized":true}]
            }),
            buffers: vec![vec![9, 0, 128, 7, 7, 255, 0, 42]],
        };
        let a = doc.accessor(&Value::from(0)).unwrap();
        assert_eq!(a.value(0, 1).unwrap(), 128. / 255.);
        assert_eq!(a.value(1, 0).unwrap(), 1.);
        doc.js["bufferViews"][0]["byteLength"] = Value::from(5);
        assert!(doc.accessor(&Value::from(0)).is_err());
    }
}
