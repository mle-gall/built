// SPDX-License-Identifier: GPL-3.0-or-later
//! Record formats adapted from OpenSKP (MIT, Copyright 2024–2026 Ahsan Mehmood).
//! The accompanying license is in `tests/data/openskp-LICENSE.txt`.
//! Little-endian MFC CArchive records share one file-wide class/object table.
mod archive;
mod builder;
mod definitions;
mod pages;
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    prelude::*,
    types::{PyBytes, PyTuple},
};

#[pyclass(module = "built_core")]
pub struct SkpArchiveCursor {
    #[pyo3(get)]
    data: Py<PyBytes>,
    #[pyo3(get, set)]
    pos: usize,
}

impl SkpArchiveCursor {
    fn bytes<'a>(&'a self, py: Python<'a>, n: usize) -> PyResult<&'a [u8]> {
        self.data
            .bind(py)
            .as_bytes()
            .get(self.pos..)
            .and_then(|s| s.get(..n))
            .ok_or_else(|| {
                PyIndexError::new_err(format!("truncated MFC archive @0x{:x}", self.pos))
            })
    }
    fn word<const N: usize>(&mut self, py: Python<'_>) -> PyResult<[u8; N]> {
        let raw = self.bytes(py, N)?.try_into().unwrap();
        self.pos += N;
        Ok(raw)
    }
    fn string_count(&mut self, py: Python<'_>) -> PyResult<usize> {
        let n = self.u8(py)?;
        if n != 255 {
            return Ok(n.into());
        }
        let n = self.u16(py)?;
        if n != 65535 {
            return Ok(n.into());
        }
        Ok(self.u32(py)? as usize)
    }
}

#[pymethods]
impl SkpArchiveCursor {
    #[new]
    #[pyo3(signature=(data,pos=0))]
    fn new(data: Py<PyBytes>, pos: usize) -> Self {
        Self { data, pos }
    }
    fn u8(&mut self, py: Python<'_>) -> PyResult<u8> {
        Ok(self.word::<1>(py)?[0])
    }
    fn u16(&mut self, py: Python<'_>) -> PyResult<u16> {
        Ok(u16::from_le_bytes(self.word(py)?))
    }
    fn u32(&mut self, py: Python<'_>) -> PyResult<u32> {
        Ok(u32::from_le_bytes(self.word(py)?))
    }
    fn f64(&mut self, py: Python<'_>) -> PyResult<f64> {
        Ok(f64::from_le_bytes(self.word(py)?))
    }
    fn f64s<'py>(&mut self, py: Python<'py>, n: usize) -> PyResult<Bound<'py, PyTuple>> {
        let len = n
            .checked_mul(8)
            .ok_or_else(|| PyValueError::new_err("invalid double count"))?;
        let result = PyTuple::new(
            py,
            self.bytes(py, len)?
                .chunks_exact(8)
                .map(|v| f64::from_le_bytes(v.try_into().unwrap())),
        )?;
        self.pos += len;
        Ok(result)
    }
    fn raw<'py>(&mut self, py: Python<'py>, n: usize) -> PyResult<Bound<'py, PyBytes>> {
        let v = PyBytes::new(py, self.bytes(py, n)?);
        self.pos += n;
        Ok(v)
    }
    fn peek<'py>(&self, py: Python<'py>, n: usize) -> Bound<'py, PyBytes> {
        let tail = self
            .data
            .bind(py)
            .as_bytes()
            .get(self.pos..)
            .unwrap_or_default();
        PyBytes::new(py, &tail[..n.min(tail.len())])
    }
    fn peek_u16(&self, py: Python<'_>) -> PyResult<u16> {
        Ok(u16::from_le_bytes(self.bytes(py, 2)?.try_into().unwrap()))
    }
    fn utf16(&mut self, py: Python<'_>) -> PyResult<String> {
        if self.word::<3>(py)? != [0xff, 0xfe, 0xff] {
            return Err(archive_error(
                py,
                format!("expected a string record @0x{:x}", self.pos - 3),
            ));
        }
        let n = self.string_count(py)?;
        let len = n
            .checked_mul(2)
            .ok_or_else(|| PyValueError::new_err("invalid string length"))?;
        let units = self
            .bytes(py, len)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()));
        let result = char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        self.pos += len;
        Ok(result)
    }
    #[pyo3(signature=(back=16,fwd=32))]
    fn ctx(&self, py: Python<'_>, back: usize, fwd: usize) -> String {
        let bytes = self.data.bind(py).as_bytes();
        let at = self.pos.min(bytes.len());
        let hex = |s: &[u8]| {
            s.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        format!(
            "@0x{:x}: …{} | {}…",
            self.pos,
            hex(&bytes[at.saturating_sub(back)..at]),
            hex(&bytes[at..at.saturating_add(fwd).min(bytes.len())])
        )
    }
}

