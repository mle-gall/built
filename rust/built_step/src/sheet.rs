// SPDX-License-Identifier: GPL-3.0-or-later
use step_io::{build::Part, model as m, AuthorError, StepBuilder};

/// The high-level builder collects solids. Use the AP242 author for a sheet
/// and bind its representation to the same product-definition shape.
pub fn add(
    b: &mut StepBuilder,
    part: Part,
    name: &str,
    faces: Vec<m::AdvancedFaceId>,
    uncertainty_mm: f64,
) -> Result<(), AuthorError> {
    let product = b.part_ids(part).shape;
    let a = b.author();
    let context = context(a, uncertainty_mm)?;
    let sheet = item(a, name, faces)?;
    let rep = a.add_manifold_surface_shape_representation(
        name,
        vec![m::Ap242ManifoldSurfaceShapeRepresentationSelectRef::ShellBasedSurfaceModel(sheet)],
        m::RepresentationContextRef::Complex(context),
    )?;
    a.add_shape_definition_representation(
        m::PropertyDefinitionRef::ProductDefinitionShape(product),
        m::ShapeRepresentationRef::ManifoldSurfaceShapeRepresentation(rep),
    )?;
    Ok(())
}

pub(crate) fn context(
    a: &mut step_io::Ap242Author,
    uncertainty_mm: f64,
) -> Result<m::ComplexEntityId, AuthorError> {
    use m::PartialEntity as P;
    let mut units = Vec::new();
    for (kind, prefix, unit) in [
        (
            P::LengthUnit,
            Some(m::SiPrefix::Milli),
            m::SiUnitName::Metre,
        ),
        (P::PlaneAngleUnit, None, m::SiUnitName::Radian),
        (P::SolidAngleUnit, None, m::SiUnitName::Steradian),
    ] {
        let id = a.add_complex(vec![
            kind,
            P::NamedUnit { dimensions: None },
            P::SiUnit { prefix, name: unit },
        ])?;
        units.push(m::UnitRef::Complex(id));
    }
    let text = a.text("");
    let kind = a.text("3D");
    let len = a.text("LENGTH_MEASURE");
    let precision = a.add_uncertainty_measure_with_unit(
        m::MeasureValue {
            type_name: Some(len),
            value: m::MeasureScalar::Real(uncertainty_mm),
        },
        units[0].clone(),
        "distance_accuracy_value",
        None,
    )?;
    a.add_complex(vec![
        P::GeometricRepresentationContext {
            coordinate_space_dimension: 3,
        },
        P::GlobalUncertaintyAssignedContext {
            uncertainty: vec![
                m::UncertaintyMeasureWithUnitRef::UncertaintyMeasureWithUnit(precision),
            ],
        },
        P::GlobalUnitAssignedContext { units },
        P::RepresentationContext {
            context_identifier: text,
            context_type: kind,
        },
    ])
}

pub fn item(
    a: &mut step_io::Ap242Author,
    name: &str,
    faces: Vec<m::AdvancedFaceId>,
) -> Result<m::ShellBasedSurfaceModelId, AuthorError> {
    let shell = a.add_open_shell(
        name,
        faces.into_iter().map(m::FaceRef::AdvancedFace).collect(),
    )?;
    a.add_shell_based_surface_model(name, vec![m::ShellRef::OpenShell(shell)])
}
