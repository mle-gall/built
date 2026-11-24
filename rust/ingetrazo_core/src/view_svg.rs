// SPDX-License-Identifier: GPL-3.0-or-later
//! Painted face, texture, photograph and shadow layers of a view SVG.
use crate::sheets::{
    dxf::fmt_gp,
    svg::{self, quoteattr, xml_id, Layout, Options},
};
use pyo3::{prelude::*, types::PyDict};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    path::PathBuf,
};
type Obj<'p> = Bound<'p, PyAny>;
type Rings = Vec<Vec<[f64; 2]>>;
fn encode(img: &Obj<'_>, alpha: Option<bool>) -> PyResult<String> {
    let py = img.py();
    let (alpha, data) =
        if img.is_instance(&py.import("ingetrazo_render")?.getattr("SurveyRaster")?)? {
            let alpha = alpha.unwrap_or(true);
            (alpha, img.call_method0(if alpha { "png" } else { "jpeg" })?)
        } else {
            let qt = py.import("PySide6.QtCore")?;
            let alpha = img.call_method0("hasAlphaChannel")?.is_truthy()?;
            let bytes = qt.call_method0("QByteArray")?;
            let buffer = qt.call_method1("QBuffer", (&bytes,))?;
            buffer.call_method1("open", (qt.getattr("QIODevice")?.getattr("WriteOnly")?,))?;
            let res = img.call_method1(
                "save",
                (
                    &buffer,
                    if alpha { "PNG" } else { "JPG" },
                    if alpha { -1 } else { 90 },
                ),
            );
            buffer.call_method0("close")?;
            res?;
            (
                alpha,
                py.import("builtins")?.call_method1("bytes", (bytes,))?,
            )
        };
    let b64 = py
        .import("base64")?
        .call_method1("b64encode", (data,))?
        .call_method0("decode")?
        .extract::<String>()?;
    Ok(format!(
        "data:image/{};base64,{b64}",
        if alpha { "png" } else { "jpeg" }
    ))
}
fn texture<'p>(py: Python<'p>, path: &Obj<'p>, shade: f64) -> PyResult<Obj<'p>> {
    py.import("ingetrazo_render")?
        .getattr("SvgTexture")?
        .call_method1("open", (path, shade, 1024))
}
#[pyfunction]
fn svg_image_uri(py: Python<'_>, path: &Obj<'_>, shade: f64) -> PyResult<Option<String>> {
    let img = texture(py, path, shade)?;
    if img.is_none() {
        Ok(None)
    } else {
        Ok(Some(encode(
            &img.call_method0("raster")?,
            Some(img.getattr("alpha")?.extract()?),
        )?))
    }
}
fn project<'p>(
    pins: &Obj<'p>,
    map: impl Fn(f64, f64) -> PyResult<(f64, f64)>,
) -> PyResult<Obj<'p>> {
    let mut src = vec![];
    let mut dest = vec![];
    for pin in pins.try_iter()? {
        let ((x, y), (u, v)): ((f64, f64), (f64, f64)) = pin?.extract()?;
        src.push((u, -v));
        dest.push(map(x, y)?);
    }
    pins.py()
        .import("ingetrazo_render")?
        .getattr("UvProjection")?
        .call_method1("pins", (src, dest))
}
#[pyfunction]
fn svg_uv_to_page<'p>(pins: &Obj<'p>, px: &Obj<'p>, py: &Obj<'p>) -> PyResult<Obj<'p>> {
    project(pins, |x, y| {
        Ok((px.call1((x,))?.extract()?, py.call1((y,))?.extract()?))
    })
}
fn path(rings: &Rings, l: &Layout) -> String {
    rings
        .iter()
        .map(|ring| {
            let mut s = String::from("M");
            for (i, [x, y]) in ring.iter().enumerate() {
                if i != 0 {
                    s.push_str(" L");
                }
                write!(s, "{:.3},{:.3}", l.px(*x), l.py(*y)).unwrap();
            }
            s.push_str(" Z");
            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}
struct Surface<'p> {
    f: Obj<'p>,
    rings: Rings,
}
#[derive(Default)]
struct Layers {
    keys: HashMap<String, usize>,
    rows: Vec<(String, String)>,
}
impl Layers {
    fn push(&mut self, name: String, s: &str) {
        let i = *self.keys.entry(name.clone()).or_insert_with(|| {
            self.rows.push((name, String::new()));
            self.rows.len() - 1
        });
        self.rows[i].1.push_str(s)
    }
}
struct Canvas<'p> {
    faces: Vec<Surface<'p>>,
    picture: Option<(Obj<'p>, [f64; 4])>,
    bake: f64,
    seam: f64,
    clip: &'static str,
}
impl Canvas<'_> {
    fn render(&self, l: &Layout, taken: &mut HashSet<String>) -> PyResult<String> {
        let mut out = String::new();
        let clip = self.clip;
        if let Some((img, [x0, y0, x1, y1])) = &self.picture {
            let id = xml_id("VISTA-IMAGEN", taken);
            let uri = encode(img, None)?;
            writeln!(out,"<g id=\"{id}\" inkscape:groupmode=\"layer\" inkscape:label=\"VISTA-IMAGEN\"{clip}>\n<image x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" preserveAspectRatio=\"none\" xlink:href=\"{uri}\"/>\n</g>",l.px(*x0),l.py(*y1),(x1-x0)*l.k,(y1-y0)*l.k).unwrap();
        }
        let mut defs = vec![];
        let mut textures = HashMap::new();
        let mut ground = String::new();
        let mut layers = [Layers::default(), Layers::default(), Layers::default()];
        for face in &self.faces {
            let f = &face.f;
            let d = path(&face.rings, l);
            let opacity = f.getattr("opacity")?.extract::<f64>()?;
            if f.getattr("extra")?
                .call_method1("get", ("ground_shadow",))?
                .is_truthy()?
            {
                writeln!(
                    ground,
                    "<path d=\"{d}\" fill=\"#000000\" fill-opacity=\"{}\"/>",
                    fmt_gp(opacity, 3)
                )
                .unwrap();
                continue;
            }
            let layer: String = f.getattr("layer")?.extract()?;
            let shadow = f.getattr("shadow")?;
            if shadow.is_truthy()? {
                let alpha = f.getattr("shadow_alpha")?.extract::<f64>()?;
                layers[2].push(
                    format!("{layer}-SOMBRAS"),
                    &format!(
                        "<path d=\"{}\" fill=\"#000000\" fill-opacity=\"{}\"/>\n",
                        path(&shadow.extract()?, l),
                        fmt_gp(alpha, 3)
                    ),
                );
            }
            let color = svg::hex(f.getattr("rgb")?.extract::<Option<[f64; 3]>>()?);
            let mut fill = color.clone();
            let tex = f.getattr("texture")?;
            if !tex.is_none() {
                let path = tex.get_item("path")?;
                let shade = tex.call_method1("get", ("shade", 1.))?.extract::<f64>()?;
                let key = (path.str()?.to_string(), format!("{shade:.5}"));
                let img = match textures.entry(key) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(texture(f.py(), &path, shade)?)
                    }
                };
                if !img.is_none() {
                    let projection = project(&tex.get_item("uv")?, |x, y| Ok((l.px(x), l.py(y))))?;
                    if !projection.is_none() {
                        let rings = face
                            .rings
                            .iter()
                            .map(|r| {
                                r.iter()
                                    .map(|[x, y]| (l.px(*x), l.py(*y)))
                                    .collect::<Vec<_>>()
                            })
                            .collect::<Vec<_>>();
                        let baked =
                            img.call_method1("bake", (projection, rings, self.bake, 2048))?;
                        if !baked.is_none() {
                            let ((x, y), (w, h), pixels): ((f64, f64), (f64, f64), Obj<'_>) =
                                baked.extract()?;
                            let uri = encode(&pixels, Some(img.getattr("alpha")?.extract()?))?;
                            let id = xml_id(&format!("cara_{}", defs.len() + 1), taken);
                            defs.push(format!("<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"{:.3}\" height=\"{:.3}\"><image x=\"{x:.3}\" y=\"{y:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" preserveAspectRatio=\"none\" xlink:href=\"{uri}\"/></pattern>\n",l.width,l.height));
                            fill = format!("url(#{id})");
                        }
                    }
                }
            }
            let see = opacity < 0.999;
            let attr = if see {
                format!("fill-opacity=\"{}\"", fmt_gp(opacity, 3))
            } else {
                format!(
                    "stroke=\"{color}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",
                    fmt_gp(self.seam, 6)
                )
            };
            layers[usize::from(see)].push(
                format!("{layer}-CARAS{}", if see { "-TRANSP" } else { "" }),
                &format!("<path d=\"{d}\" fill=\"{fill}\" {attr}/>\n"),
            );
        }
        if !defs.is_empty() {
            out.push_str("<defs>\n");
            for d in defs {
                out.push_str(&d);
            }
            out.push_str("</defs>\n");
        }
        let mut layer = |name: &str, rows: &str| {
            let id = xml_id(name, taken);
            writeln!(out,"<g id=\"{id}\" inkscape:groupmode=\"layer\" inkscape:label={}{clip} fill-rule=\"evenodd\">\n{rows}</g>",quoteattr(name)).unwrap();
        };
        if !ground.is_empty() {
            layer("SOMBRAS-SUELO", &ground);
        }
        for set in layers {
            for (name, rows) in set.rows {
                layer(&name, &rows);
            }
        }
        Ok(out)
    }
}
#[pyfunction]
#[pyo3(signature=(groups,scale_n=100.,pens=None,faces=None,line_colours=true,image=None,paper=None,bake_px_per_mm=None,window=None,margin=None,seam_mm=None))]
#[allow(clippy::too_many_arguments)]
fn view_svg_text<'p>(
    groups: &Obj<'p>,
    scale_n: f64,
    pens: Option<&Bound<'p, PyDict>>,
    faces: Option<&Obj<'p>>,
    line_colours: bool,
    image: Option<&Obj<'p>>,
    paper: Option<[f64; 2]>,
    bake_px_per_mm: Option<f64>,
    window: Option<[f64; 4]>,
    margin: Option<f64>,
    seam_mm: Option<f64>,
) -> PyResult<(String, usize)> {
    let groups = groups
        .try_iter()?
        .map(|g| svg::group_of(&g?))
        .collect::<PyResult<Vec<_>>>()?;
    let mut weights = svg::DEFAULT_PENS;
    if let Some(p) = pens {
        for (i, k) in ["cut", "profile", "edge"].into_iter().enumerate() {
            if let Some(v) = p.get_item(k)? {
                weights[i] = v.extract()?;
            }
        }
    }
    let img = image
        .map(|p| -> PyResult<_> { Ok((p.get_item(0)?, p.get_item(1)?.extract::<[f64; 4]>()?)) })
        .transpose()?;
    let fixed = if window.is_none() {
        img.as_ref().map(|i| i.1)
    } else {
        None
    };
    let picture = if let Some((img, rect)) = img {
        if img.is_none() || img.hasattr("isNull")? && img.call_method0("isNull")?.is_truthy()? {
            None
        } else {
            Some((img, rect))
        }
    } else {
        None
    };
    let measure = window.is_none() && image.is_none();
    let mut extra: Option<[f64; 4]> = None;
    let mut surfaces = vec![];
    if let Some(fs) = faces {
        for f in fs.try_iter()? {
            let f = f?;
            let rings = f.getattr("rings")?.extract::<Rings>()?;
            if measure {
                for &[x, y] in rings.iter().flatten() {
                    let r = extra.get_or_insert([x, y, x, y]);
                    r[0] = r[0].min(x);
                    r[1] = r[1].min(y);
                    r[2] = r[2].max(x);
                    r[3] = r[3].max(y);
                }
            }
            surfaces.push(Surface { f, rings });
        }
    }
    let o = Options {
        scale_n,
        pens: weights,
        line_colours,
        paper: paper.map(|[w, h]| (w, h)),
        window,
        fixed,
        extra,
        margin,
    };
    if picture.is_none() && surfaces.is_empty() {
        return Ok(svg::svg_text(&groups, &o, None));
    }
    let canvas = Canvas {
        faces: surfaces,
        picture,
        bake: bake_px_per_mm.filter(|v| *v != 0.).unwrap_or(8.),
        seam: seam_mm.filter(|v| *v != 0.).unwrap_or(0.15),
        clip: if window.is_some() {
            " clip-path=\"url(#pagina)\""
        } else {
            ""
        },
    };
    let mut error = None;
    let out = svg::svg_text(
        &groups,
        &o,
        Some(&mut |l, taken| {
            canvas.render(l, taken).unwrap_or_else(|e| {
                error = Some(e);
                String::new()
            })
        }),
    );
    if let Some(e) = error {
        Err(e)
    } else {
        Ok(out)
    }
}
#[pyfunction]
#[pyo3(signature=(path,groups,scale_n=100.,pens=None,faces=None,line_colours=true,image=None,paper=None))]
#[allow(clippy::too_many_arguments)]
fn view_svg_save<'p>(
    path: PathBuf,
    groups: &Obj<'p>,
    scale_n: f64,
    pens: Option<&Bound<'p, PyDict>>,
    faces: Option<&Obj<'p>>,
    line_colours: bool,
    image: Option<&Obj<'p>>,
    paper: Option<[f64; 2]>,
) -> PyResult<usize> {
    let (svg, n) = view_svg_text(
        groups,
        scale_n,
        pens,
        faces,
        line_colours,
        image,
        paper,
        None,
        None,
        None,
        None,
    )?;
    groups.py().detach(|| std::fs::write(path, svg))?;
    Ok(n)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(view_svg_text, m)?)?;
    m.add_function(wrap_pyfunction!(view_svg_save, m)?)?;
    m.add_function(wrap_pyfunction!(svg_uv_to_page, m)?)?;
    m.add_function(wrap_pyfunction!(svg_image_uri, m)?)
}
