// SPDX-License-Identifier: GPL-3.0-or-later
//! Locate the material/layer managers, discover the archive's absolute slot
//! base, and walk the definition and root lists in file order.
use super::*;
use pyo3::types::{PyDict, PyList};

const BASE: u32 = 1 << 20;
const LIMIT: u32 = 100_000;

fn header(data: &[u8]) -> Result<(u16, usize, u32), String> {
    let prefix = data
        .iter()
        .take(96)
        .copied()
        .filter(|&b| b != 0)
        .collect::<Vec<_>>();
    let version = prefix
        .split(|&b| b == b'{')
        .skip(1)
        .filter_map(|s| {
            let (digits, _) = s.split_at(s.iter().position(|&b| b == b'.')?);
            std::str::from_utf8(digits).ok()?.parse::<u16>().ok()
        })
        .next()
        .ok_or("missing .skp version")?;
    let find = |class: &[u8]| {
        data.windows(6 + class.len()).position(|w| {
            w.starts_with(&[0xff, 0xff])
                && w[4..6] == (class.len() as u16).to_le_bytes()
                && &w[6..] == class
        })
    };
    if let Some(start) = find(b"CMaterial") {
        let at = start.checked_sub(4).ok_or("truncated material manager")?;
        let count = u32::from_le_bytes(data[at..start].try_into().unwrap());
        if count > LIMIT {
            return Err("invalid material count".into());
        }
        return Ok((version, start, count));
    }
    let at = find(b"CLayer")
        .and_then(|at| at.checked_sub(if version >= 17 { 9 } else { 8 }))
        .ok_or("missing layer manager")?;
    Ok((version, at, 0))
}

fn archive<'py>(
    legacy: &Bound<'py, PyAny>,
    bytes: &Bound<'py, PyBytes>,
    version: u16,
    base: u32,
    start: usize,
) -> PyResult<Bound<'py, PyAny>> {
    let ar = legacy.call_method1("_Archive", (bytes, version))?;
    ar.getattr("readers")?
        .call_method1("update", (legacy.getattr("_READERS")?,))?;
    ar.setattr("next_slot", base)?;
    ar.setattr("walk_base", base)?;
    ar.getattr("r")?.setattr("pos", start)?;
    Ok(ar)
}

impl<'a, 'py> Entity<'a, 'py> {
    fn layer_count(&self) -> PyResult<u32> {
        self.skip(4)?;
        if self.ver >= 17 {
            self.skip(1)?;
        }
        if self.ver < 4 {
            return Ok(0);
        }
        let n = self.u32()?;
        if n > LIMIT {
            return Err(self.error("invalid layer count"));
        }
        Ok(n)
    }
    fn bases(&self, count: u32) -> PyResult<Vec<u32>> {
        if count >= 2 {
            self.object(Some("CMaterial"))?;
            let tag = self.peek()?;
            if tag == 0xffff || tag & 0x8000 == 0 {
                return Err(self.error("cannot determine the material slot base"));
            }
            return Ok(vec![u32::from(tag & 0x7fff)]);
        }
        for _ in 0..count {
            self.object(Some("CMaterial"))?;
        }
        let n = self.layer_count()?;
        if self.ver >= 4 && n == 0 {
            return Err(self.error("missing active layer"));
        }
        let slots = PyList::empty(self.py);
        for _ in 0..n {
            slots.append(self.object(Some("CLayer"))?.get_item(0)?)?;
        }
        let args = PyDict::new(self.py);
        args.set_item("slots_out", &slots)?;
        args.set_item("unmatched_class_ref", true)?;
        self.ar
            .call_method("_collect_trailing_layers", (self.r,), Some(&args))?;
        let anchor = self.object(None)?;
        if !anchor.get_item(1)?.eq("premodel")? {
            return Err(self.error("expected an active layer reference"));
        }
        let abs = anchor.get_item(0)?.extract::<i64>()?;
        let mut bases = Vec::new();
        for s in slots {
            if let Some(rel) = s.extract::<Option<i64>>()? {
                let base = abs - (rel - i64::from(BASE));
                if (1..i64::from(BASE)).contains(&base) {
                    bases.push(base as u32);
                }
            }
        }
        Ok(bases)
    }
    fn next_definition(&self) -> PyResult<bool> {
        let bytes = self.r.borrow().peek(self.py, 26);
        let b = bytes.as_bytes();
        if b.starts_with(&[0xff, 0xff]) && b.get(6..26) == Some(b"CComponentDefinition") {
            return Ok(true);
        }
        let slot = self
            .ar
            .getattr("class_slot")?
            .call_method1("get", ("CComponentDefinition",))?
            .extract::<Option<u32>>()?;
        Ok(slot.is_some_and(|n| super::definitions::class_at(b, 0, n)))
    }
    fn model(&self, count: u32) -> PyResult<Bound<'py, PyAny>> {
        let materials = PyList::empty(self.py);
        for _ in 0..count {
            let rec = self.object(Some("CMaterial"))?;
            materials.append((rec.get_item(0)?, rec.get_item(2)?))?;
        }
        let n = self.layer_count()?;
        let layers = PyList::empty(self.py);
        while self.ver < 4 || layers.len() < (n as usize) {
            let tag = self.peek()?;
            if tag == 0 {
                self.skip(2)?;
                continue;
            }
            if tag & 0x8000 == 0 {
                break;
            }
            let rec = self.object(Some("CLayer"))?;
            let item = rec.get_item(2)?;
            if !item.is_none() {
                layers.append((rec.get_item(0)?, item))?;
            }
        }
        let args = PyDict::new(self.py);
        args.set_item("layers_out", &layers)?;
        self.ar
            .call_method("_collect_trailing_layers", (self.r,), Some(&args))?;
        if !self.object(None)?.get_item(1)?.eq("CLayer")? {
            return Err(self.error("invalid definition list anchor"));
        }
        let at = self.r.borrow().pos;
        let mut defs = self.u32()?;
        if defs > 1_000_000 {
            if let Some(n) = self.recovered_count(at, 1_000_000)? {
                defs = n;
                if !self.next_definition()? {
                    defs = 0;
                    self.r.borrow_mut().pos -= 4;
                }
            } else {
                let bytes = self.r.borrow().data.clone_ref(self.py);
                let tail = &bytes.bind(self.py).as_bytes()[at..];
                if let Some(i) = tail
                    .windows(3)
                    .take(12)
                    .position(|w| w == [0xff, 0xfe, 0xff])
                {
                    if tail.get(i + 3..i + 21) == Some(&[0; 18]) {
                        defs = 0;
                        self.r.borrow_mut().pos = at + i + 17;
                    }
                }
            }
        }
        if defs > 1_000_000 {
            return Err(self.error("invalid definition count"));
        }
        for _ in 0..defs {
            self.object(Some("CComponentDefinition"))?;
        }
        while self.next_definition()? {
            self.object(Some("CComponentDefinition"))?;
        }
        let at = self.r.borrow().pos;
        let mut count = self.u32()?;
        if count > 5_000_000 {
            count = self
                .recovered_count(at, 5_000_000)?
                .ok_or_else(|| self.error("invalid root entity count"))?;
        }
        let root = entities(self.ar, self.r, count as usize, "root")?;
        Ok((self.ar, root, layers, materials)
            .into_pyobject(self.py)?
            .into_any())
    }
}

