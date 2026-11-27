// SPDX-License-Identifier: GPL-3.0-or-later
//! Definition and item contexts may differ, even within one assembly.
use built_step::step_io::{model as m, StepModel};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Units {
    pub metres: f64,
    pub angle: Option<f64>,
    pub tolerance: f64,
}
type Result<T> = std::result::Result<T, &'static str>;
type Key = m::EntityKey;

fn si(name: m::SiUnitName, prefix: Option<m::SiPrefix>) -> Option<(bool, f64)> {
    use m::SiPrefix::*;
    let exp = match prefix {
        None => 0,
        Some(Atto) => -18,
        Some(Femto) => -15,
        Some(Pico) => -12,
        Some(Nano) => -9,
        Some(Micro) => -6,
        Some(Milli) => -3,
        Some(Centi) => -2,
        Some(Deci) => -1,
        Some(Deca) => 1,
        Some(Hecto) => 2,
        Some(Kilo) => 3,
        Some(Mega) => 6,
        Some(Giga) => 9,
        Some(Tera) => 12,
        Some(Peta) => 15,
        Some(Exa) => 18,
    };
    match name {
        m::SiUnitName::Metre => Some((true, 10f64.powi(exp))),
        m::SiUnitName::Radian => Some((false, 10f64.powi(exp))),
        _ => None,
    }
}
fn scale(
    model: &StepModel,
    u: &m::UnitRef,
    path: &mut HashSet<Key>,
) -> Result<Option<(bool, f64)>> {
    if !path.insert(u.entity_key()) || path.len() > 64 {
        return Err("cyclic STEP unit conversion");
    }
    let factor = match u {
        m::UnitRef::SiUnit(i) => return Ok(si(model[*i].name, model[*i].prefix)),
        m::UnitRef::ConversionBasedUnit(i) => &model[*i].conversion_factor,
        m::UnitRef::Complex(i) => {
            let mut factor = None;
            for part in &model[*i].parts {
                match part {
                    m::PartialEntity::SiUnit { prefix, name } => return Ok(si(*name, *prefix)),
                    m::PartialEntity::ConversionBasedUnit {
                        conversion_factor, ..
                    } => factor = Some(conversion_factor),
                    _ => (),
                }
            }
            let Some(f) = factor else { return Ok(None) };
            f
        }
        _ => return Ok(None),
    };
    macro_rules! fields {
        ($m:expr) => {
            (&$m.value_component, &$m.unit_component)
        };
    }
    let (value, base) = match factor {
        m::MeasureWithUnitRef::MeasureWithUnit(i) => fields!(model[*i]),
        m::MeasureWithUnitRef::LengthMeasureWithUnit(i) => fields!(model[*i]),
        m::MeasureWithUnitRef::PlaneAngleMeasureWithUnit(i) => fields!(model[*i]),
        m::MeasureWithUnitRef::PositiveLengthMeasureWithUnit(i) => fields!(model[*i]),
        m::MeasureWithUnitRef::PositivePlaneAngleMeasureWithUnit(i) => fields!(model[*i]),
        m::MeasureWithUnitRef::Complex(i) => model[*i]
            .parts
            .iter()
            .find_map(|p| match p {
                m::PartialEntity::MeasureWithUnit {
                    value_component,
                    unit_component,
                } => Some((value_component, unit_component)),
                _ => None,
            })
            .ok_or("invalid complex unit measure")?,
        _ => return Err("unsupported conversion measure"),
    };
    Ok(scale(model, base, path)?.map(|(k, s)| (k, s * value.value.as_f64())))
}
fn context(model: &StepModel, c: &m::RepresentationContextRef) -> Result<Units> {
    let mut uncertainty = None;
    let units = match c {
        m::RepresentationContextRef::Complex(i) => {
            let mut units = None;
            for p in &model[*i].parts {
                match p {
                    m::PartialEntity::GlobalUnitAssignedContext { units: u } => units = Some(u),
                    m::PartialEntity::GlobalUncertaintyAssignedContext { uncertainty: u } => {
                        uncertainty = u.first()
                    }
                    _ => (),
                }
            }
            units.ok_or("missing representation units")?
        }
        m::RepresentationContextRef::GlobalUnitAssignedContext(i) => &model[*i].units,
        _ => return Err("missing representation units"),
    };
    let mut length = None;
    let mut angle = None;
    for u in units {
        if let Some((is_length, s)) = scale(model, u, &mut HashSet::new())? {
            if !s.is_finite() || s <= 0. {
                return Err("invalid unit factor");
            }
            let slot = if is_length { &mut length } else { &mut angle };
            if slot.replace(s).is_some_and(|v| v != s) {
                return Err("conflicting units in STEP context");
            }
        }
    }
    let metres = length.ok_or("missing representation length unit")?;
    let tolerance = if let Some(m::UncertaintyMeasureWithUnitRef::UncertaintyMeasureWithUnit(i)) =
        uncertainty
    {
        let q = &model[*i];
        let (true, s) = scale(model, &q.unit_component, &mut HashSet::new())?
            .ok_or("unresolved uncertainty unit")?
        else {
            return Err("expected a length uncertainty");
        };
        q.value_component.value.as_f64() * s
    } else {
        1e-6 * metres
    };
    if tolerance <= 0. || !tolerance.is_finite() {
        return Err("invalid STEP uncertainty");
    }
    Ok(Units {
        metres,
        angle,
        tolerance,
    })
}

