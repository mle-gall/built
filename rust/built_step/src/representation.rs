// SPDX-License-Identifier: GPL-3.0-or-later
//! A product can carry several open shells and solids in the same representation.
//! The high-level builder collects only solids, so wire mixed documents using
//! its typed author and the product and occurrence IDs it already allocated.
use step_io::{
    build::{Frame, Part},
    model as m, Ap242Author, AuthorError, StepBuilder, StepModel,
};

pub struct Authored(pub crate::Measure, pub m::RepresentationItemRef);
type Occurrence = (usize, Frame, m::NextAssemblyUsageOccurrenceId);

fn frame(a: &mut Ap242Author, f: Frame) -> Result<m::Axis2Placement3dId, AuthorError> {
    let p = a.add_cartesian_point("", f.origin.into())?;
    let z = a.add_direction("", f.axis.into())?;
    let x = a.add_direction("", f.ref_dir.into())?;
    a.add_axis2_placement3d(
        "",
        m::CartesianPointRef::CartesianPoint(p),
        Some(m::DirectionRef::Direction(z)),
        Some(m::DirectionRef::Direction(x)),
    )
}

pub(crate) fn finish(
    mut b: StepBuilder,
    parts: &[Part],
    items: Vec<Vec<m::RepresentationItemRef>>,
    placements: Vec<Vec<Occurrence>>,
    uncertainty: f64,
) -> Result<StepModel, AuthorError> {
    use m::RepresentationItemRef as Item;
    let header = b.header().clone();
    let ids: Vec<_> = parts.iter().map(|&p| b.part_ids(p)).collect();
    let a = b.author();
    let origin = frame(
        a,
        Frame {
            origin: [0.; 3],
            axis: [0., 0., 1.],
            ref_dir: [1., 0., 0.],
        },
    )?;
    let mut links = Vec::new();
    let mut reps = Vec::new();
    for (parent, mut row) in items.into_iter().enumerate() {
        let sheets = row
            .iter()
            .all(|i| matches!(i, Item::ShellBasedSurfaceModel(_)))
            && !row.is_empty();
        row.push(Item::Axis2Placement3d(origin));
        for &(child, at, link) in &placements[parent] {
            let to = frame(a, at)?;
            row.push(Item::Axis2Placement3d(to));
            links.push((parent, child, link, to));
        }
        let ctx = m::RepresentationContextRef::Complex(crate::sheet::context(a, uncertainty)?);
        let (shape, rep) = if sheets {
            use m::Ap242ManifoldSurfaceShapeRepresentationSelectRef as S;
            let items = row
                .into_iter()
                .map(|i| match i {
                    Item::ShellBasedSurfaceModel(x) => S::ShellBasedSurfaceModel(x),
                    Item::Axis2Placement3d(x) => S::Axis2Placement3d(x),
                    _ => unreachable!(),
                })
                .collect();
            let id = a.add_manifold_surface_shape_representation("", items, ctx)?;
            (
                m::ShapeRepresentationRef::ManifoldSurfaceShapeRepresentation(id),
                m::RepresentationOrRepresentationReferenceRef::ManifoldSurfaceShapeRepresentation(
                    id,
                ),
            )
        } else {
            let id = a.add_shape_representation("", row, ctx)?;
            (
                m::ShapeRepresentationRef::ShapeRepresentation(id),
                m::RepresentationOrRepresentationReferenceRef::ShapeRepresentation(id),
            )
        };
        a.add_shape_definition_representation(
            m::PropertyDefinitionRef::ProductDefinitionShape(ids[parent].shape),
            shape,
        )?;
        reps.push(rep);
    }
    let empty = a.text("");
    for (parent, child, link, to) in links {
        let pds = a.add_product_definition_shape(
            "Placement",
            None,
            m::Ap242ProductDefinitionShapeDefinitionRef::NextAssemblyUsageOccurrence(link),
        )?;
        let transform = a.add_item_defined_transformation(
            "",
            None,
            Item::Axis2Placement3d(origin),
            Item::Axis2Placement3d(to),
        )?;
        let rel = a.add_complex(vec![
            m::PartialEntity::RepresentationRelationship {
                name: empty,
                description: None,
                rep_1: reps[child].clone(),
                rep_2: reps[parent].clone(),
            },
            m::PartialEntity::RepresentationRelationshipWithTransformation {
                transformation_operator: m::TransformationRef::ItemDefinedTransformation(transform),
            },
            m::PartialEntity::ShapeRepresentationRelationship,
        ])?;
        a.add_context_dependent_shape_representation(
            m::ShapeRepresentationRelationshipRef::Complex(rel),
            m::ProductDefinitionShapeRef::ProductDefinitionShape(pds),
        )?;
    }
    a.add_product_related_product_category(
        "part",
        None,
        ids.iter()
            .map(|p| m::ProductRef::Product(p.product))
            .collect(),
    )?;
    Ok(std::mem::take(a).finish_with_header(&header))
}
