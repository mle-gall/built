// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary glTF writer. Ordered dictionaries preserve the established wire layout.
use glam::{DMat4, DVec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use std::{
    collections::{BTreeSet, HashMap},
    io::Write,
    path::{Path, PathBuf},
};
type Obj<'py> = Bound<'py, PyAny>;
type Dict<'py> = Bound<'py, PyDict>;
type List<'py> = Bound<'py, PyList>;
macro_rules! record {
 ($py:expr; $($k:literal => $v:expr),* $(,)?)=>{{let out=PyDict::new($py);$(out.set_item($k,$v)?;)*out}};
}
fn get<'py>(d: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    d.call_method1("get", (k,))
}
// A product close to the halfway point needs decimal rounding on the
// exact original value. All other products round to the same integer.
fn quant(value: f64, precision: usize) -> u64 {
    let scale = if precision == 6 { 1e6 } else { 1e4 };
    let scaled = value * scale;
    let rounded = if value.fract() == 0. {
        value
    } else if (scaled.fract().abs() - 0.5).abs() > f64::EPSILON * scaled.abs() {
        scaled.round() / scale
    } else {
        format!("{value:.precision$}").parse().unwrap_or(value)
    };
    if rounded == 0. {
        0
    } else {
        rounded.to_bits()
    }
}
fn yup(p: &Obj<'_>) -> PyResult<[f64; 3]> {
    let p = crate::math::xyz(p)?;
    Ok([p.x, p.z, -p.y])
}
fn image(py: Python<'_>, path: &Path) -> PyResult<Option<(Vec<u8>, &'static str)>> {
    let Ok(data) = std::fs::read(path) else {
        return Ok(None);
    };
    if data.starts_with(b"\x89PNG") {
        return Ok(Some((data, "image/png")));
    }
    if data.starts_with(b"\xff\xd8\xff") {
        return Ok(Some((data, "image/jpeg")));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tga"))
        && !py
            .import("core.texture")?
            .call_method1("is_image_bytes", (PyBytes::new(py, &data),))?
            .is_truthy()?
    {
        return Ok(None);
    }
    let raster = py
        .import("ingetrazo_render")?
        .getattr("SurveyRaster")?
        .call_method1("open", (path,))?;
    if raster.is_none() {
        return Ok(None);
    }
    Ok(Some((raster.call_method0("png")?.extract()?, "image/png")))
}
struct Writer<'py> {
    buf: Vec<u8>,
    views: List<'py>,
    accessors: List<'py>,
    materials: List<'py>,
    images: List<'py>,
    textures: List<'py>,
    samplers: List<'py>,
    extensions: BTreeSet<String>,
    texture_ids: HashMap<PathBuf, usize>,
}
impl<'py> Writer<'py> {
    fn new(py: Python<'py>) -> Self {
        Self {
            buf: vec![],
            views: PyList::empty(py),
            accessors: PyList::empty(py),
            materials: PyList::empty(py),
            images: PyList::empty(py),
            textures: PyList::empty(py),
            samplers: PyList::empty(py),
            extensions: Default::default(),
            texture_ids: Default::default(),
        }
    }
    fn view(&mut self, data: &[u8], target: Option<u32>) -> PyResult<usize> {
        self.buf.resize(self.buf.len().next_multiple_of(4), 0);
        let view = record!(self.views.py();"buffer"=>0,"byteOffset"=>self.buf.len(),"byteLength"=>data.len());
        if let Some(t) = target {
            view.set_item("target", t)?
        }
        self.buf.extend_from_slice(data);
        let id = self.views.len();
        self.views.append(view)?;
        Ok(id)
    }
    fn accessor(
        &mut self,
        data: &[u8],
        size: usize,
        kind: &str,
        range: Option<([f64; 3], [f64; 3])>,
    ) -> PyResult<usize> {
        let indices = kind == "SCALAR";
        let v = self.view(data, Some(if indices { 34963 } else { 34962 }))?;
        let acc = record!(self.views.py();"bufferView"=>v,"componentType"=>if indices{5125}else{5126},"count"=>size,"type"=>kind);
        if let Some((lo, hi)) = range {
            acc.set_item("min", lo)?;
            acc.set_item("max", hi)?
        }
        let id = self.accessors.len();
        self.accessors.append(acc)?;
        Ok(id)
    }
    fn texture(&mut self, src: &Obj<'py>) -> PyResult<Option<usize>> {
        if !src.is_truthy()? {
            return Ok(None);
        }
        let path: PathBuf = src.extract()?;
        if let Some(&i) = self.texture_ids.get(&path) {
            return Ok(Some(i));
        }
        let py = src.py();
        let Some((bytes, mime)) = image(py, &path)? else {
            return Ok(None);
        };
        let view = self.view(&bytes, None)?;
        let i = self.images.len();
        self.images.append(record!(py;"bufferView"=>view,"mimeType"=>mime,"name"=>path.file_name().unwrap_or_default().to_string_lossy()))?;
        if self.samplers.is_empty() {
            self.samplers
                .append(record!(py;"wrapS"=>10497,"wrapT"=>10497))?
        }
        self.textures.append(record!(py;"source"=>i,"sampler"=>0))?;
        self.texture_ids.insert(path, i);
        Ok(Some(i))
    }
    fn material(&mut self, info: &Obj<'py>, name: &Obj<'py>, embed: bool) -> PyResult<()> {
        let py = info.py();
        let mr = record!(py;"metallicFactor"=>0.0,"roughnessFactor"=>1.0);
        let mat = record!(py;"name"=>name,"doubleSided"=>true,"pbrMetallicRoughness"=>&mr);
        if embed && get(info, "map")?.is_truthy()? {
            let src = info.get_item("src")?;
            if let Some(i) = self.texture(&src)? {
                mr.set_item("baseColorTexture", record!(py;"index"=>i))?;
                if py
                    .import("core.texture")?
                    .call_method1("image_has_cutout", (src.str()?,))?
                    .is_truthy()?
                {
                    mat.set_item("alphaMode", "MASK")?;
                    mat.set_item("alphaCutoff", 0.5)?
                }
            } else {
                mr.set_item("baseColorFactor", [1, 1, 1, 1])?
            }
        } else {
            let c = info.get_item("color")?;
            let factor = PyList::empty(py);
            for i in 0..3 {
                factor.append(c.get_item(i)?)?
            }
            factor.append(1.)?;
            mr.set_item("baseColorFactor", factor)?;
        }
        let finish = get(info, "finish")?;
        let recipe = get(info, "pbr")?;
        if !recipe.is_none() {
            let factor = py.get_type::<PyList>().call1((info.get_item("color")?,))?;
            factor.call_method1("append", (1.,))?;
            mr.set_item("baseColorFactor", factor)?;
            mr.set_item(
                "metallicFactor",
                recipe.call_method1("get", ("metallic", 0.))?,
            )?;
            mr.set_item(
                "roughnessFactor",
                recipe.call_method1("get", ("roughness", 1.))?,
            )?;
            if embed {
                let (normal, packed, ao): (Obj<'_>, Obj<'_>, Obj<'_>) = py
                    .import("formats.pbr_maps")?
                    .call_method1("export_maps", (&recipe,))?
                    .extract()?;
                for (target, k, path, field) in [
                    (&mr, "metallicRoughnessTexture", packed, None),
                    (
                        &mat,
                        "normalTexture",
                        normal,
                        Some(("scale", "normal_scale")),
                    ),
                    (
                        &mat,
                        "occlusionTexture",
                        ao,
                        Some(("strength", "ao_strength")),
                    ),
                ] {
                    if let Some(i) = self.texture(&path)? {
                        let d = record!(py;"index"=>i);
                        if let Some((k, src)) = field {
                            d.set_item(k, recipe.call_method1("get", (src, 1.))?)?;
                        }
                        target.set_item(k, d)?;
                    }
                }
            }
        } else if finish.is_truthy()? {
            let props = py
                .import("core.finish")?
                .getattr("PBR")?
                .get_item(&finish)?;
            mr.set_item("roughnessFactor", props.get_item(0)?)?;
            mr.set_item("metallicFactor", props.get_item(1)?)?;
            mat.set_item("extras", record!(py;"ingetrazo_finish"=>finish))?;
            let ext = PyDict::new(py);
            let trans = props.get_item(2)?;
            if trans.is_truthy()? {
                ext.set_item(
                    "KHR_materials_transmission",
                    record!(py;"transmissionFactor"=>trans),
                )?
            }
            let ior = props.get_item(3)?;
            if !ior.eq(1.5)? {
                ext.set_item("KHR_materials_ior", record!(py;"ior"=>ior))?;
            }
            if !ext.is_empty() {
                for k in ext.keys().iter() {
                    self.extensions.insert(k.extract()?);
                }
                mat.set_item("extensions", ext)?;
            }
        }
        let opacity = get(info, "opacity")?;
        if !opacity.is_none() {
            let factor = mr.call_method1("setdefault", ("baseColorFactor", [1., 1., 1., 1.]))?;
            factor.set_item(3, opacity.extract::<f64>()?)?;
            mat.set_item("alphaMode", "BLEND")?;
            if mat.contains("alphaCutoff")? {
                mat.del_item("alphaCutoff")?
            }
        }
        self.materials.append(mat)
    }
    fn primitives(
        &mut self,
        source: &Obj<'py>,
        keys: &List<'py>,
        materials: &Obj<'py>,
    ) -> PyResult<List<'py>> {
        let py = source.py();
        let out = PyList::empty(py);
        for (mi, key) in keys.iter().enumerate() {
            if !source.contains(&key)? {
                continue;
            }
            let info = materials.get_item(&key)?;
            let pbr = get(&info, "pbr")?;
            let textured = key.get_item(0)?.eq("tex")?
                || (!pbr.is_none()
                    && pbr.try_iter()?.any(|k| {
                        k.is_ok_and(|k| k.extract::<String>().is_ok_and(|s| s.ends_with("_map")))
                    }));
            let mut vertices = HashMap::new();
            let (mut positions, mut normals, mut uvs, mut indices) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for tri in source.get_item(&key)?.try_iter()? {
                for v in tri?.get_item(1)?.try_iter()? {
                    let v = v?;
                    let p = yup(&v.get_item(0)?)?;
                    let n = yup(&v.get_item(2)?)?;
                    let uv = if textured {
                        let uv = v.get_item(1)?;
                        [
                            uv.get_item(0)?.extract::<f64>()?,
                            uv.get_item(1)?.extract()?,
                        ]
                    } else {
                        [0.; 2]
                    };
                    let id = [
                        quant(p[0], 6),
                        quant(p[1], 6),
                        quant(p[2], 6),
                        quant(n[0], 4),
                        quant(n[1], 4),
                        quant(n[2], 4),
                        quant(uv[0], 6),
                        quant(uv[1], 6),
                    ];
                    let next = vertices.len() as u32;
                    let idx = *vertices.entry(id).or_insert(next);
                    if idx == next {
                        for d in 0..3 {
                            if p[d] < lo[d] {
                                lo[d] = p[d]
                            }
                            if p[d] > hi[d] {
                                hi[d] = p[d]
                            }
                            positions.extend((p[d] as f32).to_le_bytes());
                            normals.extend((n[d] as f32).to_le_bytes())
                        }
                        if textured {
                            uvs.extend((uv[0] as f32).to_le_bytes());
                            uvs.extend(((1. - uv[1]) as f32).to_le_bytes())
                        }
                    }
                    indices.extend(idx.to_le_bytes());
                }
            }
            let nv = vertices.len();
            if nv == 0 {
                continue;
            }
            let attrs = record!(py;"POSITION"=>self.accessor(&positions,nv,"VEC3",Some((lo,hi)))?,"NORMAL"=>self.accessor(&normals,nv,"VEC3",None)?);
            if textured {
                attrs.set_item("TEXCOORD_0", self.accessor(&uvs, nv, "VEC2", None)?)?
            }
            let i = self.accessor(&indices, indices.len() / 4, "SCALAR", None)?;
            out.append(record!(py;"attributes"=>attrs,"indices"=>i,"material"=>mi,"mode"=>4))?;
        }
        Ok(out)
    }
    fn finish(mut self, doc: Dict<'py>, path: PathBuf) -> PyResult<()> {
        let py = doc.py();
        if !self.accessors.is_empty() {
            doc.set_item("accessors", self.accessors)?;
            doc.set_item("bufferViews", self.views)?;
            doc.set_item("buffers", [record!(py;"byteLength"=>self.buf.len())])?;
        }
        if !self.materials.is_empty() {
            doc.set_item("materials", self.materials)?
        }
        if !self.extensions.is_empty() {
            doc.set_item(
                "extensionsUsed",
                self.extensions.into_iter().collect::<Vec<_>>(),
            )?;
        }
        if !self.images.is_empty() {
            doc.set_item("images", self.images)?;
            doc.set_item("textures", self.textures)?;
            doc.set_item("samplers", self.samplers)?;
        }
        let kw = record!(py;"separators"=>(",",":"));
        let mut json = py
            .import("json")?
            .getattr("dumps")?
            .call((doc,), Some(&kw))?
            .extract::<String>()?
            .into_bytes();
        json.resize(json.len().next_multiple_of(4), b' ');
        self.buf.resize(self.buf.len().next_multiple_of(4), 0);
        let total = u32::try_from(
            12 + 8
                + json.len()
                + if self.buf.is_empty() {
                    0
                } else {
                    8 + self.buf.len()
                },
        )
        .map_err(|_| PyValueError::new_err("glTF exceeds 4 GiB container limit"))?;
        py.detach(move || -> std::io::Result<()> {
            let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
            f.write_all(b"glTF")?;
            f.write_all(&2u32.to_le_bytes())?;
            f.write_all(&total.to_le_bytes())?;
            for (kind, data) in [(b"JSON", &json), (b"BIN\0", &self.buf)] {
                if !data.is_empty() {
                    f.write_all(&(data.len() as u32).to_le_bytes())?;
                    f.write_all(kind)?;
                    f.write_all(data)?
                }
            }
            f.flush()
        })?;
        Ok(())
    }
}
#[pyfunction]
fn file_facing(_anchor: &Obj<'_>) -> crate::math::Vec3 {
    DVec3::NEG_Y.into()
}
struct Hierarchy<'py> {
    helper: Bound<'py, PyModule>,
    materials: Dict<'py>,
    meshes: Dict<'py>,
    figures: List<'py>,
    smoother: Obj<'py>,
    tr: Obj<'py>,
}
impl<'py> Hierarchy<'py> {
    fn part(
        &self,
        g: &Obj<'py>,
        base: &Obj<'py>,
        above: &Obj<'py>,
        depth: usize,
    ) -> PyResult<Option<Dict<'py>>> {
        if depth > 512 {
            return Err(PyValueError::new_err("group hierarchy too deep"));
        }
        let py = g.py();
        let m = g.getattr("xform")?;
        let world = if base.is_none() {
            m.clone()
        } else if m.is_none() {
            base.clone()
        } else {
            base.mul(&m)?
        };
        let paint = g.getattr("material")?;
        let paint = if paint.is_truthy()? {
            paint
        } else {
            above.clone()
        };
        let mesh = g.getattr("mesh")?;
        let kind = g.getattr("billboard")?;
        let figure = kind.is_truthy()?;
        let name = g.getattr("name")?;
        let name = if name.is_truthy()? {
            name
        } else {
            self.tr.call1((if figure { "Figure" } else { "Group" },))?
        };
        let key = if figure {
            ("figure", self.figures.len()).into_pyobject(py)?.into_any()
        } else {
            (
                mesh.as_ptr() as usize,
                py.import("core.materials")?
                    .call_method1("material_sig", (&paint,))?,
            )
                .into_pyobject(py)?
                .into_any()
        };
        let mut feet = None;
        if !self.meshes.contains(&key)? {
            let painted = if figure {
                let placed = if world.is_none() {
                    mesh
                } else {
                    py.import("core.group")?
                        .call_method1("transformed_mesh", (mesh, &world))?
                };
                let anchor = self.helper.call_method1("_feet", (&placed,))?;
                if anchor.is_none() {
                    return Ok(None);
                }
                feet = Some(crate::math::xyz(&anchor)?);
                self.helper.call_method1(
                    if kind.eq("mesh")? {
                        "_turned_toward"
                    } else {
                        "_facing_quad"
                    },
                    (placed, wrap_pyfunction!(file_facing, py)?),
                )?
            } else {
                mesh
            };
            let prims = PyDict::new(py);
            for face in painted.getattr("faces")?.try_iter()? {
                let face = face?;
                let attrs = self.helper.call_method1("_shown", (&face, &paint))?;
                self.helper.call_method1(
                    "_add_face",
                    (face, &self.materials, &prims, attrs, &self.smoother),
                )?;
            }
            if figure && prims.is_empty() {
                return Ok(None);
            }
            self.meshes.set_item(&key, prims)?;
        }
        let kids = PyList::empty(py);
        if let Some(p) = feet {
            let extra = record!(py;"ingetrazo_faceme"=>[p.x,p.y,p.z,-std::f64::consts::FRAC_PI_2]);
            self.figures.append(record!(py;"name"=>name,"matrix"=>py.None(),"mesh"=>key,"children"=>kids,"extras"=>extra))?;
            return Ok(None);
        }
        let node = record!(py;"name"=>name,"matrix"=>m,"mesh"=>if self.meshes.get_item(&key)?.is_some_and(|r|r.len().unwrap_or(0)>0){Some(key)}else{None},"children"=>&kids);
        for child in g.getattr("children")?.try_iter()? {
            if let Some(node) = self.part(&child?, &world, &paint, depth + 1)? {
                kids.append(node)?
            }
        }
        Ok(Some(node))
    }
    fn collect(
        scene: &Obj<'py>,
        only: Option<&Obj<'py>>,
    ) -> PyResult<(Obj<'py>, Obj<'py>, List<'py>)> {
        let py = scene.py();
        let helper = py.import("formats.meshexport")?;
        let h = Self {
            smoother: helper.call_method0("SmoothNormals")?,
            helper,
            materials: PyDict::new(py),
            meshes: PyDict::new(py),
            figures: PyList::empty(py),
            tr: py.import("core.i18n")?.getattr("tr")?,
        };
        let visible = |f: &Obj<'py>| -> PyResult<bool> {
            Ok(scene.call_method1("entity_visible", (f,))?.is_truthy()?
                && match only {
                    Some(o) => o.contains(f)?,
                    None => true,
                })
        };
        let roots = PyList::empty(py);
        let prims = PyDict::new(py);
        let mut loose = false;
        for f in scene.getattr("loose_mesh")?.getattr("faces")?.try_iter()? {
            let f = f?;
            if visible(&f)? {
                loose = true;
                h.helper.call_method1(
                    "_add_face",
                    (f, &h.materials, &prims, py.None(), &h.smoother),
                )?;
            }
        }
        if loose {
            h.meshes.set_item("loose", &prims)?;
            roots.append(record!(py;"name"=>h.tr.call1(("Loose geometry",))?,"matrix"=>py.None(),"mesh"=>"loose","children"=>PyList::empty(py)))?;
        }
        let none = py.None().into_bound(py);
        for g in scene.getattr("groups")?.try_iter()? {
            let g = g?;
            if visible(&g)? {
                if let Some(node) = h.part(&g, &none, &none, 0)? {
                    roots.append(node)?
                }
            }
        }
        roots.call_method1("extend", (h.figures,))?;
        h.helper
            .call_method1("_resolve_finishes", (scene, &h.materials))?;
        Ok((h.materials.into_any(), h.meshes.into_any(), roots))
    }
}
fn emit<'py>(part: &Obj<'py>, nodes: &List<'py>, mesh_ids: &Dict<'py>) -> PyResult<usize> {
    let py = part.py();
    let node = record!(py;"name"=>part.get_item("name")?);
    if let Some(id) = mesh_ids.get_item(part.get_item("mesh")?)? {
        node.set_item("mesh", id)?
    }
    let m = part.get_item("matrix")?;
    if !m.is_none() && !m.call_method0("isIdentity")?.is_truthy()? {
        let c = DMat4::from_cols_array(&[
            1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
        ]);
        let m = DMat4::from_cols_array(&m.call_method0("data")?.extract()?);
        node.set_item("matrix", (c * m * c.transpose()).to_cols_array())?;
    }
    let extras = get(part, "extras")?;
    if extras.is_truthy()? {
        node.set_item("extras", extras)?
    }
    let i = nodes.len();
    nodes.append(&node)?;
    let mut kids = Vec::new();
    for child in part.get_item("children")?.try_iter()? {
        kids.push(emit(&child?, nodes, mesh_ids)?)
    }
    if !kids.is_empty() {
        node.set_item("children", kids)?
    }
    Ok(i)
}
#[pyfunction]
#[pyo3(signature=(scene,path,face_me=None,only=None,*,embed_textures=true))]
fn gltf_save<'py>(
    py: Python<'py>,
    scene: &Obj<'py>,
    path: PathBuf,
    face_me: Option<&Obj<'py>>,
    only: Option<&Obj<'py>>,
    embed_textures: bool,
) -> PyResult<()> {
    let helper = py.import("formats.meshexport")?;
    let (mat, sources, parts) = if let Some(camera) = face_me {
        helper
            .call_method1("collect_geometry_split", (scene, camera))?
            .extract()?
    } else {
        Hierarchy::collect(scene, only)?
    };
    let keys = PyDict::new(py);
    let add = |source: &Obj<'py>| -> PyResult<()> {
        for k in source.try_iter()? {
            keys.set_item(k?, true)?
        }
        Ok(())
    };
    if face_me.is_some() {
        add(&sources)?;
        for figure in parts.iter() {
            add(&figure.get_item("prims")?)?
        }
    } else {
        for (_, source) in sources.cast::<PyDict>()? {
            add(&source)?
        }
    }
    let keys = keys.keys();
    let names = helper.call_method1("export_names", (&mat,))?;
    let mut writer = Writer::new(py);
    for key in keys.iter() {
        writer.material(&mat.get_item(&key)?, &names.get_item(&key)?, embed_textures)?;
    }
    let extras = record!(py;"generator"=>"IngeTrazo");
    let geo = helper.call_method1("geolocation", (scene,))?;
    if !geo.is_none() {
        extras.set_item(
            "ingetrazo_geolocation",
            record!(py;"lat"=>geo.get_item(0)?,"lon"=>geo.get_item(1)?,"alt"=>geo.get_item(2)?),
        )?;
    }
    let doc =
        record!(py;"asset"=>record!(py;"version"=>"2.0","generator"=>"IngeTrazo","extras"=>extras));
    let meshes = PyList::empty(py);
    let nodes = PyList::empty(py);
    let mut roots = Vec::new();
    if face_me.is_some() {
        let primitives = writer.primitives(&sources, &keys, &mat)?;
        if !primitives.is_empty() {
            meshes.append(record!(py;"name"=>"IngeTrazo","primitives"=>primitives))?;
            nodes.append(record!(py;"mesh"=>0,"name"=>"IngeTrazo"))?
        }
        let mut seq = 0;
        for figure in parts.iter() {
            let prim = writer.primitives(&figure.get_item("prims")?, &keys, &mat)?;
            if prim.is_empty() {
                continue;
            }
            seq += 1;
            let name = format!("Figure {seq}");
            meshes.append(record!(py;"name"=>&name,"primitives"=>prim))?;
            let p = crate::math::xyz(&figure.get_item("feet")?)?;
            nodes.append(record!(py;"mesh"=>meshes.len()-1,"name"=>name,"extras"=>record!(py;"ingetrazo_faceme"=>[p.x,p.y,p.z,figure.get_item("yaw")?.extract::<f64>()?])))?;
        }
        roots.extend(0..nodes.len());
    } else {
        let mesh_names = PyDict::new(py);
        let mut queue: std::collections::VecDeque<_> = parts.iter().collect();
        while let Some(part) = queue.pop_front() {
            let k = part.get_item("mesh")?;
            if !mesh_names.contains(&k)? {
                mesh_names.set_item(k, part.get_item("name")?)?
            }
            for c in part.get_item("children")?.try_iter()? {
                queue.push_back(c?)
            }
        }
        let ids = PyDict::new(py);
        for (key, source) in sources.cast::<PyDict>()? {
            let prim = writer.primitives(&source, &keys, &mat)?;
            if !prim.is_empty() {
                ids.set_item(&key, meshes.len())?;
                meshes.append(record!(py;"name"=>mesh_names.call_method1("get",(key,"Mesh"))?,"primitives"=>prim))?
            }
        }
        for part in parts.iter() {
            roots.push(emit(&part, &nodes, &ids)?)
        }
    }
    if !meshes.is_empty() {
        doc.set_item("meshes", meshes)?
    }
    if !nodes.is_empty() {
        doc.set_item("nodes", nodes)?
    }
    doc.set_item("scenes", [record!(py;"nodes"=>roots)])?;
    doc.set_item("scene", 0)?;
    writer.finish(doc, path)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(gltf_save, m)?)
}
