use pyo3::prelude::*;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::{fs::File, io, path::PathBuf};

/// Keep an unpublished save beside its destination so that rename is atomic.
/// A failed encoder, cancelled save or failed rename discards only this file.
#[pyclass]
pub struct AtomicDocument {
    target: PathBuf,
    temp: Option<PathBuf>,
    file: Option<File>,
}

impl Drop for AtomicDocument {
    fn drop(&mut self) {
        self.discard();
    }
}

#[pymethods]
impl AtomicDocument {
    #[new]
    fn new(py: Python<'_>, target: PathBuf) -> PyResult<Self> {
        Ok(py.detach(|| Self::create(target))?)
    }

    #[cfg(unix)]
    fn fileno(&self) -> PyResult<i32> {
        self.file
            .as_ref()
            .map(File::as_raw_fd)
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("closed document output"))
    }

    fn write(&mut self, py: Python<'_>, bytes: &[u8]) -> PyResult<()> {
        use std::io::Write;
        Ok(py.detach(|| self.output()?.write_all(bytes))?)
    }

    fn publish(&mut self, py: Python<'_>) -> PyResult<()> {
        Ok(py.detach(|| self.commit())?)
    }

    pub fn discard(&mut self) {
        self.file.take();
        if let Some(path) = self.temp.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl AtomicDocument {
    pub fn create(target: PathBuf) -> io::Result<Self> {
        use std::hash::{BuildHasher, RandomState};
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let id = RandomState::new().hash_one(NEXT.fetch_add(1, Ordering::Relaxed));
            let temp = target.with_file_name(format!(".{id:016x}.part"));
            match File::create_new(&temp) {
                Ok(file) => {
                    return Ok(Self {
                        target,
                        temp: Some(temp),
                        file: Some(file),
                    })
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
    pub fn output(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("closed output"))
    }
    pub fn commit(&mut self) -> io::Result<()> {
        self.file.take();
        let temp = self
            .temp
            .as_ref()
            .ok_or_else(|| io::Error::other("closed document output"))?;
        std::fs::rename(temp, &self.target)?;
        self.temp.take();
        Ok(())
    }
}
