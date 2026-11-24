//! Deadlines driven by the winit and native modal turns. A timer belongs to
//! its view/window. The queue and the callback do not keep that owner alive.
use pyo3::{prelude::*, types::PyWeakrefReference};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

static NEXT_QUEUE: AtomicU64 = AtomicU64::new(1);

struct Deadline {
    at: Option<Instant>,
    interval: Duration,
    repeat: bool,
    delivering: bool,
}

impl Deadline {
    fn start(&mut self, now: Instant, ms: Option<u32>) {
        if let Some(ms) = ms {
            self.interval = Duration::from_millis(ms.into());
        }
        self.at = Some(now + self.interval);
    }

    fn remaining(&self, now: Instant) -> Option<u64> {
        self.at.map(|at| {
            // Never wake early because a sub-millisecond remainder rounded down.
            at.saturating_duration_since(now).as_micros().div_ceil(1000) as u64
        })
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.at.is_none_or(|at| at > now) {
            return false;
        }
        // Coalesce missed periods after sleep. Consume before delivery so the
        // callback can cancel, restart or enter a nested modal turn.
        self.at = self
            .repeat
            .then(|| now + self.interval.max(Duration::from_millis(1)));
        true
    }
}

struct Slot {
    owner: AtomicU64,
    deadline: Mutex<Deadline>,
    context: Option<Py<PyWeakrefReference>>,
    callback: Callback,
}

enum Callback {
    WeakMethod(Py<PyAny>),
    Function(Py<PyAny>),
}

impl Slot {
    fn belongs_to(&self, context: &Bound<'_, PyAny>) -> bool {
        self.context
            .as_ref()
            .and_then(|r| r.bind(context.py()).upgrade())
            .is_some_and(|o| o.is(context))
    }

    fn deliver(&self, py: Python<'_>) -> PyResult<()> {
        let cb = match &self.callback {
            Callback::WeakMethod(reference) => reference.bind(py).call0()?,
            Callback::Function(f) => f.bind(py).clone(),
        };
        if cb.is_none()
            || self
                .context
                .as_ref()
                .is_some_and(|r| r.bind(py).upgrade().is_none())
        {
            self.deadline.lock().unwrap().at = None;
        } else {
            cb.call0()?;
        }
        Ok(())
    }
}

#[pyclass]
pub struct WindowTimer(Arc<Slot>);

#[pymethods]
#[allow(non_snake_case)]
impl WindowTimer {
    #[pyo3(signature=(ms=None))]
    fn start(&self, ms: Option<u32>) {
        self.0.deadline.lock().unwrap().start(Instant::now(), ms);
    }

    fn stop(&self) {
        self.0.deadline.lock().unwrap().at = None;
    }

    fn isActive(&self) -> bool {
        self.0.deadline.lock().unwrap().at.is_some()
    }

    fn remainingTime(&self) -> i64 {
        self.0
            .deadline
            .lock()
            .unwrap()
            .remaining(Instant::now())
            .map_or(-1, |ms| ms as i64)
    }
}

#[pyclass]
pub struct WindowTimers {
    id: u64,
    slots: Mutex<Vec<Weak<Slot>>>,
    // Fire-and-forget callbacks are retained only until delivery or cancellation.
    pending: Mutex<Vec<Arc<Slot>>>,
}

impl WindowTimers {
    fn live(&self) -> Vec<Weak<Slot>> {
        let mut slots = self.slots.lock().unwrap();
        slots.retain(|s| {
            s.upgrade()
                .is_some_and(|s| s.owner.load(Ordering::Relaxed) == self.id)
        });
        slots.clone()
    }
}

#[pymethods]
impl WindowTimers {
    #[new]
    fn new() -> Self {
        Self {
            id: NEXT_QUEUE.fetch_add(1, Ordering::Relaxed),
            slots: Mutex::default(),
            pending: Mutex::default(),
        }
    }