fn archive_error(py: Python<'_>, why: String) -> PyErr {
    match py
        .import("openskp.legacy")
        .and_then(|m| m.getattr("LegacyParseError"))
        .and_then(|c| c.call1((why,)))
    {
        Ok(v) => PyErr::from_value(v),
        Err(e) => e,
    }
}

enum Tag {
    Null,
    Declaration(String, u16),
    Class(u32),
    Object(u32),
}
fn walker_slot(archive: &Bound<'_, PyAny>, slot: u32) -> PyResult<Option<i64>> {
    let mut walker = i64::from(slot);
    for band in archive.getattr("burns")?.try_iter()? {
        let (start, width) = band?.extract::<(i64, i64)>()?;
        if i64::from(slot) < start {
            break;
        }
        if i64::from(slot) < start + width {
            return Ok(None);
        }
        walker -= width;
    }
    Ok(Some(walker))
}
fn tag(r: &mut SkpArchiveCursor, py: Python<'_>) -> PyResult<Tag> {
    Ok(match r.u16(py)? {
        0 => Tag::Null,
        0xffff => {
            let schema = r.u16(py)?;
            let n = usize::from(r.u16(py)?);
            if n > 40 {
                return Err(archive_error(
                    py,
                    format!("invalid class name length @0x{:x}", r.pos),
                ));
            }
            let name = r.bytes(py, n)?;
            if !name.is_ascii() {
                return Err(archive_error(
                    py,
                    format!("invalid class name @0x{:x}", r.pos),
                ));
            }
            let name = std::str::from_utf8(name).unwrap().to_owned();
            r.pos += n;
            Tag::Declaration(name, schema)
        }
        0x7fff => {
            let n = r.u32(py)?;
            if n & 0x80000000 != 0 {
                Tag::Class(n & 0x7fffffff)
            } else {
                Tag::Object(n)
            }
        }
        n if n & 0x8000 != 0 => Tag::Class((n & 0x7fff).into()),
        n => Tag::Object(n.into()),
    })
}

#[pyfunction]
#[pyo3(signature=(archive,r,expect=None))]
pub fn skp_mfc_object<'py>(
    py: Python<'py>,
    archive: &Bound<'py, PyAny>,
    r: &Bound<'py, SkpArchiveCursor>,
    expect: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    crate::file_open::skp_checkpoint(py)?;
    let cursor = || r.borrow().pos;
    let fail = |msg: &str| archive_error(py, format!("{msg} @0x{:x}", cursor()));
    let slots = archive
        .getattr("slots")?
        .cast_into::<pyo3::types::PyDict>()?;
    let classes = archive.getattr("class_slot")?;
    let record = tag(&mut *r.try_borrow_mut()?, py)?;
    let class = match record {
        Tag::Null => {
            return Ok((py.None(), py.None(), py.None())
                .into_pyobject(py)?
                .into_any())
        }
        Tag::Object(slot) => {
            let Some(walker) = walker_slot(archive, slot)? else {
                return Ok((slot, "reserved", py.None()).into_pyobject(py)?.into_any());
            };
            if let Some(entry) = slots.get_item(walker)? {
                if entry.get_item(0)?.eq("class")? {
                    return Err(fail("back-reference to a class"));
                }
                return Ok((walker, entry.get_item(1)?, entry.get_item(2)?)
                    .into_pyobject(py)?
                    .into_any());
            }
            if walker < archive.getattr("walk_base")?.extract::<i64>()? {
                return Ok((walker, "premodel", py.None())
                    .into_pyobject(py)?
                    .into_any());
            }
            return Err(fail("unresolved object back-reference"));
        }
        Tag::Class(slot) => {
            if let Some(entry) = slots.get_item(slot)? {
                if !entry.get_item(0)?.eq("class")? {
                    return Err(fail("class reference points to an object"));
                }
                entry.get_item(1)?.extract::<String>()?
            } else {
                let name = expect
                    .ok_or_else(|| fail("unresolved class reference"))?
                    .extract::<String>()?;
                slots.set_item(slot, ("class", &name, py.None()))?;
                classes.set_item(&name, slot)?;
                name
            }
        }
        Tag::Declaration(name, schema) => {
            let slot = archive.call_method1("alloc", (("class", &name, schema),))?;
            classes.set_item(&name, slot)?;
            archive.getattr("class_schema")?.set_item(&name, schema)?;
            name
        }
    };
    archive.setattr("_as_item", archive.getattr("in_entity_list")?)?;
    archive.setattr("in_entity_list", false)?;
    let slot = archive.call_method1("alloc", (("obj", &class, py.None()),))?;
    let reader = archive.getattr("readers")?.call_method1("get", (&class,))?;
    if reader.is_none() {
        return Err(fail(&format!("unknown class {class}")));
    }
    let prev_class = archive.getattr("current_class")?;
    let prev_slot = archive.getattr("current_obj_slot")?;
    archive.setattr("current_class", &class)?;
    archive.setattr("current_obj_slot", &slot)?;
    let value = reader.call1((archive, r));
    archive.setattr("current_class", prev_class)?;
    archive.setattr("current_obj_slot", prev_slot)?;
    let value = value?;
    slots.set_item(&slot, ("obj", &class, &value))?;
    if matches!(class.as_str(), "CDimensionLinear" | "CText") {
        archive.setattr("annot_watermark", archive.getattr("next_slot")?)?;
    }
    Ok((slot, class, value).into_pyobject(py)?.into_any())
}

