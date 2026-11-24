// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! `Value`: the JSON-like data the document keeps for fields that are
//! free-form today (a group's paint dict, a tag's colour, later the custom
//! attributes of plugins and records).
//!
//! It round-trips Python exactly: a tuple comes back a tuple, a list a
//! list, dict order is kept, `True` stays a bool and not an int.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

/// A Python object that is not JSON-like (a set an extension put in its
/// plugin data, an array): kept as the object itself, compared by identity.
/// Saves drop what they cannot write, as they did before.
#[derive(Clone)]
pub struct Opaque(pub std::sync::Arc<Py<PyAny>>);

impl PartialEq for Opaque {
    fn eq(&self, other: &Opaque) -> bool {
        self.0.as_ptr() == other.0.as_ptr()
    }
}

impl std::fmt::Debug for Opaque {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Opaque({:p})", self.0.as_ptr())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    ElementKey(u128),
    Vector([f64; 3]),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    /// Insertion order, as a Python dict.
    Dict(Vec<(Value, Value)>),
    Opaque(Opaque),
}

const MAX_DEPTH: usize = 64;

impl Value {
    pub fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
        Self::from_py_depth(obj, 0)
    }

    fn from_py_depth(obj: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
        if depth > MAX_DEPTH {
            return Err(PyTypeError::new_err("document value nested too deep"));
        }
        if obj.is_none() {
            return Ok(Value::None);
        }
        if let Ok(b) = obj.cast::<PyBool>() {
            return Ok(Value::Bool(b.is_true()));
        }
        if obj.is_instance_of::<PyInt>() {
            return match obj.extract::<i64>() {
                Ok(i) => Ok(Value::Int(i)),
                Err(_) => Ok(Value::ElementKey(obj.extract()?)),
            };
        }
        if let Ok(f) = obj.cast::<PyFloat>() {
            return Ok(Value::Float(f.value()));
        }
        if let Ok(s) = obj.cast::<PyString>() {
            return Ok(Value::Str(s.to_str()?.to_owned()));
        }
        if let Ok(t) = obj.cast::<PyTuple>() {
            let mut out = Vec::with_capacity(t.len());
            for item in t.iter() {
                out.push(Self::from_py_depth(&item, depth + 1)?);
            }
            return Ok(Value::Tuple(out));
        }
        if let Ok(l) = obj.cast::<PyList>() {
            let mut out = Vec::with_capacity(l.len());
            for item in l.iter() {
                out.push(Self::from_py_depth(&item, depth + 1)?);
            }
            return Ok(Value::List(out));
        }
        if let Ok(d) = obj.cast::<PyDict>() {
            let mut out = Vec::with_capacity(d.len());
            for (k, v) in d.iter() {
                out.push((Self::from_py_depth(&k, depth + 1)?, Self::from_py_depth(&v, depth + 1)?));
            }
            return Ok(Value::Dict(out));
        }
        // NumPy scalars and other numbers that are not int/float subclasses.
        if obj.hasattr("__index__")? {
            if let Ok(i) = obj.extract::<i64>() {
                return Ok(Value::Int(i));
            }
        }
        if obj.hasattr("__float__")? {
            if let Ok(f) = obj.extract::<f64>() {
                return Ok(Value::Float(f));
            }
        }
        if depth == usize::MAX {
            return Err(PyTypeError::new_err(format!(
                "the document keeps JSON-like values (None, bool, int, float, str, list, tuple, dict), not {}",
                obj.get_type().name()?
            )));
        }
        Ok(Value::Opaque(Opaque(std::sync::Arc::new(obj.clone().unbind()))))
    }

    pub fn to_py<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(match self {
            Value::None => py.None().into_bound(py),
            Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
            Value::Int(i) => i.into_pyobject(py)?.into_any(),
            Value::Vector(v) => Py::new(py, crate::math::Vec3::from(glam::DVec3::from(*v)))?.into_bound(py).into_any(),
            Value::ElementKey(k) => k.into_pyobject(py)?.into_any(),
            Value::Float(f) => PyFloat::new(py, *f).into_any(),
            Value::Str(s) => PyString::new(py, s).into_any(),
            Value::List(items) => {
                let l = PyList::empty(py);
                for v in items {
                    l.append(v.to_py(py)?)?;
                }
                l.into_any()
            }
            Value::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    out.push(v.to_py(py)?);
                }
                PyTuple::new(py, out)?.into_any()
            }
            Value::Dict(items) => {
                let d = PyDict::new(py);
                for (k, v) in items {
                    d.set_item(k.to_py(py)?, v.to_py(py)?)?;
                }
                d.into_any()
            }
            Value::Opaque(o) => o.0.clone_ref(py).into_bound(py),
        })
    }

    /// Bytes this value holds on the heap.
    #[allow(dead_code)]
    pub fn heap_bytes(&self) -> usize {
        match self {
            Value::Str(s) => s.capacity(),
            Value::List(v) | Value::Tuple(v) => {
                v.capacity() * std::mem::size_of::<Value>() + v.iter().map(Value::heap_bytes).sum::<usize>()
            }
            Value::Dict(v) => {
                v.capacity() * std::mem::size_of::<(Value, Value)>()
                    + v.iter().map(|(a, b)| a.heap_bytes() + b.heap_bytes()).sum::<usize>()
            }
            _ => 0,
        }
    }
}

