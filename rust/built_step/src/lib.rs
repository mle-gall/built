// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed STEP exchange. Source bytes and Part 21 identifiers survive diagnostics.
use std::io::Read;
pub use step_io;
use step_io::{Error, Report, StepModel};

pub struct Exchange {
    pub model: StepModel,
    pub report: Report,
    pub synthetic_file_name: bool,
}

pub fn read(source: &[u8]) -> Result<Exchange, Error> {
    let (model, report, synthetic_file_name) = match step_io::read(source) {
        Ok((model, report)) => (model, report, false),
        Err(e @ Error::MissingHeaderEntity { name: "FILE_NAME" }) => {
            let Some(offset) = missing_name_offset(source) else {
                return Err(e);
            };
            const RECORD: &[u8] = b"\nFILE_NAME('','',(''),(''),'','','');\n";
            let (model, report) =
                step_io::read(source[..offset].chain(RECORD).chain(&source[offset..]))?;
            (model, report, true)
        }
        Err(e) => return Err(e),
    };
    Ok(Exchange {
        model,
        report,
        synthetic_file_name,
    })
}

fn missing_name_offset(source: &[u8]) -> Option<usize> {
    use ruststep::parser::exchange::{header_entity, header_section};
    // Diagnose only the header. Reading arbitrary body bytes as UTF-8 would
    // prevent recovery of legacy encodings in product names.
    let prefix = &source[..source.len().min(1024 * 1024)];
    let text = match std::str::from_utf8(prefix) {
        Ok(text) => text,
        Err(e) => std::str::from_utf8(&prefix[..e.valid_up_to()]).ok()?,
    };
    let header = text
        .trim_start()
        .strip_prefix("ISO-10303-21;")?
        .trim_start();
    let (_, records) = header_section(header).ok()?;
    if records.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
        != ["FILE_DESCRIPTION", "FILE_SCHEMA"]
    {
        return None;
    }
    let (after_description, _) =
        header_entity(header.strip_prefix("HEADER;")?.trim_start()).ok()?;
    Some(text.len() - after_description.len())
}

mod representation;
pub use representation::Authored;
pub mod planar;
pub mod sheet;
#[derive(Clone, Copy, Debug)]
pub enum Measure {
    Volume(f64),
    Area(f64),
}
impl Measure {
    fn ordered(self) -> (u8, f64) {
        match self {
            Self::Volume(v) => (0, v),
            Self::Area(a) => (1, a),
        }
    }
}

use std::io::Write;
use std::path::Path;
use step_io::build::Frame;

pub struct Definition<B = planar::Body> {
    pub name: String,
    pub bodies: Vec<B>,
    /// Direct child definitions with rigid, definition-local placements in metres.
    pub children: Vec<(usize, Frame)>,
}

/// Writes planar solids, one product per definition. Validate the file on disk
/// before replacing an existing destination, including for in-place conversions.
pub fn write_document(definitions: &[Definition], path: &Path) -> Result<(), String> {
    write_checked(
        definitions,
        path,
        1e-7,
        planar::Body::add,
        |_, s, m| Ok(Measure::Volume(planar::Body::from_solid(s, m)?.volume()?)),
        |_, sh, sense, m| {
            Ok(Measure::Area(
                planar::Body::from_shell(sh, sense, m)?.area()?,
            ))
        },
    )
}