#[pyfunction]
pub(super) fn entities<'py>(
    ar: &Bound<'py, PyAny>,
    r: &Bound<'py, SkpArchiveCursor>,
    count: usize,
    owner: &str,
) -> PyResult<Bound<'py, PyList>> {
    let py = ar.py();
    let inline = PyDict::new(py);
    let inlines = ar.getattr("inline_stack")?;
    let burns = ar.getattr("burn_stack")?;
    burns.call_method1("append", (0,))?;
    inlines.call_method1("append", (&inline,))?;
    let result = (|| {
        let list = super::definitions::skp_mfc_entities(ar, r, count, owner, &PyList::empty(py))?;
        for (slot, _) in inline {
            if list.len() == count {
                break;
            }
            let object = ar.getattr("slots")?.get_item(&slot)?;
            list.append((slot, object.get_item(1)?, object.get_item(2)?))?;
        }
        Ok(list)
    })();
    inlines.call_method0("pop")?;
    burns.call_method0("pop")?;
    result
}

#[pyfunction]
pub(super) fn walk<'py>(data: &Bound<'py, PyBytes>) -> PyResult<Bound<'py, PyAny>> {
    let py = data.py();
    let bytes = data.as_bytes();
    let (ver, start, count) = py
        .detach(|| header(bytes))
        .map_err(|why| archive_error(py, why))?;
    let legacy = py.import("openskp.legacy")?;
    let probe = archive(&legacy, data, ver, BASE, start)?;
    let cursor = probe.getattr("r")?.cast_into::<SkpArchiveCursor>()?;
    let bases = Entity {
        ar: &probe,
        r: &cursor,
        py,
        ver,
    }
    .bases(count)?;
    let mut best = None;
    for base in bases {
        let ar = archive(&legacy, data, ver, base, start)?;
        let r = ar.getattr("r")?.cast_into::<SkpArchiveCursor>()?;
        match (Entity {
            ar: &ar,
            r: &r,
            py,
            ver,
        })
        .model(count)
        {
            Ok(out) => return Ok(out),
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyInterruptedError>(py) => {
                return Err(e)
            }
            Err(e) => {
                let pos = r.borrow().pos;
                if best.as_ref().is_none_or(|(p, _)| *p < pos) {
                    best = Some((pos, e));
                }
            }
        }
    }
    Err(best
        .map(|(_, e)| e)
        .unwrap_or_else(|| archive_error(py, "no viable slot base".into())))
}
