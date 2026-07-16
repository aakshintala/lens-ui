use crate::clock::UiClock;
use lens_core::actor::{ActorFeed, SummaryUpdate};
use lens_core::domain::controls::{SandboxStatus, Todo, TodoStatus};
use lens_core::domain::ids::SessionId;
use lens_core::domain::scalars::{SessionLifecycle, SessionStatusValue};
use lens_core::domain::usage::Cost;
use lens_core::reduce::StreamUpdate;

pub const CARD_WIDTH_PX: f32 = 280.0;
pub const CARD_HEIGHT_PX: f32 = 148.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    pub name: String,
    pub branch: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionCard {
    pub session_id: SessionId,
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub activity_summary: String,
    pub last_completed_turn: u32,
    pub seen_turn: u32,
    pub last_completed_at: Option<i64>,
    pub connection_overlay: ConnectionOverlay,
    /// Test/instrumentation: increments on each `cx.notify` from poller folds.
    pub notify_count: u64,
    pub llm_model: Option<String>,
    pub model_override: Option<String>,
    pub agent_name: Option<String>,
    pub harness: Option<String>,
    pub cumulative_cost: Cost,
    pub context_window: Option<u64>,
    pub last_total_tokens: Option<u64>,
    pub sandbox_status: Option<SandboxStatus>,
    pub git_branch: Option<String>,
    pub workspace: Option<String>,
    pub reasoning_effort: Option<String>,
    pub needs_attention: bool,
    pub subagent_active: bool,
    pub last_task_error: Option<lens_core::domain::scalars::ErrorInfo>,
    pub lifecycle: SessionLifecycle,
    pub repos: Vec<RepoRef>,
    pub todos: Vec<Todo>,
    /// Task 4: Ready stamp seeding.
    pub seeded: bool,
    /// Task 4: one-shot decay reschedule guard.
    pub ready_reschedule: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionOverlay {
    #[default]
    Connected,
    Reconnecting,
    Disconnected,
}

impl SessionCard {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            status: SessionStatusValue::Idle,
            title: None,
            activity_summary: String::new(),
            last_completed_turn: 0,
            seen_turn: 0,
            last_completed_at: None,
            connection_overlay: ConnectionOverlay::Connected,
            notify_count: 0,
            llm_model: None,
            model_override: None,
            agent_name: None,
            harness: None,
            cumulative_cost: Cost::default(),
            context_window: None,
            last_total_tokens: None,
            sandbox_status: None,
            git_branch: None,
            workspace: None,
            reasoning_effort: None,
            needs_attention: false,
            subagent_active: false,
            last_task_error: None,
            lifecycle: SessionLifecycle::Active,
            repos: Vec::new(),
            todos: Vec::new(),
            seeded: false,
            ready_reschedule: false,
        }
    }

    pub fn fold_feed(&mut self, frame: ActorFeed, clock: &dyn UiClock) {
        match frame {
            ActorFeed::Summary(u) => self.fold_summary(&u, clock),
            ActorFeed::Detailed(u) => self.fold_detailed(u),
        }
    }

    pub fn fold_summary(&mut self, u: &SummaryUpdate, _clock: &dyn UiClock) {
        self.status = u.status;
        self.title = u.title.clone();
        self.last_total_tokens = u.last_total_tokens;
        self.needs_attention = u.needs_attention;
        self.subagent_active = u.subagent_active;
        self.llm_model = u.llm_model.clone();
        self.model_override = u.model_override.clone();
        self.agent_name = u.agent_name.clone();
        self.cumulative_cost = u.cumulative_cost.clone();
        self.context_window = u.context_window;
        self.sandbox_status = u.sandbox_status.clone();
        self.git_branch = u.git_branch.clone();
        self.workspace = u.workspace.clone();
        self.reasoning_effort = u.reasoning_effort.clone();
        self.activity_summary = u.activity_summary.clone();
        self.last_completed_turn = u.last_completed_turn;
        self.harness = u.harness.clone();
        self.repos = match (&u.workspace, &u.git_branch) {
            (None, None) => Vec::new(),
            (name, branch) => vec![RepoRef {
                name: name.clone().unwrap_or_else(|| "—".into()),
                branch: branch.clone(),
            }],
        };
        // Ready stamp: Task 4
    }

    pub fn fold_detailed(&mut self, u: StreamUpdate) {
        match u {
            StreamUpdate::Rebased(state) => {
                self.status = state.status;
                self.title = state.title.clone();
                self.last_task_error = state.last_task_error.clone();
                self.llm_model = state.llm_model.clone();
                self.model_override = state.model_override.clone();
                self.agent_name = state.agent_name.clone();
                self.cumulative_cost = state.cumulative_cost.clone();
                self.context_window = state.context_window;
                self.last_total_tokens = state.last_total_tokens;
                self.sandbox_status = state.sandbox_status.clone();
                self.git_branch = state.git_branch.clone();
                self.workspace = state.workspace.clone();
                self.reasoning_effort = state.reasoning_effort.clone();
                self.harness = state.harness.clone();
                self.lifecycle = state.lifecycle;
                self.needs_attention = !state.pending_elicitations.is_empty()
                    || state.status == SessionStatusValue::Failed;
                self.todos = state.todos.clone();
                self.recompute_activity();
                self.repos = match (&state.workspace, &state.git_branch) {
                    (None, None) => Vec::new(),
                    (name, branch) => vec![RepoRef {
                        name: name.clone().unwrap_or_else(|| "—".into()),
                        branch: branch.clone(),
                    }],
                };
            }
            StreamUpdate::StatusChanged(s) => self.status = s,
            StreamUpdate::LastTaskErrorChanged(e) => self.last_task_error = e,
            StreamUpdate::UsageChanged(c) => self.cumulative_cost = c,
            StreamUpdate::ModelChanged {
                llm_model,
                model_override,
            } => {
                self.llm_model = llm_model;
                self.model_override = model_override;
            }
            StreamUpdate::ReasoningEffortChanged(e) => self.reasoning_effort = e,
            StreamUpdate::TodosChanged(todos) => {
                self.todos = todos;
                self.recompute_activity();
            }
            StreamUpdate::ScratchChanged(_scratch) => {
                // Focused activity: prefer in-progress todo; scratch is consumed so
                // the match arm exists (activity must not stall while focused).
                self.recompute_activity();
            }
            StreamUpdate::SandboxChanged(s) => self.sandbox_status = s,
            StreamUpdate::TitleChanged(t) => self.title = t,
            StreamUpdate::LastTokensChanged(t) => self.last_total_tokens = t,
            StreamUpdate::ContextWindowChanged(w) => self.context_window = w,
            StreamUpdate::AgentChanged { agent_name, .. } => self.agent_name = agent_name,
            StreamUpdate::ElicitationsChanged(e) => {
                self.needs_attention = !e.is_empty() || self.status == SessionStatusValue::Failed;
            }
            StreamUpdate::Reconnecting { .. } => {
                self.connection_overlay = ConnectionOverlay::Reconnecting;
            }
            StreamUpdate::Reconnected => {
                self.connection_overlay = ConnectionOverlay::Connected;
            }
            StreamUpdate::Disconnected(_) => {
                self.connection_overlay = ConnectionOverlay::Disconnected;
            }
            // SnapshotRestored: pending inputs only — does NOT seed card scalars.
            // ResourcesChanged: valueless marker — no branch delta.
            // TranscriptAdvanced: deferred with transcript slice.
            StreamUpdate::SnapshotRestored(_)
            | StreamUpdate::ResourcesChanged
            | StreamUpdate::TranscriptAdvanced { .. }
            | StreamUpdate::SkillsChanged(_)
            | StreamUpdate::TerminalPendingChanged(_)
            | StreamUpdate::PendingUserChanged(_)
            | StreamUpdate::ChildSessionChanged
            | StreamUpdate::PresenceChanged(_)
            | StreamUpdate::CollaborationModeChanged(_)
            | StreamUpdate::ModelOptionsChanged(_) => {}
        }
    }

    /// Recompute the coarse activity line: the in-progress todo's active_form, else BLANK.
    /// Self-clears — mirrors Summary's `from_state` (which blanks when no in-progress todo).
    /// The card can't do the in-flight-tool fallback that Summary does (it doesn't retain
    /// `items`), so Detailed-mode activity is todos-only; that is an accepted skeleton limit.
    fn recompute_activity(&mut self) {
        self.activity_summary = self
            .todos
            .iter()
            .find(|t| t.status == TodoStatus::InProgress)
            .map(|t| t.active_form.clone())
            .unwrap_or_default();
    }

    pub fn set_repos_for_test(&mut self, repos: Vec<RepoRef>) {
        self.repos = repos;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::domain::controls::{Todo, TodoStatus};
    use lens_core::domain::ids::{AgentId, ConnectionId, SessionId};
    use lens_core::domain::item::StreamScratch;
    use lens_core::domain::session::SessionState;
    use std::sync::Arc;

    fn base_summary() -> SummaryUpdate {
        SummaryUpdate {
            status: SessionStatusValue::Idle,
            title: Some("hello".into()),
            last_total_tokens: Some(1000),
            host_id: None,
            needs_attention: false,
            subagent_active: false,
            llm_model: Some("opus".into()),
            model_override: None,
            agent_name: Some("coder".into()),
            cumulative_cost: Cost {
                total_cost_usd: Some(0.5),
                ..Cost::default()
            },
            context_window: Some(200_000),
            sandbox_status: None,
            git_branch: Some("main".into()),
            workspace: Some("lens".into()),
            reasoning_effort: Some("high".into()),
            activity_summary: String::new(),
            last_completed_turn: 3,
            harness: Some("claude-native".into()),
        }
    }

    #[test]
    fn summary_fold_copies_enriched_scalars_and_seeds_one_repo() {
        let mut card = SessionCard::new(SessionId::new("s"));
        let clock = crate::clock::ManualUiClock::new(0);
        card.fold_feed(ActorFeed::Summary(Box::new(base_summary())), &clock);
        assert_eq!(card.title.as_deref(), Some("hello"));
        assert_eq!(card.llm_model.as_deref(), Some("opus"));
        assert_eq!(card.harness.as_deref(), Some("claude-native"));
        assert_eq!(card.repos.len(), 1);
        assert_eq!(card.repos[0].name, "lens");
        assert_eq!(card.repos[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn detailed_fold_consumes_todos_and_scratch_for_activity() {
        let mut card = SessionCard::new(SessionId::new("s"));
        let clock = crate::clock::ManualUiClock::new(0);
        card.fold_feed(ActorFeed::Summary(Box::new(base_summary())), &clock);

        let mut baseline = SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("s"),
            AgentId::new("ag"),
        );
        baseline.title = Some("rebased".into());
        baseline.llm_model = Some("sonnet".into());
        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::Rebased(Box::new(baseline))),
            &clock,
        );
        assert_eq!(card.title.as_deref(), Some("rebased"));
        assert_eq!(card.llm_model.as_deref(), Some("sonnet"));

        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::TodosChanged(vec![Todo {
                content: "x".into(),
                status: TodoStatus::InProgress,
                active_form: "wiring cards".into(),
            }])),
            &clock,
        );
        assert_eq!(card.activity_summary, "wiring cards");

        // TodosChanged([]) must self-clear the activity (not leave stale "wiring cards").
        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::TodosChanged(vec![])),
            &clock,
        );
        assert_eq!(
            card.activity_summary,
            "",
            "activity self-clears when no in-progress todo"
        );

        // ScratchChanged recomputes from todos (still empty) — proves the arm is not a no-op.
        let scratch = Arc::new(StreamScratch::default());
        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::ScratchChanged(scratch)),
            &clock,
        );
        assert_eq!(card.activity_summary, "");
    }

    #[test]
    fn resources_changed_does_not_clear_branch() {
        let mut card = SessionCard::new(SessionId::new("s"));
        let clock = crate::clock::ManualUiClock::new(0);
        card.fold_feed(ActorFeed::Summary(Box::new(base_summary())), &clock);
        card.fold_feed(ActorFeed::Detailed(StreamUpdate::ResourcesChanged), &clock);
        assert_eq!(card.git_branch.as_deref(), Some("main"));
    }
}
