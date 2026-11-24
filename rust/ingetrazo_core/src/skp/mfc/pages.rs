// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene schemas 12, 13 and 15. This tail has a separate class table.
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use std::collections::HashMap;

struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (out, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(out)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn point(&mut self) -> Option<(f64, f64, f64)> {
        Some((self.f64()?, self.f64()?, self.f64()?))
    }
    fn string(&mut self) -> Option<String> {
        if self.take(3)? != [255, 254, 255] {
            return None;
        }
        let count = match self.u8()? {
            255 => match self.u16()? {
                65535 => self.u32()?,
                n => u32::from(n),
            },
            n => u32::from(n),
        };
        Some(
            char::decode_utf16(
                self.take((count as usize).checked_mul(2)?)?
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]])),
            )
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        )
    }
    fn pid(&mut self) -> Option<()> {
        let n = self.u8()?.count_ones();
        self.take(n as usize)?;
        Some(())
    }
    fn tag(&mut self) -> Option<PageTag> {
        Some(match self.u16()? {
            0 => PageTag::Null,
            0xffff => PageTag::Decl,
            0x7fff => {
                let n = self.u32()?;
                if n & 0x80000000 != 0 {
                    PageTag::Class(n & 0x7fffffff)
                } else {
                    PageTag::Object(n)
                }
            }
            n if n & 0x8000 != 0 => PageTag::Class(u32::from(n & 0x7fff)),
            n => PageTag::Object(u32::from(n)),
        })
    }
    fn refs(&mut self) -> Option<Vec<u32>> {
        let n = self.u32()? as usize;
        if n > self.0.len() / 2 {
            return None;
        }
        (0..n)
            .map(|_| match self.tag()? {
                PageTag::Object(n) => Some(n),
                _ => None,
            })
            .collect()
    }
}

enum PageTag {
    Null,
    Decl,
    Class(u32),
    Object(u32),
}
#[derive(Default)]
struct Classes {
    schemas: HashMap<&'static str, u16>,
    slots: HashMap<&'static str, u32>,
}
impl Classes {
    fn schema(&mut self, r: &mut Input, tag: PageTag, name: &'static str) -> Option<u16> {
        match tag {
            PageTag::Decl => {
                let schema = r.u16()?;
                let n = r.u16()? as usize;
                if r.take(n)? != name.as_bytes() {
                    return None;
                }
                self.schemas.insert(name, schema);
            }
            PageTag::Class(s) => {
                if *self.slots.entry(name).or_insert(s) != s {
                    return None;
                }
            }
            _ => return None,
        }
        self.schemas.get(name).copied()
    }
}
struct Camera {
    eye: (f64, f64, f64),
    target: (f64, f64, f64),
    up: (f64, f64, f64),
    fov: f64,
    height: f64,
    parallel: bool,
}
struct Page {
    name: String,
    hidden: Vec<u32>,
    camera: Option<Camera>,
}