pub fn write_checked<B>(
    definitions: &[Definition<B>],
    path: &Path,
    uncertainty_mm: f64,
    add: impl Fn(&B, &mut step_io::StepBuilder, step_io::build::Part, &str) -> Result<Authored, String>,
    measure: impl Fn(&StepModel, step_io::scene::Solid<'_>, f64) -> Result<Measure, String>,
    measure_sheet: impl Fn(
        &StepModel,
        step_io::scene::Shell<'_>,
        step_io::Orientation,
        f64,
    ) -> Result<Measure, String>,
) -> Result<(), String> {
    let mut builder = step_io::StepBuilder::new_with(&step_io::build::UnitsInput {
        uncertainty: uncertainty_mm,
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    let parts: Vec<_> = definitions.iter().map(|d| builder.part(&d.name)).collect();
    let mut volumes = Vec::new();
    let mut items = Vec::new();
    let mut placements = Vec::new();
    for (definition, &part) in definitions.iter().zip(&parts) {
        let mut row = Vec::new();
        for body in &definition.bodies {
            let Authored(metric, item) = add(body, &mut builder, part, &definition.name)?;
            volumes.push(metric);
            row.push(item);
        }
        items.push(row);
        let mut occurrences = Vec::new();
        for (child, at) in &definition.children {
            let child_index = *child;
            let &child = parts.get(*child).ok_or("invalid occurrence target")?;
            let rigid = |a: [f64; 3]| glam::DVec3::from(a);
            if (rigid(at.axis).length_squared() - 1.).abs() > 1e-12
                || (rigid(at.ref_dir).length_squared() - 1.).abs() > 1e-12
                || rigid(at.axis).dot(rigid(at.ref_dir)).abs() > 1e-12
                || ![at.origin, at.axis, at.ref_dir]
                    .into_iter()
                    .flatten()
                    .all(f64::is_finite)
            {
                return Err("invalid rigid placement".into());
            }
            let at = Frame {
                origin: at.origin.map(|v| v * 1000.),
                ..*at
            };
            let link = builder.place(part, child, at).map_err(|e| e.to_string())?;
            occurrences.push((child_index, at, link));
        }
        placements.push(occurrences);
    }
    if volumes.is_empty() {
        return Err("no STEP bodies".into());
    }
    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    {
        let mut out = std::io::BufWriter::new(tmp.as_file_mut());
        let model = if volumes.iter().any(|v| matches!(v, Measure::Area(_))) {
            representation::finish(builder, &parts, items, placements, uncertainty_mm)
                .map_err(|e| e.to_string())?
        } else {
            builder.finish()
        };
        step_io::write(&model, &mut out).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    let check =
        step_io::read(tmp.reopen().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let scene = check.0.scene();
    let metres = scene.units().length.ok_or("missing length unit")?.to_si;
    let mut restored = Vec::new();
    for solid in scene.all_solids() {
        restored.push(measure(&check.0, solid, metres)?);
    }
    for sheet in scene.all_surface_models() {
        for (shell, sense) in sheet.shells() {
            restored.push(measure_sheet(&check.0, shell, sense, metres)?);
        }
    }
    let order = |a: &Measure, b: &Measure| {
        let (ak, av) = a.ordered();
        let (bk, bv) = b.ordered();
        ak.cmp(&bk).then_with(|| av.total_cmp(&bv))
    };
    volumes.sort_by(order);
    restored.sort_by(order);
    if !check.1.dropped.is_empty()
        || scene.all_product_definitions().count() != parts.len()
        || scene
            .all_product_definitions()
            .map(|d| d.occurrences().count())
            .sum::<usize>()
            != definitions.iter().map(|d| d.children.len()).sum::<usize>()
        || restored.len() != volumes.len()
        || restored.iter().zip(&volumes).any(|(a, b)| {
            let (ak, a) = a.ordered();
            let (bk, b) = b.ordered();
            ak != bk || !a.is_finite() || (a - b).abs() > 1e-21_f64.max(1e-7 * b.abs())
        })
    {
        return Err("STEP reopen verification failed".into());
    }
    tmp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Source labels are diagnostics only. The typed reader packs each simple
/// entity arena in ascending Part 21 id order after dropping unresolved rows.
pub struct SourceIds(std::collections::HashMap<String, Vec<u64>>);
impl SourceIds {
    pub fn new(bytes: &[u8], exchange: &Exchange) -> Self {
        use std::collections::{HashMap, HashSet};
        let mut ids = HashMap::<String, Vec<u64>>::new();
        let dropped: HashSet<_> = exchange.report.dropped.iter().map(|r| r.0).collect();
        let offset = exchange
            .synthetic_file_name
            .then(|| missing_name_offset(bytes))
            .flatten();
        let mut input: Box<dyn Read + '_> = if let Some(i) = offset {
            Box::new(
                (&bytes[..i])
                    .chain(b"\nFILE_NAME('','',(''),(''),'','','');\n".as_slice())
                    .chain(&bytes[i..]),
            )
        } else {
            Box::new(bytes)
        };
        if let Ok(stream) = step_io::parser::stream(&mut input) {
            for record in stream {
                if let Ok(step_io::parser::RawEntity::Simple { id, name, .. }) = record {
                    if !dropped.contains(&id) {
                        ids.entry(name).or_default().push(id);
                    }
                }
            }
        }
        for v in ids.values_mut() {
            v.sort_unstable();
        }
        Self(ids)
    }
    pub fn label(&self, model: &StepModel, key: step_io::EntityKey) -> String {
        use step_io::{model as m, EntityKey as K};
        macro_rules! slot {
            ($id:expr,$t:ty) => {
                ($id.0 as usize, model.all::<$t>().count())
            };
        }
        let (slot, count) = match key {
            K::ManifoldSolidBrep(i) => slot!(i, m::ManifoldSolidBrep),
            K::BrepWithVoids(i) => slot!(i, m::BrepWithVoids),
            K::ClosedShell(i) => slot!(i, m::ClosedShell),
            K::OpenShell(i) => slot!(i, m::OpenShell),
            K::ShellBasedSurfaceModel(i) => slot!(i, m::ShellBasedSurfaceModel),
            K::MappedItem(i) => slot!(i, m::MappedItem),
            _ => return format!("{key:?}"),
        };
        let kind = key.keyword().unwrap_or("ENTITY");
        if let Some(ids) = self.0.get(kind).filter(|v| v.len() == count) {
            if let Some(id) = ids.get(slot) {
                return format!("{kind} #{id}");
            }
        }
        kind.to_string()
    }
}
