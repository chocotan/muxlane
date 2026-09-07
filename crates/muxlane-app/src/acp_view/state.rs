use muxlane_acp::{PromptId, PromptSubmission, TurnState};
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectorTarget {
    Mode,
    More,
    Config(String),
}

impl SelectorTarget {
    pub(crate) fn config(id: impl Into<String>) -> Self {
        Self::Config(id.into())
    }

    pub(crate) fn matches_id(&self, id: &str) -> bool {
        match self {
            Self::Mode => id == "mode",
            Self::More => id == "more",
            Self::Config(config_id) => config_id == id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DispatchPhase {
    Idle,
    Capturing(CaptureToken),
    AwaitingAcceptance(PromptId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureToken {
    pub(crate) session_epoch: u64,
    pub(crate) prompt_id: PromptId,
}

pub(crate) struct CaptureContext<'a> {
    pub(crate) session_epoch: u64,
    pub(crate) front_id: Option<&'a PromptId>,
    pub(crate) paused: bool,
    pub(crate) connected: bool,
    pub(crate) turn_state: TurnState,
    pub(crate) phase: &'a DispatchPhase,
    pub(crate) checkpoint_busy: bool,
    pub(crate) restore_confirmed: bool,
}

impl CaptureToken {
    pub(crate) fn is_valid(&self, context: CaptureContext<'_>) -> bool {
        self.session_epoch == context.session_epoch
            && context.front_id == Some(&self.prompt_id)
            && !context.paused
            && context.connected
            && context.turn_state == TurnState::Idle
            && !context.checkpoint_busy
            && !context.restore_confirmed
            && matches!(context.phase, DispatchPhase::Capturing(token) if token == self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptQueue {
    pub(crate) submissions: VecDeque<PromptSubmission>,
    pub(crate) paused: bool,
    pub(crate) phase: DispatchPhase,
}

impl PromptQueue {
    pub(crate) fn new(
        submissions: impl IntoIterator<Item = PromptSubmission>,
        paused: bool,
    ) -> Self {
        Self {
            submissions: submissions.into_iter().collect(),
            paused,
            phase: DispatchPhase::Idle,
        }
    }

    pub(crate) fn front(&self) -> Option<&PromptSubmission> {
        self.submissions.front()
    }

    pub(crate) fn enqueue(&mut self, submission: PromptSubmission) {
        self.submissions.push_back(submission);
    }

    pub(crate) fn invalidate(&mut self) {
        self.phase = DispatchPhase::Idle;
    }

    pub(crate) fn suspend_uncertain_submission(&mut self) {
        if matches!(self.phase, DispatchPhase::AwaitingAcceptance(_)) {
            self.paused = true;
        }
        self.phase = DispatchPhase::Idle;
    }

    pub(crate) fn cancel_capture(&mut self, token: &CaptureToken) {
        if matches!(&self.phase, DispatchPhase::Capturing(current) if current == token) {
            self.phase = DispatchPhase::Idle;
        }
    }

    pub(crate) fn begin_capture(&mut self, token: CaptureToken) {
        self.phase = DispatchPhase::Capturing(token);
    }

    pub(crate) fn await_acceptance(&mut self, id: PromptId) {
        self.phase = DispatchPhase::AwaitingAcceptance(id);
    }

    pub(crate) fn clear_pending(&mut self) {
        self.phase = DispatchPhase::Idle;
    }

    pub(crate) fn accept(&mut self, id: &PromptId) -> bool {
        if self.front().is_some_and(|submission| &submission.id == id) {
            self.submissions.pop_front();
            self.phase = DispatchPhase::Idle;
            true
        } else {
            false
        }
    }

    pub(crate) fn remove_front(&mut self) {
        self.invalidate();
        self.submissions.pop_front();
    }

    pub(crate) fn clear(&mut self) {
        self.invalidate();
        self.submissions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_acp::PromptPayload;

    fn submission(text: &str) -> PromptSubmission {
        PromptSubmission::new(PromptPayload::new(text))
    }

    #[test]
    fn queue_pops_only_on_matching_acceptance() {
        let first = submission("first");
        let second = submission("second");
        let first_id = first.id.clone();
        let second_id = second.id.clone();
        let mut queue = PromptQueue::new([first, second], false);
        queue.await_acceptance(first_id.clone());
        assert_eq!(queue.submissions.len(), 2);
        assert!(!queue.accept(&second_id));
        assert_eq!(queue.submissions.len(), 2);
        assert!(queue.accept(&first_id));
        assert_eq!(
            queue.submissions.front().map(|item| &item.id),
            Some(&second_id)
        );
    }

    #[test]
    fn uncertain_submission_is_retained_and_paused_for_restart() {
        let submission = submission("prompt");
        let id = submission.id.clone();
        let mut queue = PromptQueue::new([submission], false);
        queue.await_acceptance(id);

        queue.suspend_uncertain_submission();

        assert_eq!(queue.submissions.len(), 1);
        assert!(queue.paused);
        assert_eq!(queue.phase, DispatchPhase::Idle);
    }

    #[test]
    fn capture_token_rejects_changed_guards() {
        let submission = submission("prompt");
        let token = CaptureToken {
            session_epoch: 4,
            prompt_id: submission.id.clone(),
        };
        let phase = DispatchPhase::Capturing(token.clone());
        let context =
            |session_epoch, front_id, paused, turn_state, checkpoint_busy| CaptureContext {
                session_epoch,
                front_id,
                paused,
                connected: true,
                turn_state,
                phase: &phase,
                checkpoint_busy,
                restore_confirmed: false,
            };
        assert!(token.is_valid(context(
            4,
            Some(&submission.id),
            false,
            TurnState::Idle,
            false,
        )));
        assert!(!token.is_valid(context(
            5,
            Some(&submission.id),
            false,
            TurnState::Idle,
            false,
        )));
        assert!(!token.is_valid(context(
            4,
            Some(&submission.id),
            true,
            TurnState::Idle,
            false,
        )));
        assert!(!token.is_valid(context(
            4,
            Some(&submission.id),
            false,
            TurnState::Generating,
            false,
        )));
        assert!(!token.is_valid(context(4, None, false, TurnState::Idle, false)));
        assert!(!token.is_valid(context(
            4,
            Some(&submission.id),
            false,
            TurnState::Idle,
            true,
        )));
        assert!(!token.is_valid(CaptureContext {
            session_epoch: 4,
            front_id: Some(&submission.id),
            paused: false,
            connected: true,
            turn_state: TurnState::Idle,
            phase: &phase,
            checkpoint_busy: false,
            restore_confirmed: true,
        }));
    }

    #[test]
    fn stale_capture_completion_does_not_cancel_current_capture() {
        let first = submission("first");
        let second = submission("second");
        let stale = CaptureToken {
            session_epoch: 1,
            prompt_id: first.id,
        };
        let current = CaptureToken {
            session_epoch: 2,
            prompt_id: second.id,
        };
        let mut queue = PromptQueue::new([], false);
        queue.begin_capture(current.clone());

        queue.cancel_capture(&stale);

        assert_eq!(queue.phase, DispatchPhase::Capturing(current));
    }

    #[test]
    fn selector_config_ids_do_not_collide_with_reserved_targets() {
        assert_ne!(SelectorTarget::config("mode"), SelectorTarget::Mode);
        assert_ne!(SelectorTarget::config("more"), SelectorTarget::More);
        assert_ne!(SelectorTarget::config("sessions"), SelectorTarget::More);
    }
}
