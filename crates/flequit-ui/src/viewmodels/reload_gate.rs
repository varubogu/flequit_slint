//! Coalescing of full project reloads.
//!
//! Every mutation asks for a reload of the whole project tree, which keeps the
//! displayed data unambiguously correct but is the most expensive thing the
//! ViewModel does. A burst of edits — dragging a task between lists, ticking
//! several subtasks, holding a reorder shortcut — used to run one reload per
//! edit, each reading the same rows and all but the last publishing a snapshot
//! that was replaced before it could be read.
//!
//! The gate keeps at most one reload in flight and remembers that more were
//! asked for while it ran, so a burst of any length costs two reloads: the one
//! already running, and one more that observes every write in the burst.

/// Tracks whether a reload is running and whether another one is owed.
#[derive(Debug, Default)]
pub struct ReloadGate {
    running: bool,
    pending: bool,
}

impl ReloadGate {
    /// Claims the right to run a reload.
    ///
    /// `false` means one is already in flight; the request has been recorded
    /// and the running reload will serve it, so the caller must not start its
    /// own.
    pub fn begin(&mut self) -> bool {
        if self.running {
            self.pending = true;
            return false;
        }
        self.running = true;
        true
    }

    /// Reports the end of a reload.
    ///
    /// `true` means a request arrived while it was running, so the caller has
    /// to reload once more before releasing the gate.
    pub fn finish(&mut self) -> bool {
        if self.pending {
            self.pending = false;
            return true;
        }
        self.running = false;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::ReloadGate;

    #[test]
    fn first_request_runs() {
        let mut gate = ReloadGate::default();
        assert!(gate.begin());
        assert!(!gate.finish());
    }

    #[test]
    fn request_during_a_reload_is_served_once_afterwards() {
        let mut gate = ReloadGate::default();
        assert!(gate.begin());

        // Three more edits land while the first reload is still reading.
        assert!(!gate.begin());
        assert!(!gate.begin());
        assert!(!gate.begin());

        // They cost a single extra pass, not three.
        assert!(gate.finish());
        assert!(!gate.finish());
    }

    #[test]
    fn the_gate_is_reusable_after_it_drains() {
        let mut gate = ReloadGate::default();
        assert!(gate.begin());
        assert!(!gate.finish());

        assert!(gate.begin());
        assert!(!gate.begin());
        assert!(gate.finish());
        assert!(!gate.finish());
    }
}