/// Replay a change to a scene table without replacing unrelated entries
/// added, removed or edited outside History (e.g. the links registry).
/// The usual case restores exactly the recorded value and insertion order.
pub fn replay_table(current: &Value, from: &Value, to: &Value) -> Value {
    if current == from {
        return to.clone();
    }
    match (current, from, to) {
        (Value::Dict(now), Value::Dict(before), Value::Dict(after)) => {
            let mut result = now.clone();
            result.retain(|(k, _)| !before.iter().any(|(b, _)| b == k)
                || after.iter().any(|(a, _)| a == k));
            for (i, (key, new)) in after.iter().enumerate() {
                let old = before.iter().find(|(k, _)| k == key).map(|(_, v)| v);
                if old == Some(new) { continue; }
                if let Some((_, val)) = result.iter_mut().find(|(k, _)| k == key) {
                    *val = old.map_or_else(|| new.clone(), |old| replay_table(val, old, new));
                } else {
                    result.insert(i.min(result.len()), (key.clone(), new.clone()));
                }
            }
            Value::Dict(result)
        }
        // Folder paths are unique strings. Keep folders made in the panel
        // since the command, inserting restored paths in their old order.
        (Value::List(now), Value::List(before), Value::List(after))
            if now.iter().chain(before).chain(after).all(|v| matches!(v, Value::Str(_))) => {
            let mut result: Vec<_> = now.iter()
                .filter(|v| !before.contains(v) || after.contains(v)).cloned().collect();
            for (i, value) in after.iter().enumerate() {
                if !before.contains(value) && !result.contains(value) {
                    result.insert(i.min(result.len()), value.clone());
                }
            }
            Value::List(result)
        }
        _ => to.clone(),
    }
}

/// The value at ``path`` in ``root`` (dict keys, list indices), mutable.
fn slot_mut<'a>(root: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    let mut at = root;
    for step in path {
        at = match at {
            Value::Dict(items) => items.iter_mut().find(|(k, _)| k == step).map(|(_, v)| v)?,
            Value::List(items) | Value::Tuple(items) => match step {
                Value::Int(i) if *i >= 0 => items.get_mut(*i as usize)?,
                _ => return None,
            },
            _ => return None,
        };
    }
    Some(at)
}

/// Put ``new`` at ``path`` in ``root`` (None: take the entry out): a dict
/// key keeps its place or goes last, a list index one past the end
/// appends. The empty path is the whole value. Returns what was there
/// (None: nothing), or Err when the path does not lead into a container.
pub fn put_at(root: &mut Value, path: &[Value], new: Option<Value>) -> Result<Option<Value>, String> {
    let Some((last, up)) = path.split_last() else {
        return match new {
            Some(v) => Ok(Some(std::mem::replace(root, v))),
            None => Err("the whole value cannot be taken out".into()),
        };
    };
    let parent = slot_mut(root, up).ok_or("no container at that path")?;
    match parent {
        Value::Dict(items) => {
            let i = items.iter().position(|(k, _)| k == last);
            Ok(match (i, new) {
                (Some(i), Some(v)) => Some(std::mem::replace(&mut items[i].1, v)),
                (Some(i), None) => Some(items.remove(i).1),
                (None, Some(v)) => { items.push((last.clone(), v)); None }
                (None, None) => None,
            })
        }
        Value::List(items) => {
            let Value::Int(i) = last else { return Err("a list index is an int".into()) };
            let i = usize::try_from(*i).map_err(|_| "a negative list index")?;
            Ok(match new {
                Some(v) if i < items.len() => Some(std::mem::replace(&mut items[i], v)),
                Some(v) if i == items.len() => { items.push(v); None }
                None if i < items.len() => Some(items.remove(i)),
                _ => return Err("list index out of range".into()),
            })
        }
        _ => Err("no container at that path".into()),
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    fn s(x: &str) -> Value { Value::Str(x.into()) }

    #[test]
    fn put_at_sets_removes_and_appends() {
        let mut v = Value::Dict(vec![(s("a"), Value::Dict(vec![(s("n"), Value::Int(1))])), (s("l"), Value::List(vec![]))]);
        assert_eq!(put_at(&mut v, &[s("a"), s("n")], Some(Value::Int(2))).unwrap(), Some(Value::Int(1)));
        assert_eq!(put_at(&mut v, &[s("a"), s("m")], Some(Value::Int(3))).unwrap(), None);
        assert_eq!(put_at(&mut v, &[s("l"), Value::Int(0)], Some(Value::Int(9))).unwrap(), None);
        assert!(put_at(&mut v, &[s("l"), Value::Int(5)], Some(Value::Int(9))).is_err());
        assert_eq!(put_at(&mut v, &[s("a"), s("n")], None).unwrap(), Some(Value::Int(2)));
        assert_eq!(v, Value::Dict(vec![(s("a"), Value::Dict(vec![(s("m"), Value::Int(3))])),
                                       (s("l"), Value::List(vec![Value::Int(9)]))]));
        assert!(put_at(&mut v, &[s("x"), s("y")], Some(Value::None)).is_err());
    }
}
