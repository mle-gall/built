//! An export preview may run the host's modal pump. An edit or dismissal
//! during that call makes its result stale. Only one render is admitted.
#[derive(Default)]
pub(super) struct Preview {
    pub enabled: bool,
    pub ready: bool,
    pub error: String,
    revision: Option<String>,
    ticket: u64,
    running: bool,
}

impl Preview {
    pub fn update(&mut self, revision: Option<String>) -> Option<u64> {
        if self.revision == revision {
            return None;
        }
        self.revision = revision;
        self.ticket += 1;
        self.error.clear();
        if self.revision.is_none() {
            self.ready = false;
        }
        (!self.running && self.revision.is_some()).then_some(self.ticket)
    }

    pub fn begin(&mut self, ticket: u64) -> bool {
        if self.running || self.revision.is_none() || ticket != self.ticket {
            return false;
        }
        self.running = true;
        true
    }

    pub fn current(&self, ticket: u64) -> bool {
        self.enabled && self.revision.is_some() && ticket == self.ticket
    }

    pub fn finish(&mut self, ticket: u64, error: String) -> bool {
        self.running = false;
        if !self.current(ticket) {
            // The next build can now schedule the edit received during rendering.
            self.revision = None;
            return false;
        }
        self.ready = error.is_empty();
        self.error = error;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_coalesce_and_closed_sheets_discard_results() {
        let mut p = Preview {
            enabled: true,
            ..Default::default()
        };
        let first = p.update(Some("A".into())).unwrap();
        let second = p.update(Some("B".into())).unwrap();
        assert!(!p.begin(first));
        assert!(p.begin(second));
        assert!(p.update(Some("C".into())).is_none());
        assert!(!p.finish(second, String::new()));
        let third = p.update(Some("C".into())).unwrap();
        assert!(p.begin(third));
        p.update(None);
        assert!(!p.finish(third, String::new()));
        assert!(!p.ready);
    }
}