/// No cursor borrow crosses a nested object read. Its body may define a class
/// and recurse into this same cursor before returning to the containing record.
struct Entity<'a, 'py> {
    ar: &'a Bound<'py, PyAny>,
    r: &'a Bound<'py, SkpArchiveCursor>,
    py: Python<'py>,
    ver: u16,
}
impl<'a, 'py> Entity<'a, 'py> {
    fn object(&self, class: Option<&str>) -> PyResult<Bound<'py, PyAny>> {
        self.ar.call_method1("read_object", (self.r, class))
    }
    fn u8(&self) -> PyResult<u8> {
        self.r.borrow_mut().u8(self.py)
    }
    fn u16(&self) -> PyResult<u16> {
        self.r.borrow_mut().u16(self.py)
    }
    fn u32(&self) -> PyResult<u32> {
        self.r.borrow_mut().u32(self.py)
    }
    fn peek(&self) -> PyResult<u16> {
        self.r.borrow().peek_u16(self.py)
    }
    fn doubles(&self, n: usize) -> PyResult<Bound<'py, PyTuple>> {
        self.r.borrow_mut().f64s(self.py, n)
    }
    fn string(&self) -> PyResult<String> {
        self.r.borrow_mut().utf16(self.py)
    }
    fn skip(&self, n: usize) -> PyResult<()> {
        let mut r = self.r.borrow_mut();
        r.bytes(self.py, n)?;
        r.pos += n;
        Ok(())
    }
    fn error(&self, msg: &str) -> PyErr {
        archive_error(self.py, format!("{} @0x{:x}", msg, self.r.borrow().pos))
    }
    fn preamble(&self) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>, u64)> {
        let attributes = self.object(Some("CAttributeContainer"))?;
        let mut pid = 0;
        if self.ar.getattr("has_pid")?.is_truthy()? {
            let mask = self.u8()?;
            for bit in 0..8 {
                if mask & (1 << bit) != 0 {
                    pid |= u64::from(self.u8()?) << (8 * bit);
                }
            }
        }
        Ok((attributes.get_item(0)?, attributes.get_item(2)?, pid))
    }
    fn draw(&self) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let n = if self.ver < 4 {
            5
        } else if self.ver < 5 {
            7
        } else {
            8
        };
        let mut bytes = [0; 8];
        {
            let mut r = self.r.borrow_mut();
            bytes[..n].copy_from_slice(r.bytes(self.py, n)?);
            r.pos += n;
        }
        let tag = self.peek()?;
        let layer = if self
            .ar
            .getattr("class_slot")?
            .call_method1("get", ("CLayer",))?
            .extract::<Option<u32>>()?
            .is_some_and(|id| u32::from(tag) == 0x8000 | id)
        {
            self.object(Some("CLayer"))?;
            0
        } else if tag == 0x7fff {
            self.u16()?;
            if self.u32()? & 0x80000000 != 0 {
                return Err(self.error("unexpected inline layer class"));
            }
            0
        } else {
            self.u16()?
        };
        if self.ver < 4 {
            bytes = [0; 8];
        }
        let db = pyo3::types::PyDict::new(self.py);
        db.set_item("layer", layer)?;
        db.set_item("mat", u16::from_le_bytes([bytes[0], bytes[1]]))?;
        db.set_item("hidden", bytes[2])?;
        db.set_item("soft", bytes[5])?;
        db.set_item("smooth", bytes[6])?;
        Ok(db)
    }
    fn guid(&self) -> PyResult<String> {
        let bytes = self.r.borrow_mut().word::<16>(self.py)?;
        use std::fmt::Write;
        let mut guid = String::with_capacity(32);
        for b in bytes {
            write!(guid, "{b:02X}").unwrap();
        }
        Ok(guid)
    }
    fn texture(&self, out: &Bound<'py, pyo3::types::PyDict>) -> PyResult<()> {
        let (slot, dib) = if self.ver < 6 {
            (
                self.py.None().into_bound(self.py),
                self.read("CDib")?.into_any(),
            )
        } else {
            self.r.borrow_mut().pos -= 1;
            self.preamble()?;
            let rec = self.object(Some("CDib"))?;
            (rec.get_item(0)?, rec.get_item(2)?)
        };
        let window = self.r.borrow().peek(self.py, 28);
        match window
            .as_bytes()
            .windows(3)
            .position(|w| w == [0xff, 0xfe, 0xff])
        {
            Some(16) => (),
            Some(20) => self.skip(4)?,
            _ => return Err(self.error("misaligned texture size")),
        }
        out.set_item(
            "tex_data",
            if slot.is_none() {
                dib.get_item("data")?
            } else {
                self.py.None().into_bound(self.py)
            },
        )?;
        out.set_item("tex_dib", slot)?;
        for field in ["tex_w", "tex_h"] {
            out.set_item(field, self.r.borrow_mut().f64(self.py)?)?;
        }
        out.set_item("tex_file", self.string()?)?;
        let colors = self.r.borrow_mut().word::<9>(self.py)?;
        out.set_item("rgba", (colors[5], colors[6], colors[7], 255))?;
        self.string()?;
        let flags = self.r.borrow_mut().word::<8>(self.py)?;
        out.set_item("colorized", flags[0] == 2)?;
        out.set_item("colorize_type", u8::from(flags[4] == 1))?;
        out.set_item("opacity", self.r.borrow_mut().f64(self.py)?)?;
        out.set_item("use_opacity", self.u8()?)
    }

    fn entity_ref(&self, allow_inline: bool, member: bool) -> PyResult<Option<u32>> {
        let tag = self.peek()?;
        let escaped = tag == 0x7fff;
        let value = if escaped {
            u32::from_le_bytes(self.r.borrow().bytes(self.py, 6)?[2..6].try_into().unwrap())
        } else {
            u32::from(tag)
        };
        if value & (if escaped { 0x80000000 } else { 0x8000 }) != 0 {
            if !allow_inline {
                return Err(self.error("expected an entity reference"));
            }
            let rec = self.object(None)?;
            let slot = rec.get_item(0)?;
            if member {
                if let Some(stack) = self.ar.getattr_opt("inline_stack")? {
                    if stack.is_truthy()? {
                        stack.get_item(-1)?.set_item(&slot, self.py.None())?;
                    }
                }
            }
            return slot.extract();
        }
        self.skip(if escaped { 6 } else { 2 })?;
        Ok((tag != 0).then_some(value))
    }
    fn paths(&self) -> PyResult<(Option<u32>, Vec<Option<u32>>, Vec<Option<u32>>)> {
        let extra = self.entity_ref(false, false)?;
        let read_list = || -> PyResult<Vec<Option<u32>>> {
            let count = self.u32()?;
            if count > 1000 {
                return Err(self.error("invalid annotation path length"));
            }
            (0..count).map(|i| self.entity_ref(true, i == 0)).collect()
        };
        Ok((extra, read_list()?, read_list()?))
    }

    fn dimension(&self, out: &Bound<'py, pyo3::types::PyDict>) -> PyResult<()> {
        out.set_item("db", self.draw()?)?;
        out.set_item("text", self.string()?)?;
        if self.ver >= 6 {
            self.object(Some("CSkFont"))?;
            self.skip(5)?;
        }
        let endpoint = |key| -> PyResult<_> {
            let typ = self.u32()?;
            self.skip(4)?;
            let p = self.doubles(3)?;
            if typ == 1 {
                out.set_item(key, p)?;
            }
            Ok((self.entity_ref(true, false)?, self.paths()?))
        };
        let (a, pa) = endpoint("a")?;
        let (b, pb) = endpoint("b")?;
        out.set_item("connect", (a, b))?;
        out.set_item("paths", (pa, pb))?;
        self.skip(52)?;
        out.set_item("offset", self.r.borrow_mut().f64(self.py)?)?;
        self.skip(if self.ver >= 6 { 12 } else { 8 })
    }

    fn text(&self, out: &Bound<'py, pyo3::types::PyDict>) -> PyResult<()> {
        out.set_item("db", self.draw()?)?;
        self.object(Some("CSkFont"))?;
        let prefix = self.r.borrow().peek(self.py, 24);
        let anchored = prefix.as_bytes().get(20..24) == Some(&4u32.to_le_bytes())
            && (2..=15).contains(&u32::from_le_bytes(
                prefix.as_bytes()[16..20].try_into().unwrap(),
            ));
        if anchored {
            self.skip(48)?;
            out.set_item("connect", self.entity_ref(false, false)?)?;
            out.set_item("paths", self.paths()?)?;
        } else {
            out.set_item("connect", self.py.None())?;
            out.set_item("paths", self.py.None())?;
        }
        let block = self.r.borrow().peek(self.py, 512);
        let bytes = block.as_bytes();
        let str_off = bytes
            .windows(3)
            .enumerate()
            .find_map(|(i, w)| {
                if w != [0xff, 0xfe, 0xff] || i < 11 {
                    return None;
                }
                let b = &bytes[i - 11..i];
                (b[..4] == 1u32.to_le_bytes()
                    && u32::from_le_bytes(b[6..10].try_into().unwrap()) <= 8
                    && b[10] <= 1)
                    .then_some(i)
            })
            .ok_or_else(|| self.error("missing text delimiter"))?;
        let mid = &bytes[..str_off];
        let off = (!anchored)
            .then(|| mid.windows(8).position(|w| w == [1, 0, 0, 0, 4, 0, 0, 0]))
            .flatten();
        let xyz = |at: usize| mid.get(at..at + 24).and_then(super::point);
        let point = off.and_then(|i| xyz(i + 8));
        let label = off
            .and_then(|i| xyz(i + 44))
            .filter(|(a, b, c)| a.abs() > 1e-12 || b.abs() > 1e-12 || c.abs() > 1e-12);
        out.set_item("p", point)?;
        out.set_item("lp", label)?;
        self.skip(str_off)?;
        out.set_item("text", self.string()?)?;
        self.skip(5)?;
        let mut attachments = Vec::new();
        loop {
            let b = self.r.borrow().peek(self.py, 6);
            let b = b.as_bytes();
            if b.len() != 6 || b[..2] != [0xff, 0x7f] {
                break;
            }
            let v = u32::from_le_bytes(b[2..].try_into().unwrap());
            if v & 0x80000000 != 0 {
                break;
            }
            attachments.push(v);
            self.skip(6)?;
        }
        out.set_item("attach", attachments)
    }
    fn read(&self, cls: &str) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        use pyo3::types::{PyDict, PyList};
        let out = PyDict::new(self.py);
        let pre = if (self.ver < 4
            && matches!(
                cls,
                "CVertex" | "CEdgeUse" | "CLoop" | "CMaterial" | "CLayer"
            ))
            || matches!(cls, "CDib" | "CCamera")
        {
            (
                self.py.None().into_bound(self.py),
                self.py.None().into_bound(self.py),
                0,
            )
        } else {
            self.preamble()?
        };
        let kind = match cls {
            "CVertex" => {
                out.set_item("xyz", self.doubles(3)?)?;
                "vertex"
            }
            "CEdge" => {
                out.set_item("db", self.draw()?)?;
                for end in ["v1", "v2"] {
                    out.set_item(end, self.object(Some("CVertex"))?.get_item(0)?)?;
                }
                let curve = self.object(None)?;
                if !matches!(
                    curve.get_item(1)?.extract::<Option<String>>()?.as_deref(),
                    None | Some("CCurve" | "CArcCurve")
                ) {
                    return Err(self.error("invalid edge curve reference"));
                }
                out.set_item("curve", curve.get_item(0)?)?;
                out.set_item("curve_value", curve.get_item(2)?)?;
                "edge"
            }
            "CCurve" => {
                self.skip(1)?;
                out.set_item("n", self.u32()?)?;
                "curve"
            }
            "CArcCurve" => {
                self.skip(5)?;
                let v = self.doubles(14)?;
                let frame = PyDict::new(self.py);
                for (key, i) in [("center", 0), ("normal", 3), ("x_axis", 6), ("y_axis", 11)] {
                    frame.set_item(key, v.get_slice(i, i + 3))?;
                }
                frame.set_item("start_angle", v.get_item(9)?)?;
                frame.set_item("end_angle", v.get_item(10)?)?;
                out.set_item("params", frame)?;
                "arccurve"
            }
            "CEdgeUse" => {
                out.set_item("edge", self.object(Some("CEdge"))?.get_item(0)?)?;
                out.set_item("sense", self.u8()?)?;
                let start = self.r.borrow().pos;
                let parent = match self.u16()? {
                    0 => None,
                    0x7fff => {
                        let s = self.u32()?;
                        if s & 0x80000000 != 0 {
                            return Err(self.error("invalid edge-use parent"));
                        }
                        Some(i64::from(s))
                    }
                    s if s & 0x8000 == 0 => Some(i64::from(s)),
                    _ => return Err(self.error("invalid edge-use parent")),
                };
                let delta = self.ar.getattr("cum_delta")?.extract::<i64>()?;
                let expected = self
                    .ar
                    .getattr("current_loop")?
                    .extract::<Option<i64>>()?
                    .map(|v| v + delta);
                if parent != expected {
                    let burn = parent.zip(expected).map_or(0, |(a, b)| a - b);
                    if (1..=4096).contains(&burn) && !self.ar.getattr("annot_watermark")?.is_none()
                    {
                        let watermark = self.ar.getattr("annot_watermark")?.extract::<i64>()?;
                        self.ar
                            .getattr("burns")?
                            .call_method1("append", ((watermark + delta, burn),))?;
                        self.ar.setattr("cum_delta", delta + burn)?;
                        self.ar.setattr("annot_watermark", self.py.None())?;
                        let credits = self.ar.getattr("burn_stack")?;
                        if credits.is_truthy()? {
                            credits.set_item(-1, credits.get_item(-1)?.extract::<u32>()? + 1)?;
                        }
                    } else {
                        self.r.borrow_mut().pos = start;
                        return Err(self.error("edge-use does not reference its loop"));
                    }
                }
                "edgeuse"
            }
            "CLoop" => {
                self.skip(2)?;
                let prev = self.ar.getattr("current_loop")?;
                // The enclosing object's slot was assigned before its preamble.
                self.ar
                    .setattr("current_loop", self.ar.getattr("current_obj_slot")?)?;
                let result = (|| -> PyResult<_> {
                    let uses = PyList::empty(self.py);
                    while self.peek()? != 0 {
                        uses.append(self.object(Some("CEdgeUse"))?.get_item(2)?)?;
                    }
                    self.skip(2)?;
                    Ok(uses)
                })();
                self.ar.setattr("current_loop", prev)?;
                out.set_item("uses", result?)?;
                "loop"
            }
            "CFace" => {
                let db = self.draw()?;
                let mut attrs = pre.1;
                if self.ver < 4
                    && attrs.cast::<PyDict>().is_ok()
                    && attrs.call_method1("get", ("k",))?.eq("material")?
                {
                    db.set_item("mat", pre.0)?;
                    attrs = self.py.None().into_bound(self.py);
                }
                out.set_item("attrs", attrs)?;
                out.set_item("db", db)?;
                out.set_item("plane", self.doubles(4)?)?;
                let n = self.u32()?;
                if n > 10000 {
                    return Err(self.error("invalid face loop count"));
                }
                let loops = PyList::empty(self.py);
                for _ in 0..n {
                    loops.append(self.object(Some("CLoop"))?.get_item(2)?)?;
                }
                out.set_item("loops", loops)?;
                out.set_item("back_mat", self.u16()?)?;
                "face"
            }
            "CGroup" | "CComponentInstance" => {
                out.set_item("db", self.draw()?)?;
                let def = self.object(Some("CComponentDefinition"))?;
                if !def.get_item(1)?.eq("CComponentDefinition")? {
                    return Err(self.error("invalid placement definition"));
                }
                out.set_item("def", def.get_item(0)?)?;
                out.set_item("xf", self.doubles(13)?)?;
                let named = self.ver >= 5
                    || self.r.borrow().peek(self.py, 3).as_bytes() == [0xff, 0xfe, 0xff];
                out.set_item("name", if named { self.string()? } else { String::new() })?;
                out.set_item(
                    "guid",
                    if self.ver >= 14 {
                        self.guid()?
                    } else {
                        String::new()
                    },
                )?;
                out.set_item("attrs", pre.1)?;
                out.set_item("persistent_id", pre.2)?;
                "instance"
            }
            "CImage" => {
                out.set_item("db", self.draw()?)?;
                out.set_item("def", self.object(None)?.get_item(0)?)?;
                out.set_item("xform", self.doubles(12)?)?;
                self.skip(8)?;
                self.string()?;
                out.set_item("guid", self.guid()?)?;
                "image"
            }
            "CFaceTextureCoords" => {
                self.skip(4)?;
                let values = self.doubles(24)?;
                out.set_item("front", values.get_slice(0, 9))?;
                out.set_item("back", values.get_slice(12, 21))?;
                for key in ["front_pins", "back_pins"] {
                    let list = PyList::empty(self.py);
                    for _ in 0..self.u32()? {
                        list.append(self.doubles(4)?)?;
                    }
                    out.set_item(key, list)?;
                }
                out.set_item("front_projected", self.u32()? & 2 != 0)?;
                out.set_item("back_projected", self.u32()? & 2 != 0)?;
                "ftc"
            }
            "CDib" => {
                out.set_item("subtype", self.u32()?)?;
                let size = self.u32()? as usize;
                out.set_item("data", self.r.borrow_mut().raw(self.py, size)?)?;
                "dib"
            }
            "CCamera" => {
                self.skip(139)?;
                self.string()?;
                self.skip(if self.ver >= 6 { 33 } else { 10 })?;
                "camera"
            }
            "CThumbnail" => {
                let mut dib = self.py.None().into_bound(self.py);
                if self.ver < 4 {
                    self.skip(129)?;
                } else {
                    self.object(Some("CCamera"))?;
                    if self.ver >= 6 {
                        dib = self.object(Some("CDib"))?.get_item(2)?;
                    }
                }
                out.set_item("dib", dib)?;
                "thumbnail"
            }
            "CSkFont" => {
                self.string()?;
                self.skip(15)?;
                "font"
            }
            "CAttributeContainer" => {
                let items = PyList::empty(self.py);
                while self.peek()? != 0 {
                    let attr = self.object(Some("CAttributeNamed"))?;
                    items.append((attr.get_item(1)?, attr.get_item(2)?))?;
                }
                self.skip(2)?;
                out.set_item("children", items)?;
                "attrs"
            }
            "CDimensionLinear" => {
                self.dimension(&out)?;
                "dimension"
            }
            "CAttributeNamed" => {
                self.skip(4)?;
                out.set_item("name", self.string()?)?;
                let entries = PyDict::new(self.py);
                loop {
                    let key = self.string()?;
                    if key.is_empty() {
                        break;
                    }
                    entries.set_item(key, self.typed_attribute(0)?)?;
                }
                if self.ver >= 7 {
                    self.skip(4)?;
                }
                out.set_item("entries", entries)?;
                "dict"
            }
            "CComponentDefinition" => {
                self.definition(&out)?;
                "definition"
            }
            "CConstructionLine" => {
                self.construction_line(&out)?;
                "cline"
            }
            "CSectionPlane" => {
                out.set_item("db", self.draw()?)?;
                let first =
                    f64::from_le_bytes(self.r.borrow().bytes(self.py, 8)?.try_into().unwrap());
                if !(first.abs() <= 1.0001) {
                    self.object(None)?;
                }
                out.set_item("plane", PyList::new(self.py, self.doubles(4)?.iter())?)?;
                let (name, label) =
                    if self.r.borrow().peek(self.py, 3).as_bytes() == [0xff, 0xfe, 0xff] {
                        (self.string()?, self.string()?)
                    } else {
                        (String::new(), String::new())
                    };
                out.set_item("name", name)?;
                out.set_item("label", label)?;
                "sectionplane"
            }
            "CText" => {
                self.text(&out)?;
                "text"
            }
            "CRelationship" => {
                out.set_item(
                    "refs",
                    (
                        self.entity_ref(false, false)?,
                        self.entity_ref(false, false)?,
                    ),
                )?;
                "relationship"
            }
            "CConstructionPoint" => {
                out.set_item("db", self.draw()?)?;
                out.set_item("pos", self.doubles(3)?)?;
                self.skip(25)?;
                "cpoint"
            }
            "CLayer" => {
                out.set_item("name", self.string()?)?;
                let mut hidden = 0;
                for i in 0..8 {
                    if self.r.borrow().peek(self.py, 3).as_bytes() == [0xff, 0xfe, 0xff] {
                        break;
                    }
                    let b = self.u8()?;
                    if i == 0 {
                        hidden = b;
                    }
                }
                out.set_item("hidden", hidden)?;
                self.string()?;
                if self.u16()? & 0xff != 0 {
                    let texture = PyDict::new(self.py);
                    self.texture(&texture)?;
                    out.set_item("rgba", texture.get_item("rgba")?)?;
                    self.skip(4)?;
                } else {
                    out.set_item(
                        "rgba",
                        PyTuple::new(self.py, self.r.borrow_mut().word::<4>(self.py)?)?,
                    )?;
                    self.string()?;
                    self.skip(if self.ver < 4 { 17 } else { 21 })?;
                    self.layer_extension()?;
                }
                if self.ver == 19 && self.r.borrow().peek(self.py, 2).as_bytes() == [0, 0] {
                    self.skip(2)?;
                }
                "layer"
            }
            "CMaterial" => {
                out.set_item("name", self.string()?)?;
                if self.u16()? == 0 {
                    let rgba = self.r.borrow_mut().word::<4>(self.py)?;
                    out.set_item("rgba", PyTuple::new(self.py, rgba)?)?;
                    self.string()?;
                    self.skip(8)?;
                    out.set_item("opacity", self.r.borrow_mut().f64(self.py)?)?;
                    out.set_item("use_opacity", self.u8()?)?;
                } else {
                    self.texture(&out)?;
                }
                "material"
            }
            _ => return Err(self.error("unknown MFC entity")),
        };
        out.set_item("k", kind)?;
        Ok(out)
    }
}

