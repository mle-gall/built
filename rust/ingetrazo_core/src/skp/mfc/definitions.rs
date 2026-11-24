// SPDX-License-Identifier: GPL-3.0-or-later
//! Definition membership, version-specific counts and class record bodies.
use super::*;
use pyo3::types::{PyDict, PyList};
const MARKER: &[u8] = &[0xff, 0xfe, 0xff];
fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(at..)?.get(..2)?.try_into().ok()?,
    ))
}
fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(at..)?.get(..4)?.try_into().ok()?,
    ))
}
pub(super) fn class_at(bytes: &[u8], at: usize, slot: u32) -> bool {
    if slot < 0x7fff {
        u16_at(bytes, at) == Some(0x8000 | slot as u16)
    } else {
        u16_at(bytes, at) == Some(0x7fff) && u32_at(bytes, at + 2) == Some(0x80000000 | slot)
    }
}

impl<'a, 'py> Entity<'a, 'py> {
    fn valid_tag(
        &self,
        data: &[u8],
        at: usize,
        allow_null: bool,
        allow_backref: bool,
    ) -> PyResult<bool> {
        Ok(match u16_at(data, at) {
            None => false,
            Some(0) => allow_null,
            Some(0x7fff | 0xffff) => true,
            Some(t) if t & 0x8000 != 0 => match self
                .ar
                .getattr("slots")?
                .call_method1("get", (t & 0x7fff,))?
            {
                v if v.is_none() => false,
                v => v.get_item(0)?.eq("class")?,
            },
            Some(t) => {
                allow_backref && u32::from(t) < self.ar.getattr("next_slot")?.extract::<u32>()?
            }
        })
    }
    pub(super) fn recovered_count(&self, at: usize, max: u32) -> PyResult<Option<u32>> {
        let bytes = self.r.borrow().data.clone_ref(self.py);
        let data = bytes.bind(self.py).as_bytes();
        let Some(pos) = data
            .get(at..)
            .and_then(|tail| tail.windows(3).take(12).position(|w| w == MARKER))
            .map(|p| p + at)
        else {
            return Ok(None);
        };
        if data.get(pos + 3) != Some(&0) {
            return Ok(None);
        }
        let pad = pos + 4;
        let first = pad + data[pad..].iter().take_while(|&&b| b == 0).count();
        for candidate in (first.saturating_sub(3).max(pad)..=first).rev() {
            if let Some(n) = u32_at(data, candidate).filter(|&n| n > 0 && n <= max) {
                if self.valid_tag(data, candidate + 4, true, true)? {
                    self.r.borrow_mut().pos = candidate + 4;
                    return Ok(Some(n));
                }
            }
        }
        Ok(None)
    }
    pub(super) fn typed_attribute(&self, depth: u8) -> PyResult<Bound<'py, PyAny>> {
        if depth > 96 {
            return Err(self.error("attribute arrays nested too deeply"));
        }
        Ok(match self.u8()? {
            0 => self.py.None().into_bound(self.py),
            4 => (self.u32()? as i32).into_pyobject(self.py)?.into_any(),
            6 | 12 => self
                .r
                .borrow_mut()
                .f64(self.py)?
                .into_pyobject(self.py)?
                .into_any(),
            7 => self.u8()?.into_pyobject(self.py)?.into_any(),
            9 => self.u32()?.into_pyobject(self.py)?.into_any(),
            10 => self.string()?.into_pyobject(self.py)?.into_any(),
            11 => {
                let count = self.u32()?;
                if count > 100_000 {
                    return Err(self.error("invalid attribute array size"));
                }
                let list = PyList::empty(self.py);
                for _ in 0..count {
                    list.append(self.typed_attribute(depth + 1)?)?;
                }
                list.into_any()
            }
            17 | 18 => self.doubles(3)?.into_any(),
            _ => return Err(self.error("unknown attribute type")),
        })
    }
    pub(super) fn construction_line(&self, out: &Bound<'py, PyDict>) -> PyResult<()> {
        out.set_item("db", self.draw()?)?;
        let p = self.doubles(3)?;
        let d = self.doubles(3)?;
        let [a, b] = self.doubles(2)?.extract::<[f64; 2]>()?;
        out.set_item("point", &p)?;
        out.set_item("direction", &d)?;
        let p = p.extract::<[f64; 3]>()?;
        let d = d.extract::<[f64; 3]>()?;
        for (key, t) in [("start", a), ("end", b)] {
            out.set_item(
                key,
                (t.abs() < 1e20).then(|| (p[0] + t * d[0], p[1] + t * d[1], p[2] + t * d[2])),
            )?;
        }
        let cached = self.ar.getattr_opt("_cline_tail")?.filter(|v| !v.is_none());
        let tail = if let Some(val) = cached {
            val.extract()?
        } else {
            let r = self.r.borrow();
            let data = r.data.bind(self.py).as_bytes();
            let default = if self.ver == 17 { 7 } else { 4 };
            let mut chosen = 4;
            'candidates: for (allow_null, order) in [
                (false, [default, 0, if default == 7 { 4 } else { 7 }]),
                (true, [4, 0, 7]),
            ] {
                for off in order {
                    if self.valid_tag(data, r.pos + off, allow_null, false)? {
                        chosen = off;
                        break 'candidates;
                    }
                }
            }
            self.ar.setattr("_cline_tail", chosen)?;
            chosen
        };
        self.skip(tail)
    }
    pub(super) fn definition(&self, out: &Bound<'py, PyDict>) -> PyResult<()> {
        self.skip(match self.ver {
            0..=3 => 13,
            4 => 19,
            5..=16 => 20,
            _ => 22,
        })?;
        let layers = self.u32()?;
        if layers > 10_000 {
            return Err(self.error("invalid layer count"));
        }
        for _ in 0..layers {
            while self.peek()? == 0 {
                self.skip(2)?;
            }
            self.object(Some("CLayer"))?;
        }
        if self.ver == 19 && layers > 0 && self.peek()? == 0 {
            self.skip(2)?;
        }
        if self.u16()? == 0x7fff {
            self.skip(4)?;
        }
        let at = self.r.borrow().pos;
        let recovered = if self.ver >= 20 {
            self.recovered_count(at, 5_000_000)?
        } else {
            None
        };
        let mut count = match recovered {
            Some(n) => n,
            None => {
                self.skip(4)?;
                self.u32()?
            }
        };
        if count > 5_000_000 || count == 0 {
            let at = self.r.borrow().pos - 4;
            count = self.recovered_count(at, 5_000_000)?.unwrap_or(count);
        }
        if count > 5_000_000 {
            return Err(self.error("invalid definition entity count"));
        }
        let legacy = self.py.import("openskp.legacy")?;
        out.set_item(
            "ents",
            legacy.call_method1("_read_entity_list", (self.ar, self.r, count, "def"))?,
        )?;
        let mut nrel = self.u32()?;
        if nrel > 100_000 {
            let at = self.r.borrow().pos - 4;
            nrel = self
                .recovered_count(at, 100_000)?
                .ok_or_else(|| self.error("invalid relationship count"))?;
        }
        for _ in 0..nrel {
            self.object(Some("CRelationship"))?;
        }
        self.skip(2)?;
        let bytes = self.r.borrow().data.clone_ref(self.py);
        let data = bytes.bind(self.py).as_bytes();
        let pos = self.r.borrow().pos;
        if let Some(anchor) = [0, -1, 1, -2, 2, -3, 3, -4, 4]
            .into_iter()
            .filter_map(|off| pos.checked_add_signed(off))
            .find(|at| data.get(at + 16..at + 19) == Some(MARKER))
        {
            self.r.borrow_mut().pos = anchor;
        }
        out.set_item("guid", self.guid()?)?;
        out.set_item("name", self.string()?)?;
        self.string()?;
        self.string()?;
        self.skip(4)?;
        let pos = self.r.borrow().pos;
        let slot = self
            .ar
            .getattr("class_slot")?
            .call_method1("get", ("CThumbnail",))?
            .extract::<Option<u32>>()?;
        let thumb = (pos..pos + 96)
            .find(|&i| {
                (u16_at(data, i) == Some(0xffff)
                    && u16_at(data, i + 4) == Some(10)
                    && data.get(i + 6..i + 16) == Some(b"CThumbnail"))
                    || slot.is_some_and(|n| class_at(data, i, n))
            })
            .ok_or_else(|| self.error("missing definition thumbnail"))?;
        let gap = &data[pos..thumb];
        let flags = gap.len().checked_sub(9).map_or(0, |k| gap[k]);
        out.set_item("faces_camera", flags & 1 != 0)?;
        out.set_item("shadows_face_sun", flags & 2 != 0)?;
        out.set_item("is_group", gap.ends_with(&1u32.to_le_bytes()))?;
        self.r.borrow_mut().pos = thumb;
        self.object(Some("CThumbnail"))?;
        Ok(())
    }
}

