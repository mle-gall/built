use super::*;
#[pyfunction(signature=(this,mesh,backing,replaced=false,created=None,removed=None,modified=None,moved=None,revision=None,ids=None,derived=None,merged=None))]
#[allow(clippy::too_many_arguments)]
fn mesh_init(
    this: &Obj<'_>,
    mesh: &Obj<'_>,
    backing: &Obj<'_>,
    replaced: bool,
    created: Option<&Obj<'_>>,
    removed: Option<&Obj<'_>>,
    modified: Option<&Obj<'_>>,
    moved: Option<&Obj<'_>>,
    revision: Option<&Obj<'_>>,
    ids: Option<&Obj<'_>>,
    derived: Option<&Obj<'_>>,
    merged: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = this.py();
    // Persistent identities (docs/persistent-ids.md §6.1).
    this.setattr("definition", crate::element_ids::definition_uid(mesh)?)?;
    this.setattr("ids", ids.cloned().unwrap_or_else(|| dict(py)))?;
    this.setattr("derived", derived.cloned().unwrap_or_else(|| list(py)))?;
    this.setattr("merged", merged.cloned().unwrap_or_else(|| list(py)))?;
    this.setattr(
        "key",
        py.import("built_core.command_protocol")?
            .call_method1("mesh_key", (mesh,))?,
    )?;
    this.setattr("_ref", py.import("weakref")?.call_method1("ref", (mesh,))?)?;
    this.setattr("backing", backing)?;
    this.setattr("replaced", replaced)?;
    for (k, v) in [
        ("created", created),
        ("removed", removed),
        ("modified", modified),
    ] {
        let d = match v {
            Some(v) if v.is_truthy()? => v.clone(),
            _ => sets(py, ["vertex", "edge", "face"])?,
        };
        this.setattr(k, d)?
    }
    this.setattr(
        "moved",
        match moved {
            Some(v) => py.get_type::<PyList>().call1((v,))?,
            None => list(py),
        },
    )?;
    this.setattr("revision", revision)
}
fn sets<'p>(py: Python<'p>, names: [&str; 3]) -> PyResult<Obj<'p>> {
    let d = dict(py);
    for k in names {
        d.set_item(k, list(py))?
    }
    Ok(d)
}
#[pyfunction]
fn step_init(
    this: &Obj<'_>,
    revision: &Obj<'_>,
    phase: &Obj<'_>,
    command: &Obj<'_>,
) -> PyResult<()> {
    let py = this.py();
    for (k, v) in [
        ("revision", revision),
        ("phase", phase),
        ("command", command),
    ] {
        this.setattr(k, v)?
    }
    for k in ["scene_fields", "meshes", "copies"] {
        this.setattr(k, list(py))?
    }
    for k in ["placements", "tags", "materials"] {
        this.setattr(k, sets(py, ["created", "removed", "modified"])?)?
    }
    this.setattr("records", dict(py))?;
    this.setattr("document", py.None())?;
    this.setattr("complete", true)
}
#[pyfunction]
fn connect(this: &Obj<'_>, history: &Obj<'_>) -> PyResult<()> {
    this.call_method1("_connect", (history,))?;
    Ok(())
}
pub(super) fn install(m: &Module<'_>, core: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    for(name,base,slots,init)in[
 ("MeshChange","FeedMeshRecord","key _ref backing replaced created removed modified moved revision definition ids derived merged",wrap_pyfunction!(mesh_init,m)?),
 ("Step","FeedStepRecord","revision phase command meshes placements tags materials complete copies scene_fields records _reader_context document",wrap_pyfunction!(step_init,m)?),
 ("ChangeFeed","HistoryFeed","",wrap_pyfunction!(connect,m)?)]{let ns=PyDict::new(py);if !slots.is_empty(){ns.set_item("__slots__",PyTuple::new(py,slots.split_whitespace().collect::<Vec<_>>())?)?}else{ns.set_item("maxlen",256)?}method(&ns,"__init__",init.into_any())?;class(m,name,vec![core.getattr(base)?],&ns)?;}
    m.add("ATTRS_COPY_MAX", 50_000)
}
