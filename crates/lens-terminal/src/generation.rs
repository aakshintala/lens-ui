//! Pure resource-generation correlation (Slice 4). No gpui, no I/O.
//!
//! Given our attached identity + target class, decide the lifecycle
//! consequence of a normalized `session.resource.created`/`.deleted` signal.
//! The `saw_delete`-before-`created` discriminator is the positive-reset test;
//! see the plan design note for why a create without a prior delete is benign.

use lens_client::ids::{SessionId, TerminalId};

use crate::{DetachedDetail, TerminalKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceSignal {
    Created {
        session_id: SessionId,
        terminal_id: TerminalId,
        terminal_name: String,
        session_key: String,
    },
    Deleted {
        terminal_id: TerminalId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GenerationVerdict {
    /// Nothing relevant changed.
    Unchanged,
    /// Positive reset for an `OpenOrCreate` key: enter `ReplacementWaiting`.
    AwaitReplacement,
    /// The exact-key successor arrived after a delete: adopt it (fresh engine).
    AdoptSuccessor {
        session_id: SessionId,
        terminal_id: TerminalId,
    },
    /// Identity gone/changed with no adoptable successor.
    Detach(DetachedDetail),
}

/// Per-attachment correlation state. Rebuilt on every fresh attach
/// (initial open, adoption, wake) so `saw_delete` never leaks across identities.
pub(crate) struct GenerationGuard {
    tid: TerminalId,
    /// `Some` => `OpenOrCreate` (adoptable by key); `None` => `Existing` (never adopts).
    key: Option<TerminalKey>,
    saw_delete: bool,
}

impl GenerationGuard {
    pub fn new(tid: TerminalId, key: Option<TerminalKey>) -> Self {
        Self {
            tid,
            key,
            saw_delete: false,
        }
    }

    /// True once a `resource.deleted` for our id has been observed — used by
    /// Sleep/Wake to test whether the same observed generation survived.
    #[cfg_attr(not(test), expect(dead_code, reason = "Task 5 Sleep/Wake dirty check"))]
    pub fn is_dirty(&self) -> bool {
        self.saw_delete
    }

    pub fn on_signal(&mut self, signal: &ResourceSignal) -> GenerationVerdict {
        match signal {
            ResourceSignal::Deleted { terminal_id } if *terminal_id == self.tid => {
                self.saw_delete = true;
                match self.key {
                    Some(_) => GenerationVerdict::AwaitReplacement,
                    None => GenerationVerdict::Detach(DetachedDetail::TerminalGone),
                }
            }
            ResourceSignal::Created {
                session_id,
                terminal_id,
                terminal_name,
                session_key,
            } => match &self.key {
                Some(k) if k.terminal_name == *terminal_name && k.session_key == *session_key => {
                    if self.saw_delete {
                        GenerationVerdict::AdoptSuccessor {
                            session_id: session_id.clone(),
                            terminal_id: terminal_id.clone(),
                        }
                    } else {
                        // DEVIATION (plan design note): a matching create with NO
                        // prior delete is our own echo / a missed-delete recreation.
                        // Detaching here would spuriously kill a healthy tab on a
                        // lagged self-echo; degrade to the accepted missed-event gap.
                        GenerationVerdict::Unchanged
                    }
                }
                _ => GenerationVerdict::Unchanged,
            },
            ResourceSignal::Deleted { .. } => GenerationVerdict::Unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_client::ids::{SessionId, TerminalId};

    fn key(name: &str, sk: &str) -> TerminalKey {
        TerminalKey {
            terminal_name: name.into(),
            session_key: sk.into(),
        }
    }
    fn created(sid: &str, tid: &str, name: &str, sk: &str) -> ResourceSignal {
        ResourceSignal::Created {
            session_id: SessionId::new(sid),
            terminal_id: TerminalId::new(tid),
            terminal_name: name.into(),
            session_key: sk.into(),
        }
    }
    fn deleted(tid: &str) -> ResourceSignal {
        ResourceSignal::Deleted {
            terminal_id: TerminalId::new(tid),
        }
    }

    #[test]
    fn open_or_create_delete_then_matching_create_adopts() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(
            g.on_signal(&deleted("t1")),
            GenerationVerdict::AwaitReplacement
        );
        assert!(g.is_dirty());
        match g.on_signal(&created("sess", "t2", "bash", "s1")) {
            GenerationVerdict::AdoptSuccessor {
                session_id,
                terminal_id,
            } => {
                assert_eq!(session_id.as_str(), "sess");
                assert_eq!(terminal_id.as_str(), "t2");
            }
            other => panic!("expected AdoptSuccessor, got {other:?}"),
        }
    }

    #[test]
    fn open_or_create_create_without_delete_is_benign_echo() {
        // Deliberate deviation from spec §Identity — see plan design note.
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(
            g.on_signal(&created("sess", "t1", "bash", "s1")),
            GenerationVerdict::Unchanged
        );
        assert!(!g.is_dirty());
    }

    #[test]
    fn open_or_create_wrong_key_create_is_unchanged() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(
            g.on_signal(&deleted("t1")),
            GenerationVerdict::AwaitReplacement
        );
        assert_eq!(
            g.on_signal(&created("sess", "t9", "zsh", "s2")),
            GenerationVerdict::Unchanged
        );
        assert!(g.is_dirty());
    }

    #[test]
    fn existing_delete_detaches_terminal_gone() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), None);
        assert_eq!(
            g.on_signal(&deleted("t1")),
            GenerationVerdict::Detach(DetachedDetail::TerminalGone)
        );
    }

    #[test]
    fn existing_ignores_create_for_our_id_without_delete() {
        // Existing never adopts; a bare create echo is benign (missed-event gap on next reconnect).
        let mut g = GenerationGuard::new(TerminalId::new("t1"), None);
        assert_eq!(
            g.on_signal(&created("sess", "t1", "bash", "s1")),
            GenerationVerdict::Unchanged
        );
    }

    #[test]
    fn unrelated_delete_is_unchanged() {
        let mut g = GenerationGuard::new(TerminalId::new("t1"), Some(key("bash", "s1")));
        assert_eq!(g.on_signal(&deleted("other")), GenerationVerdict::Unchanged);
        assert!(!g.is_dirty());
    }
}