pub(super) struct Contexts {
    items: HashMap<Key, Units>,
    definitions: HashMap<Key, Units>,
    fallback: Option<Units>,
}
impl Contexts {
    pub fn new(model: &StepModel) -> Result<Self> {
        let mut cache = HashMap::new();
        let mut items = HashMap::new();
        let mut reps = HashMap::new();
        macro_rules! collect {
            ($t:ty,$kind:ident) => {
                for (id, r) in model.all::<$t>() {
                    let ctx = r.context_of_items.entity_key();
                    let units = if let Some(&u) = cache.get(&ctx) {
                        u
                    } else {
                        let u = context(model, &r.context_of_items)?;
                        cache.insert(ctx, u);
                        u
                    };
                    reps.insert(Key::$kind(id), units);
                    for item in &r.items {
                        if items
                            .insert(item.entity_key(), units)
                            .is_some_and(|prev: Units| {
                                prev.metres != units.metres || prev.angle != units.angle
                            })
                        {
                            return Err("same STEP item has incompatible unit contexts");
                        }
                    }
                }
            };
        }
        collect!(m::ShapeRepresentation, ShapeRepresentation);
        collect!(
            m::AdvancedBrepShapeRepresentation,
            AdvancedBrepShapeRepresentation
        );
        collect!(
            m::ManifoldSurfaceShapeRepresentation,
            ManifoldSurfaceShapeRepresentation
        );
        collect!(
            m::TessellatedShapeRepresentation,
            TessellatedShapeRepresentation
        );
        let mut definitions = HashMap::new();
        for (_, s) in model.all::<m::ShapeDefinitionRepresentation>() {
            if let m::RepresentedDefinitionRef::ProductDefinitionShape(i) = s.definition {
                if let Some(&u) = reps.get(&s.used_representation.entity_key()) {
                    let k = model[i].definition.entity_key();
                    definitions.entry(k).or_insert(u);
                }
            }
        }
        let first = cache.values().copied().next();
        let fallback = first.filter(|f| cache.values().all(|u| u == f));
        Ok(Self {
            items,
            definitions,
            fallback,
        })
    }
    pub fn item(&self, k: Key) -> Result<Units> {
        self.items
            .get(&k)
            .copied()
            .or(self.fallback)
            .ok_or("unresolved item representation context")
    }
    pub fn definition(&self, k: Key) -> Result<Units> {
        self.definitions
            .get(&k)
            .copied()
            .or(self.fallback)
            .ok_or("unresolved placement context")
    }
}