#[pyfunction]
fn skp_mfc_entity<'py>(
    class: &str,
    archive: &Bound<'py, PyAny>,
    r: &Bound<'py, SkpArchiveCursor>,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    Entity {
        py: r.py(),
        ar: archive,
        r,
        ver: archive.getattr("ver")?.extract()?,
    }
    .read(class)
}

#[pyfunction]
pub fn skp_mfc_readers(legacy: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = legacy.py();
    let partial = py.import("functools")?.getattr("partial")?;
    legacy.setattr(
        "full_parse_legacy",
        pyo3::wrap_pyfunction!(builder::read_legacy, py)?,
    )?;
    legacy.setattr("_fill_builder", pyo3::wrap_pyfunction!(builder::fill, py)?)?;
    legacy.setattr(
        "_read_page_list",
        pyo3::wrap_pyfunction!(pages::page_list, py)?,
    )?;
    legacy.setattr("_walk", pyo3::wrap_pyfunction!(archive::walk, py)?)?;
    legacy.setattr(
        "_read_entity_list",
        pyo3::wrap_pyfunction!(archive::entities, py)?,
    )?;
    let readers = legacy.getattr("_READERS")?;
    legacy.setattr(
        "_read_entity_list_inner",
        pyo3::wrap_pyfunction!(definitions::skp_mfc_entities, py)?,
    )?;
    let dispatch = pyo3::wrap_pyfunction!(skp_mfc_entity, py)?;
    for (class, method) in [
        ("CVertex", "vertex"),
        ("CEdge", "edge"),
        ("CCurve", "curve"),
        ("CArcCurve", "arccurve"),
        ("CEdgeUse", "edgeuse"),
        ("CLoop", "loop"),
        ("CFace", "face"),
        ("CGroup", "instance"),
        ("CComponentInstance", "instance"),
        ("CImage", "image"),
        ("CFaceTextureCoords", "ftc"),
        ("CCamera", "camera"),
        ("CThumbnail", "thumbnail"),
        ("CDib", "dib"),
        ("CSkFont", "skfont"),
        ("CAttributeContainer", "attr_container"),
        ("CDimensionLinear", "dimlinear"),
        ("CText", "text"),
        ("CLayer", "layer"),
        ("CComponentDefinition", "definition"),
        ("CConstructionLine", "constructionline"),
        ("CAttributeNamed", "attr_named"),
        ("CSectionPlane", "sectionplane"),
        ("CRelationship", "relationship"),
        ("CConstructionPoint", "constructionpoint"),
        ("CMaterial", "material"),
    ] {
        let handle = partial.call1((&dispatch, class))?;
        readers.set_item(class, &handle)?;
        legacy.setattr(format!("_read_{method}"), handle)?;
    }
    Ok(())
}