fn camera(r: &mut Input) -> Option<Camera> {
    let (eye, target, up) = (r.point()?, r.point()?, r.point()?);
    r.take(16)?;
    let parallel = r.u8()? == 0;
    let (fov, height) = (r.f64()?, r.f64()?);
    r.take(34)?;
    r.string()?;
    r.take(33)?;
    Some(Camera {
        eye,
        target,
        up,
        parallel,
        fov,
        height,
    })
}
fn style(r: &mut Input, schema: u16) -> Option<()> {
    if r.u16()? != 0 {
        return None;
    }
    if schema >= 2 {
        r.pid()?;
    }
    r.take(16)?;
    r.string()?;
    r.take(4)?;
    r.string()?;
    r.string()?;
    for _ in 0..r.u32()? {
        r.take(4)?;
        for _ in 0..r.u32()? {
            let n = match r.u32()? {
                1 => 1,
                3 => 2,
                4..=6 => 4,
                7 => 8,
                _ => return None,
            };
            r.take(n)?;
        }
    }
    Some(())
}
fn page(r: &mut Input, table: &mut Classes, version: u16) -> Option<Page> {
    let tag = r.tag()?;
    let schema = table.schema(r, tag, "CViewPage")?;
    if r.u16()? != 0 {
        return None;
    }
    if schema >= 13 {
        r.pid()?;
    }
    let name = r.string()?;
    r.string()?;
    let flags = r.u32()?;
    if flags & !0x1ff != 0 {
        return None;
    }
    let mut view = None;
    if schema < 15 || flags & 1 != 0 {
        let tag = r.tag()?;
        if matches!(tag, PageTag::Class(_) | PageTag::Decl) {
            table.schema(r, tag, "CCamera")?;
            view = Some(camera(r)?);
        } else if schema >= 15 {
            return None;
        }
    }
    if schema < 15 {
        let tag = r.tag()?;
        if matches!(tag, PageTag::Class(_) | PageTag::Decl) {
            table.schema(r, tag, "CDib")?;
            r.take(4)?;
            let n = r.u32()?;
            r.take(n as usize)?;
        }
        if schema >= 13 {
            r.take(1)?;
        }
    }
    if flags & 2 != 0 {
        let len = match version {
            14..=17 => 206,
            18 => 211,
            19 => 212,
            20 => 213,
            _ => return None,
        };
        r.take(len + if schema >= 15 { 3 } else { 0 })?;
        let tag = r.tag()?;
        if matches!(tag, PageTag::Class(_) | PageTag::Decl) {
            let s = table.schema(r, tag, "CSkpStyle")?;
            style(r, s)?;
        }
    }
    if flags & 4 != 0 {
        r.take(if schema >= 13 { 8 } else { 7 })?;
        r.string()?;
        r.string()?;
        r.take(60)?;
    }
    if flags & 8 != 0 {
        r.take(110 + usize::from(schema >= 13))?;
    }
    if flags & 16 != 0 {
        r.take(1)?;
        r.refs()?;
    }
    let hidden = if flags & 32 != 0 { r.refs()? } else { vec![] };
    if flags & 64 != 0 {
        r.refs()?;
    }
    if schema >= 15 {
        r.take(21)?;
    } else {
        r.take(22)?;
        let n = r.u32()?;
        r.take(n as usize)?;
    }
    Some(Page {
        name,
        camera: view,
        hidden,
    })
}
fn decode(data: &[u8], start: usize, version: u16) -> Option<Vec<Page>> {
    let tail = data.get(start..)?;
    let Some(at) = tail.windows(15).position(|w| {
        w[..2] == [255, 255]
            && matches!(w[2], 12 | 13 | 15)
            && w[3..6] == [0, 9, 0]
            && &w[6..] == b"CViewPage"
    }) else {
        return (!tail.windows(9).any(|w| w == b"CViewPage")).then(Vec::new);
    };
    let at = at + start;
    let mut r = Input(&data[at.checked_sub(4)?..]);
    let count = r.u32()?;
    if !(1..=10000).contains(&count) {
        return None;
    }
    let mut table = Classes::default();
    table.schemas.extend([("CCamera", 5), ("CDib", 3)]);
    let pages = (0..count)
        .map(|_| page(&mut r, &mut table, version))
        .collect::<Option<Vec<_>>>()?;
    matches!(r.tag()?, PageTag::Object(_) | PageTag::Null).then_some(pages)
}

#[pyfunction]
pub(super) fn page_list<'py>(
    data: &Bound<'py, PyBytes>,
    ar: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyList>>> {
    let py = ar.py();
    let raw = data.as_bytes();
    let start = ar.getattr("r")?.getattr("pos")?.extract()?;
    let version = ar.getattr("ver")?.extract()?;
    let Some(pages) = py.detach(|| decode(raw, start, version)) else {
        return Ok(None);
    };
    let out = PyList::empty(py);
    let slots = ar.getattr("slots")?.cast_into::<PyDict>()?;
    for page in pages {
        let row = PyDict::new(py);
        row.set_item("name", page.name)?;
        let mut hidden = Vec::new();
        for id in page.hidden {
            if let Some(rec) = slots.get_item(id)? {
                if rec.get_item(0)?.eq("obj")? && rec.get_item(1)?.eq("CLayer")? {
                    hidden.push(id);
                }
            }
        }
        row.set_item("hidden_layer_ids", hidden)?;
        if let Some(c) = page.camera {
            row.set_item("eye", c.eye)?;
            row.set_item("target", c.target)?;
            row.set_item("up", c.up)?;
            row.set_item("parallel", c.parallel)?;
            row.set_item("ortho_height", c.height)?;
            row.set_item("fov", c.fov)?;
        }
        out.append(row)?;
    }
    Ok(Some(out))
}