#[pyfunction]
pub(super) fn skp_mfc_entities<'py>(
    ar: &Bound<'py, PyAny>,
    r: &Bound<'py, SkpArchiveCursor>,
    count: usize,
    owner: &str,
    ents: &Bound<'py, PyList>,
) -> PyResult<Bound<'py, PyList>> {
    let py = ar.py();
    let legacy = py.import("openskp.legacy")?;
    let error = legacy.getattr("LegacyParseError")?;
    let inline = ar
        .getattr("inline_stack")?
        .get_item(-1)?
        .cast_into::<PyDict>()?;
    while ents.len() < count {
        let p = r.borrow().pos;
        if !inline.is_empty()
            && inline.len() + ents.len() >= count
            && !inline.contains(legacy.call_method1("_peek_backref", (ar, r))?)?
        {
            break;
        }
        let prefix = r.borrow().peek(py, 25);
        if owner == "def"
            && prefix.as_bytes().starts_with(&[0; 4])
            && prefix.as_bytes().get(22..25) == Some(MARKER)
        {
            break;
        }
        let prev = ar.getattr("in_entity_list")?;
        ar.setattr("in_entity_list", true)?;
        let rec = ar.call_method1("read_object", (r,));
        ar.setattr("in_entity_list", prev)?;
        match rec {
            Ok(rec) => {
                inline.call_method1("pop", (rec.get_item(0)?, py.None()))?;
                ents.append(rec)?;
            }
            Err(e)
                if e.is_instance(py, &error)
                    && (owner == "root"
                        || (owner == "def"
                            && ar.getattr("burn_stack")?.get_item(-1)?.is_truthy()?)) =>
            {
                r.borrow_mut().pos = p;
                break;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(ents.clone())
}
