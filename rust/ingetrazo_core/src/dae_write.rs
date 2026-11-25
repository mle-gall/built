// SPDX-License-Identifier: GPL-3.0-or-later
//! Structured COLLADA 1.4.1 scene writer.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::{
    collections::HashMap,
    fmt::Write,
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn name(s: &str) -> String {
    let mut safe: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_-.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !safe.starts_with(|c: char| c == '_' || c.is_alphabetic()) {
        safe.insert(0, '_')
    }
    safe
}
fn text(o: &Obj<'_>) -> PyResult<String> {
    o.str()?.extract()
}
fn get<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    o.call_method1("get", (k,))
}
fn ints(a: &[usize]) -> String {
    a.iter().map(usize::to_string).collect::<Vec<_>>().join(" ")
}
fn matrix(m: &Obj<'_>) -> PyResult<String> {
    let cols: [f64; 16] = m.call_method0("data")?.extract()?;
    let fmt = m.py().import("builtins")?.getattr("format")?;
    let mut out = vec![];
    for r in 0..4 {
        for c in 0..4 {
            out.push(fmt.call1((cols[c * 4 + r], ".9g"))?.extract::<String>()?)
        }
    }
    Ok(out.join(" "))
}
fn extra(n: &str, layer: Option<&str>, bill: Option<&str>) -> String {
    let mut s = format!(
        "<extra><technique profile=\"IngeTrazo\"><name>{}</name>",
        escape(n)
    );
    for (t, v) in [("layer", layer), ("billboard", bill)] {
        if let Some(v) = v {
            write!(s, "<{t}>{}</{t}>", escape(v)).unwrap()
        }
    }
    s.push_str("</technique></extra>");
    s
}
fn xyz(p: &Obj<'_>) -> PyResult<[f64; 3]> {
    Ok(crate::math::xyz(p)?.to_array())
}
struct Source<const N: usize> {
    ids: HashMap<[u64; N], usize>,
    rows: Vec<[f64; N]>,
}
impl<const N: usize> Default for Source<N> {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            rows: vec![],
        }
    }
}
impl<const N: usize> Source<N> {
    fn rounded(v: [f64; N]) -> [f64; N] {
        v.map(|c| {
            if c.fract() == 0. {
                return c;
            }
            format!("{c:.6}").parse().unwrap_or(c)
        })
    }
    fn key(v: [f64; N]) -> [u64; N] {
        v.map(|f| if f == 0. { 0 } else { f.to_bits() })
    }
    fn insert(&mut self, v: [f64; N]) -> usize {
        let r = Self::rounded(v);
        let key = Self::key(r);
        *self.ids.entry(key).or_insert_with(|| {
            let i = self.rows.len();
            self.rows.push(r);
            i
        })
    }
    fn lookup(&self, v: [f64; N]) -> Option<&usize> {
        self.ids.get(&Self::key(Self::rounded(v)))
    }
    fn encode(&self, g: &str, id: &str, params: &str, out: &mut String) {
        let n = self.rows.len();
        let sid = format!("{g}-{id}");
        write!(
            out,
            "<source id=\"{sid}\"><float_array id=\"{sid}-array\" count=\"{}\">",
            n * N
        )
        .unwrap();
        for (i, c) in self.rows.iter().flatten().enumerate() {
            if i != 0 {
                out.push(' ')
            }
            write!(out, "{c:.6}").unwrap()
        }
        write!(out,"</float_array><technique_common><accessor source=\"#{sid}-array\" count=\"{n}\" stride=\"{N}\">").unwrap();
        for p in params.chars() {
            write!(out, "<param name=\"{p}\" type=\"float\"/>").unwrap()
        }
        out.push_str("</accessor></technique_common></source>");
    }
}
#[derive(Default)]
struct Primitive {
    mat: usize,
    textured: bool,
    counts: Vec<usize>,
    poly: Vec<usize>,
    tri: Vec<usize>,
    runs: Vec<usize>,
}
#[derive(Default)]
struct Geometry {
    pos: Source<3>,
    norm: Source<3>,
    uv: Source<2>,
    slots: Vec<Primitive>,
    slot_ids: HashMap<usize, usize>,
    lines: Vec<usize>,
    soft: Vec<([f64; 3], [f64; 3])>,
}
impl Geometry {
    fn append_face(
        &mut self,
        exp: &mut Export<'_>,
        face: &Obj<'_>,
        paint: &Obj<'_>,
    ) -> PyResult<()> {
        let py = face.py();
        let mut attrs = face.getattr("attrs")?;
        if paint.is_truthy()? {
            attrs = py
                .import("ingetrazo_core.materials")?
                .call_method1("effective_attrs", (attrs, paint))?;
            if !attrs.is_truthy()? {
                attrs = PyDict::new(py).into_any()
            }
        }
        let (mat, textured) = exp.material(&attrs)?;
        let slot = *self.slot_ids.entry(mat).or_insert_with(|| {
            let n = self.slots.len();
            self.slots.push(Primitive {
                mat,
                textured,
                ..Default::default()
            });
            n
        });
        let slot = &mut self.slots[slot];
        let normal = exp.smoother.call_method1("flat", (face,))?;
        let n = crate::math::xyz(&normal)?;
        let ni = self.norm.insert(n.to_array());
        let corners = exp.smoother.call_method1("corners", (face,))?;
        let verts = face.getattr("vertices")?;
        let vp = verts
            .try_iter()?
            .map(|p| crate::math::xyz(&p?))
            .collect::<PyResult<Vec<_>>>()?;
        let convex = !face.getattr("hole_loops")?.is_truthy()?
            && (0..vp.len()).all(|i| {
                let a = vp[(i + vp.len() - 1) % vp.len()];
                let b = vp[i];
                let c = vp[(i + 1) % vp.len()];
                (b - a).cross(c - b).dot(n) >= 2e-9
            });
        let polys = if convex {
            slot.counts.push(vp.len());
            vec![verts]
        } else {
            let tris = exp
                .helpers
                .call_method1("export_triangles", (face,))?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?;
            slot.runs.push(tris.len());
            tris
        };
        let stream = if convex {
            &mut slot.poly
        } else {
            &mut slot.tri
        };
        let tex = get(&attrs, "texture")?;
        for pts in polys {
            let uvs = if textured {
                let uvw = get(&tex, "uvw")?;
                let texture = py.import("core.texture")?;
                if uvw.is_truthy()? {
                    Some(crate::model::module(texture.py(), "ingetrazo_core")?.call_method1("texture_affine_uv", (uvw, &pts))?)
                } else {
                    let getf = |k: &str, d: f64| -> PyResult<f64> {
                        let v = tex.call_method1("get", (k, d))?;
                        if v.is_truthy()? {
                            v.extract()
                        } else {
                            Ok(d)
                        }
                    };
                    Some(crate::model::module(texture.py(), "ingetrazo_core")?.call_method1(
                        "texture_planar_uv",
                        (
                            face.call_method0("normal")?,
                            &pts,
                            getf("sw", 1.)?,
                            getf("sh", 1.)?,
                            getf("rot", 0.)?,
                        ),
                    )?)
                }
            } else {
                None
            };
            for (i, p) in pts.try_iter()?.enumerate() {
                let p = p?;
                stream.push(self.pos.insert(xyz(&p)?));
                let smooth = corners.call_method1("get", (p.call_method0("toTuple")?,))?;
                stream.push(if smooth.is_none() {
                    ni
                } else {
                    self.norm.insert(xyz(&smooth)?)
                });
                if let Some(uv) = &uvs {
                    stream.push(self.uv.insert(uv.get_item(i)?.extract()?));
                }
            }
        }
        Ok(())
    }
    fn encode(&self, g: &str, label: &str) -> (String, String) {
        let mut s = format!("<geometry id=\"{g}\" name=\"{}\"><mesh>", name(label));
        self.pos.encode(g, "pos", "XYZ", &mut s);
        if !self.norm.rows.is_empty() {
            self.norm.encode(g, "nrm", "XYZ", &mut s)
        }
        if !self.uv.rows.is_empty() {
            self.uv.encode(g, "uv", "ST", &mut s)
        }
        write!(s,"<vertices id=\"{g}-vtx\"><input semantic=\"POSITION\" source=\"#{g}-pos\"/></vertices>").unwrap();
        let mut binds = String::new();
        let mut runs = String::new();
        for slot in &self.slots {
            let sym = format!("mat{}", slot.mat);
            let mut inputs=format!("<input semantic=\"VERTEX\" source=\"#{g}-vtx\" offset=\"0\"/><input semantic=\"NORMAL\" source=\"#{g}-nrm\" offset=\"1\"/>");
            write!(
                binds,
                "<instance_material symbol=\"{sym}\" target=\"#{sym}\">"
            )
            .unwrap();
            if slot.textured {
                write!(
                    inputs,
                    "<input semantic=\"TEXCOORD\" source=\"#{g}-uv\" offset=\"2\" set=\"0\"/>"
                )
                .unwrap();
                binds.push_str("<bind_vertex_input semantic=\"UVSET0\" input_semantic=\"TEXCOORD\" input_set=\"0\"/>")
            }
            binds.push_str("</instance_material>");
            if !slot.counts.is_empty() {
                write!(s,"<polylist material=\"{sym}\" count=\"{}\">{inputs}<vcount>{}</vcount><p>{}</p></polylist>",slot.counts.len(),ints(&slot.counts),ints(&slot.poly)).unwrap()
            }
            if !slot.tri.is_empty() {
                write!(
                    s,
                    "<triangles material=\"{sym}\" count=\"{}\">{inputs}<p>{}</p></triangles>",
                    slot.tri.len() / if slot.textured { 9 } else { 6 },
                    ints(&slot.tri)
                )
                .unwrap()
            }
            if !slot.runs.is_empty() {
                write!(
                    runs,
                    "<faces material=\"{sym}\">{}</faces>",
                    ints(&slot.runs)
                )
                .unwrap()
            }
        }
        if !self.lines.is_empty() {
            write!(s,"<lines count=\"{}\"><input semantic=\"VERTEX\" source=\"#{g}-vtx\" offset=\"0\"/><p>{}</p></lines>",self.lines.len()/2,ints(&self.lines)).unwrap()
        }
        let soft = self
            .soft
            .iter()
            .filter_map(|&(a, b)| Some(format!("{} {}", self.pos.lookup(a)?, self.pos.lookup(b)?)))
            .collect::<Vec<_>>();
        if !soft.is_empty() || !runs.is_empty() {
            s.push_str("<extra><technique profile=\"IngeTrazo\">");
            if !soft.is_empty() {
                write!(s, "<soft>{}</soft>", soft.join(" ")).unwrap()
            }
            s.push_str(&runs);
            s.push_str("</technique></extra>")
        }
        s.push_str("</mesh></geometry>");
        let instance = if binds.is_empty() {
            format!("<instance_geometry url=\"#{g}\"/>")
        } else {
            format!("<instance_geometry url=\"#{g}\"><bind_material><technique_common>{binds}</technique_common></bind_material></instance_geometry>")
        };
        (s, instance)
    }
}
#[derive(Eq, Hash, PartialEq)]
struct Signature {
    mesh: usize,
    paint: String,
    children: Vec<(Signature, Option<String>)>,
}
struct Export<'p> {
    scene: Obj<'p>,
    helpers: Bound<'p, PyModule>,
    smoother: Obj<'p>,
    materials: Bound<'p, PyDict>,
    ids: Bound<'p, PyDict>,
    geom: Vec<String>,
    cache: HashMap<(usize, String), String>,
    defs: Vec<String>,
    comps: HashMap<Signature, usize>,
    nodes: usize,
}
fn paint_key(paint: &Obj<'_>) -> PyResult<String> {
    if !paint.is_truthy()? {
        return Ok(String::new());
    }
    if paint.is_instance_of::<PyDict>() {
        Ok(paint
            .py()
            .import("builtins")?
            .call_method1("sorted", (paint.call_method0("items")?,))?
            .repr()?
            .to_string())
    } else {
        Ok(paint.repr()?.to_string())
    }
}
impl<'p> Export<'p> {
    fn visible(&self, e: &Obj<'_>) -> PyResult<bool> {
        self.scene.call_method1("entity_visible", (e,))?.is_truthy()
    }
    fn material(&mut self, attrs: &Obj<'_>) -> PyResult<(usize, bool)> {
        let py = attrs.py();
        let kw = PyDict::new(py);
        kw.set_item("with_opacity", false)?;
        let key = self
            .helpers
            .getattr("material_key")?
            .call((attrs,), Some(&kw))?;
        let tex = key.get_item(0)?.eq("tex")?;
        let id = if let Some(i) = self.ids.get_item(&key)? {
            i.extract()?
        } else {
            let id = self.ids.len();
            let d = PyDict::new(py);
            if tex {
                let src: PathBuf = get(attrs, "texture")?.get_item("path")?.extract()?;
                d.set_item("color", (1., 1., 1.))?;
                d.set_item("map", src.file_name().unwrap_or_default().to_string_lossy())?;
                d.set_item("src", src)?;
            } else {
                let c = get(attrs, "color")?;
                let c = if c.is_truthy()? {
                    c
                } else {
                    self.helpers.getattr("_DEFAULT_COLOR")?
                };
                d.set_item("color", py.get_type::<PyTuple>().call1((c,))?)?;
                d.set_item("map", py.None())?;
            }
            self.materials.set_item(&key, d)?;
            self.ids.set_item(&key, id)?;
            id
        };
        let m = get(attrs, "mat")?;
        if m.is_truthy()? {
            let d = self.materials.get_item(&key)?.unwrap();
            if !get(&d, "mat")?.is_truthy()? {
                d.set_item("mat", m)?
            }
        }
        Ok((id, tex))
    }
    fn geometry(&mut self, mesh: &Obj<'p>, label: &str, paint: &Obj<'p>) -> PyResult<String> {
        let k = (mesh.as_ptr() as usize, paint_key(paint)?);
        if let Some(v) = self.cache.get(&k) {
            return Ok(v.clone());
        }
        let mut geo = Geometry::default();
        for face in mesh.getattr("faces")?.try_iter()? {
            let face = face?;
            if self.visible(&face)? && face.getattr("loop")?.len()? >= 3 {
                geo.append_face(self, &face, paint)?
            }
        }
        for e in mesh.getattr("edges")?.try_iter()? {
            let e = e?;
            let faces = e.getattr("faces")?;
            if faces.is_truthy()? {
                if e.getattr("soft")?.is_truthy()? && faces.len()? == 2 {
                    geo.soft
                        .push((xyz(&e.getattr("a")?)?, xyz(&e.getattr("b")?)?));
                }
            } else if self.visible(&e)? {
                for k in ["a", "b"] {
                    geo.lines.push(geo.pos.insert(xyz(&e.getattr(k)?)?))
                }
            }
        }
        let result = if geo.slots.is_empty() && geo.lines.is_empty() {
            String::new()
        } else {
            let (s, instance) = geo.encode(&format!("geom{}", self.geom.len()), label);
            self.geom.push(s);
            instance
        };
        self.cache.insert(k, result.clone());
        Ok(result)
    }
    fn paint(&self, g: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.scene
            .py()
            .import("core.group")?
            .call_method1("effective_material", (g,))
    }
    fn signature(&self, g: &Obj<'p>, depth: usize) -> PyResult<Signature> {
        if depth > 512 {
            return Err(PyValueError::new_err("group hierarchy too deep"));
        }
        let mut kids = vec![];
        for c in g.getattr("children")?.try_iter()? {
            let c = c?;
            if self.visible(&c)? {
                let m = c.getattr("xform")?;
                kids.push((
                    self.signature(&c, depth + 1)?,
                    if m.is_none() { None } else { Some(matrix(&m)?) },
                ));
            }
        }
        Ok(Signature {
            mesh: g.getattr("mesh")?.as_ptr() as usize,
            paint: paint_key(&self.paint(g)?)?,
            children: kids,
        })
    }
    fn body(&mut self, g: &Obj<'p>, depth: usize) -> PyResult<String> {
        let mut s = self.geometry(
            &g.getattr("mesh")?,
            &text(&g.getattr("name")?)?,
            &self.paint(g)?,
        )?;
        for c in g.getattr("children")?.try_iter()? {
            let c = c?;
            if self.visible(&c)? {
                s.push_str(&self.node(&c, depth + 1)?)
            }
        }
        Ok(s)
    }
    fn node(&mut self, g: &Obj<'p>, depth: usize) -> PyResult<String> {
        if depth > 512 {
            return Err(PyValueError::new_err("group hierarchy too deep"));
        }
        let n = text(&g.getattr("name")?)?;
        let layer = g.getattr("layer")?;
        let layer = if layer.is_truthy()? {
            Some(text(&layer)?)
        } else {
            None
        };
        let bill = g.getattr("billboard")?;
        let bill = if bill.is_truthy()? {
            Some(text(&bill)?)
        } else {
            None
        };
        let mut s = format!("<node id=\"node{}\" name=\"{}\"", self.nodes, name(&n));
        self.nodes += 1;
        if let Some(l) = &layer {
            write!(s, " layer=\"{}\"", name(l)).unwrap()
        }
        s.push('>');
        let m = g.getattr("xform")?;
        if !m.is_none() {
            write!(s, "<matrix>{}</matrix>", matrix(&m)?).unwrap()
        }
        if g.call_method0("is_component")?.is_truthy()? {
            let sig = self.signature(g, depth)?;
            let i = self.comps.len();
            let cid = *self.comps.entry(sig).or_insert(i);
            if cid == i {
                let body = self.body(g, depth)?;
                self.defs.push(format!(
                    "<node id=\"comp{cid}\" name=\"{}\">{body}{}</node>",
                    name(&n),
                    extra(&n, None, None)
                ))
            }
            write!(s, "<instance_node url=\"#comp{cid}\"/>").unwrap()
        } else {
            s.push_str(&self.body(g, depth)?)
        }
        s.push_str(&extra(&n, layer.as_deref(), bill.as_deref()));
        s.push_str("</node>");
        Ok(s)
    }
    fn libraries(&self, path: &Path, out: &mut Vec<String>) -> PyResult<()> {
        let py = self.scene.py();
        let names = self
            .helpers
            .call_method1("export_names", (&self.materials,))?;
        let (mut images, mut effects, mut materials) = (vec![], vec![], vec![]);
        for (i, (key, info)) in self.materials.iter().enumerate() {
            let mid = format!("mat{i}");
            let map = get(&info, "map")?;
            effects.push(format!("<effect id=\"{mid}-fx\"><profile_COMMON>"));
            if map.is_truthy()? {
                let fname = text(&map)?;
                let src: PathBuf = info.get_item("src")?.extract()?;
                let dst = path.with_file_name(&fname);
                if py
                    .import("core.texture")?
                    .call_method1("is_image_file", (&src,))?
                    .is_truthy()?
                    && (dst.canonicalize().ok() != src.canonicalize().ok())
                {
                    let _ = std::fs::copy(&src, &dst);
                }
                images.push(format!(
                    "<image id=\"{mid}-img\" name=\"{mid}-img\"><init_from>{}</init_from></image>",
                    escape(&fname)
                ));
                effects.push(format!("<newparam sid=\"{mid}-surf\"><surface type=\"2D\"><init_from>{mid}-img</init_from></surface></newparam>"));
                effects.push(format!("<newparam sid=\"{mid}-samp\"><sampler2D><source>{mid}-surf</source></sampler2D></newparam>"));
                effects.push(format!("<technique sid=\"common\"><lambert><diffuse><texture texture=\"{mid}-samp\" texcoord=\"UVSET0\"/></diffuse></lambert></technique>"));
            } else {
                let c = info.get_item("color")?;
                let component = |i: usize| c.get_item(i)?.extract::<f64>();
                effects.push(format!("<technique sid=\"common\"><lambert><diffuse><color>{:.4} {:.4} {:.4} 1</color></diffuse></lambert></technique>",component(0)?,component(1)?,component(2)?));
            }
            effects.push("</profile_COMMON></effect>".into());
            let orig = get(&info, "mat")?;
            materials.push(format!("<material id=\"{mid}\" name=\"{}\"><instance_effect url=\"#{mid}-fx\"/>{}</material>",name(&text(&names.get_item(key)?)?),if orig.is_truthy()?{extra(&text(&orig)?,None,None)}else{String::new()}));
        }
        for (tag, rows) in [
            ("images", images),
            ("effects", effects),
            ("materials", materials),
        ] {
            if !rows.is_empty() {
                out.push(format!("<library_{tag}>"));
                out.extend(rows);
                out.push(format!("</library_{tag}>"));
            }
        }
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(scene,path,project_name="IngeTrazo model"))]
fn dae_save<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: PathBuf,
    project_name: &str,
) -> PyResult<()> {
    let h = py.import("formats.meshexport")?;
    let mut exp = Export {
        scene: scene.clone(),
        smoother: h.call_method0("SmoothNormals")?,
        helpers: h,
        ids: PyDict::new(py),
        materials: PyDict::new(py),
        geom: vec![],
        cache: HashMap::new(),
        defs: vec![],
        comps: HashMap::new(),
        nodes: 0,
    };
    let mesh = scene
        .getattr("loose_mesh")
        .or_else(|_| scene.getattr("mesh"))?;
    let loose = exp.geometry(&mesh, project_name, &py.None().into_bound(py))?;
    let mut nodes = vec![];
    if !loose.is_empty() {
        nodes.push(format!(
            "<node id=\"loose\" name=\"{}\">{loose}</node>",
            name(project_name)
        ))
    }
    if let Ok(gs) = scene.getattr("groups") {
        for g in gs.try_iter()? {
            let g = g?;
            if exp.visible(&g)? {
                nodes.push(exp.node(&g, 0)?)
            }
        }
    }
    let time = py.import("datetime")?;
    let kw = PyDict::new(py);
    kw.set_item("microsecond", 0)?;
    let now = time
        .getattr("datetime")?
        .call_method1("now", (time.getattr("timezone")?.getattr("utc")?,))?
        .call_method("replace", (), Some(&kw))?
        .call_method0("isoformat")?
        .extract::<String>()?;
    let mut out: Vec<String> = vec![
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>".into(),
        "<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">".into(),
        "<asset>".into(),
        "<contributor><authoring_tool>IngeTrazo</authoring_tool></contributor>".into(),
        format!("<created>{now}</created><modified>{now}</modified>"),
    ];
    let geo = exp.helpers.call_method1("geolocation", (scene,))?;
    if !geo.is_none() {
        let (lat, lon, alt): (f64, f64, f64) = geo.extract()?;
        out.extend([
            "<coverage><geographic_location>".into(),
            format!("<longitude>{lon:.9}</longitude>"),
            format!("<latitude>{lat:.9}</latitude>"),
            format!("<altitude mode=\"absolute\">{alt:.3}</altitude>"),
            "</geographic_location></coverage>".into(),
        ]);
    }
    out.extend(
        [
            "<unit name=\"meter\" meter=\"1\"/>",
            "<up_axis>Z_UP</up_axis>",
            "</asset>",
        ]
        .map(String::from),
    );
    exp.libraries(&path, &mut out)?;
    for (tag, rows) in [("geometries", exp.geom), ("nodes", exp.defs)] {
        if !rows.is_empty() {
            out.push(format!("<library_{tag}>"));
            out.extend(rows);
            out.push(format!("</library_{tag}>"));
        }
    }
    out.push(format!(
        "<library_visual_scenes><visual_scene id=\"scene\" name=\"{}\">",
        name(project_name)
    ));
    out.extend(nodes);
    out.extend(
        [
            "</visual_scene></library_visual_scenes>",
            "<scene><instance_visual_scene url=\"#scene\"/></scene>",
            "<extra><technique profile=\"IngeTrazo\"><structure>1</structure></technique></extra>",
            "</COLLADA>",
        ]
        .map(String::from),
    );
    py.detach(move || std::fs::write(path, out.join("\n")))?;
    Ok(())
}
pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(dae_save, module)?)
}