    #[pyo3(signature=(method, interval=0, repeat=false, context=None))]
    fn timer(
        &self,
        method: &Bound<'_, PyAny>,
        interval: u32,
        repeat: bool,
        context: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<WindowTimer> {
        let callback = if method.getattr("__self__").is_ok() && method.hasattr("__func__")? {
            Callback::WeakMethod(
                method
                    .py()
                    .import("weakref")?
                    .call_method1("WeakMethod", (method,))?
                    .unbind(),
            )
        } else {
            Callback::Function(method.clone().unbind())
        };
        let context = context
            .map(PyWeakrefReference::new)
            .transpose()?
            .map(Bound::unbind);
        let slot = Arc::new(Slot {
            owner: AtomicU64::new(self.id),
            deadline: Mutex::new(Deadline {
                at: None,
                interval: Duration::from_millis(interval.into()),
                repeat,
                delivering: false,
            }),
            context,
            callback,
        });
        self.slots.lock().unwrap().push(Arc::downgrade(&slot));
        Ok(WindowTimer(slot))
    }

    fn later(
        &self,
        context: &Bound<'_, PyAny>,
        ms: u32,
        callback: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let timer = self.timer(callback, ms, false, Some(context))?;
        timer.start(None);
        self.pending.lock().unwrap().push(timer.0);
        Ok(())
    }

    fn cancel_context(&self, context: &Bound<'_, PyAny>) {
        for slot in self.live().into_iter().filter_map(|s| s.upgrade()) {
            if slot.belongs_to(context) {
                slot.deadline.lock().unwrap().at = None;
            }
        }
        let mut pending = self.pending.lock().unwrap();
        let dropped = pending
            .extract_if(.., |s| s.belongs_to(context))
            .collect::<Vec<_>>();
        drop(pending);
        drop(dropped);
    }

    fn transfer_context(&self, source: &WindowTimers, context: &Bound<'_, PyAny>) {
        if self.id == source.id {
            return;
        }
        for slot in source.live().into_iter().filter_map(|s| s.upgrade()) {
            if slot.belongs_to(context) {
                self.adopt(&WindowTimer(slot));
            }
        }
        self.pending.lock().unwrap().extend(
            source
                .pending
                .lock()
                .unwrap()
                .extract_if(.., |s| s.belongs_to(context)),
        );
    }

    /// A moved project keeps its scheduled deadlines on the destination window.
    fn adopt(&self, timer: &WindowTimer) {
        let slot = &timer.0;
        if slot.owner.swap(self.id, Ordering::Relaxed) != self.id {
            let weak = Arc::downgrade(slot);
            let mut slots = self.slots.lock().unwrap();
            if !slots.iter().any(|other| other.ptr_eq(&weak)) {
                slots.push(weak);
            }
        }
    }

    /// Release this window's timers on shutdown. A transferred document's
    /// handles may still appear as stale entries in the source queue.
    fn cancel_all(&self) {
        let _released = std::mem::take(&mut *self.pending.lock().unwrap());
        let slots = std::mem::take(&mut *self.slots.lock().unwrap());
        for slot in slots.into_iter().filter_map(|weak| weak.upgrade()) {
            if slot.owner.load(Ordering::Relaxed) == self.id {
                slot.deadline.lock().unwrap().at = None;
            }
        }
    }

    fn next_ms(&self) -> Option<u64> {
        let now = Instant::now();
        self.live()
            .into_iter()
            .filter_map(|weak| {
                let slot = weak.upgrade()?;
                let timer = slot.deadline.lock().unwrap();
                if timer.delivering {
                    None
                } else {
                    timer.remaining(now)
                }
            })
            .min()
    }

    fn run_due(&self, py: Python<'_>) -> PyResult<()> {
        let now = Instant::now();
        let mut first_error = None;
        for weak in self.live() {
            let Some(slot) = weak.upgrade() else { continue };
            {
                let mut timer = slot.deadline.lock().unwrap();
                if slot.owner.load(Ordering::Relaxed) != self.id
                    || timer.delivering
                    || !timer.take_due(now)
                {
                    continue;
                }
                timer.delivering = true;
            }
            let result = slot.deliver(py);
            slot.deadline.lock().unwrap().delivering = false;
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        let mut pending = self.pending.lock().unwrap();
        let finished = pending
            .extract_if(.., |s| {
                let d = s.deadline.lock().unwrap();
                d.at.is_none() && !d.delivering
            })
            .collect::<Vec<_>>();
        drop(pending);
        drop(finished);
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_restarted_and_overdue_timers() {
        let now = Instant::now();
        let after = |ms| now + Duration::from_millis(ms);
        let mut timer = Deadline {
            at: None,
            interval: Duration::ZERO,
            repeat: false,
            delivering: false,
        };
        timer.start(now, Some(50));
        assert_eq!(timer.remaining(after(13)), Some(37));
        timer.start(after(30), None);
        assert!(!timer.take_due(after(50)));
        assert!(timer.take_due(after(80)));
        assert_eq!(timer.remaining(after(80)), None);
        timer.repeat = true;
        timer.start(now, Some(20));
        assert!(timer.take_due(after(1000)));
        assert_eq!(timer.remaining(after(1000)), Some(20));
        assert!(!timer.take_due(after(1000)));
        timer.at = None;
        assert!(!timer.take_due(after(2000)));
    }
}
