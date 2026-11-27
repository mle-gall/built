//! Device-independent validation and compaction for a resident model.
use super::{edges, faces, Command};
use numpy::PyReadonlyArray1;
use pyo3::{exceptions::PyValueError, prelude::*};

type Bytes<'py> = PyReadonlyArray1<'py, u8>;
pub(super) type PrototypeArrays<'py> = (Bytes<'py>, Bytes<'py>, Bytes<'py>, Bytes<'py>);

pub(super) struct Geometry {
    pub edges: Vec<u8>,
    pub vertices: Vec<u8>,
    pub indices: Vec<u8>,
    pub commands: Vec<u8>,
    pub shifts: Vec<i64>,
    pub bounds: Vec<u8>,
    pub codes: Vec<u32>,
    pub memos: Vec<(usize, crate::edge_memo::Memo)>,
}

#[pyclass]
pub(super) struct SceneUpload(pub Option<Geometry>);

#[pymethods]
impl SceneUpload {
    #[new]
    pub(super) fn new(
        py: Python<'_>,
        prototypes: PrototypeArrays<'_>,
        instances: Bytes<'_>,
    ) -> PyResult<Self> {
        let (v, ix, p, cmd) = (
            prototypes.0.as_slice()?,
            prototypes.1.as_slice()?,
            prototypes.2.as_slice()?,
            prototypes.3.as_slice()?,
        );
        let instances = instances.as_slice()?;
        py.detach(|| Geometry::prepare(v, ix, p, cmd, instances))
            .map(|geometry| Self(Some(geometry)))
    }
}

impl Geometry {
    pub(super) fn prepare(
        v: &[u8],
        ix: &[u8],
        p: &[u8],
        cmd: &[u8],
        instances: &[u8],
    ) -> PyResult<Self> {
        if v.len() % 28 != 0
            || ix.len() % 4 != 0
            || p.is_empty()
            || p.len() % 32 != 0
            || cmd.len() != p.len() / 32 * 40
            || instances.is_empty()
            || instances.len() % 80 != 0
        {
            return Err(PyValueError::new_err("invalid scene array layout"));
        }
        let count = p.len() / 32;
        let mut codes = Vec::with_capacity(instances.len() / 80);
        let mut counts = vec![0u32; count];
        for row in instances.chunks_exact(80) {
            let id = u32::from_le_bytes(row[64..68].try_into().unwrap()) as usize;
            if id >= count {
                return Err(PyValueError::new_err("prototype id out of range"));
            }
            counts[id] += 1;
            codes.push(id as u32);
        }
        let mut start = 0;
        for (j, (row, capacity)) in cmd.chunks_exact(20).zip(counts.iter().cycle()).enumerate() {
            if j == count {
                start = 0;
            }
            let c: Command = bytemuck::pod_read_unaligned(row);
            if c.instances != 0
                || c.start != start
                || (c.first as u64 + c.count as u64) * 4 > ix.len() as u64
            {
                return Err(PyValueError::new_err("invalid command range"));
            }
            for index in ix[c.first as usize * 4..(c.first + c.count) as usize * 4].chunks_exact(4)
            {
                let j = u32::from_le_bytes(index.try_into().unwrap()) as i64 + c.base as i64;
                if j < 0 || j >= v.len() as i64 / 28 {
                    return Err(PyValueError::new_err("vertex index out of range"));
                }
            }
            start += capacity;
        }
        // Preserve the scene ABI; pack unique edges separately from face vertices.
        let mut memos = Vec::new();
        let (expanded_v, expanded_ix, mut expanded_cmd) =
            edges::expand_with_memos(v, ix, cmd, count, Some(&mut memos)).map_err(PyValueError::new_err)?;
        let (face_v, face_ix, material_shifts) =
            faces::compact_with_slack(v, &expanded_ix, &mut expanded_cmd, count, &memos);
        Ok(Self {
            edges: expanded_v,
            vertices: face_v,
            indices: face_ix,
            commands: expanded_cmd,
            shifts: material_shifts,
            bounds: p.to_vec(),
            codes,
            memos,
        })
    }
}
