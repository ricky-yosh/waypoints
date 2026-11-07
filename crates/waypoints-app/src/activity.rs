//! Synthetic fixture output, run-event formatting, and periodic run polling.

use std::time::Duration;

use rune::Tx;
use waypoints_backend::RunStatus;
use waypoints_core::run::RunState;

use crate::backend::Backend;
use crate::fixture;
use crate::state::{Mode, OutputLine, State};

pub(crate) const OUTPUT_INTERVAL: Duration = Duration::from_millis(120);
pub(crate) const MAX_OUTPUT_LINES: usize = 80;
const RUN_POLL_INTERVAL: f64 = 0.25;
/// External workflow/run changes appear within this interval without manual refresh.
pub(crate) const SYNC_INTERVAL: f64 = 2.0;

pub(crate) fn run_is_active(run: &RunStatus) -> bool {
    matches!(
        run.run.state,
        RunState::Starting
            | RunState::Running
            | RunState::AwaitingPermission
            | RunState::Cancelling
            | RunState::Finalizing
    )
}

pub(crate) fn status_for_run(state: &RunState) -> fixture::Status {
    match state {
        RunState::Completed => fixture::Status::Done,
        RunState::Failed { .. } | RunState::Interrupted { .. } => fixture::Status::Blocked,
        RunState::AwaitingPermission => fixture::Status::NeedsPermission,
        RunState::Cancelled | RunState::Recovered { .. } => fixture::Status::Ready,
        _ => fixture::Status::Running,
    }
}

fn push_output(s: &mut State) {
    let seq = s.output_seq;
    s.output_seq += 1;
    s.output.push(OutputLine {
        seq,
        text: fixture::synthetic_line(seq),
    });
    if s.output.len() > MAX_OUTPUT_LINES {
        let excess = s.output.len() - MAX_OUTPUT_LINES;
        s.output.drain(0..excess);
    }
}

pub(crate) fn seed_output(s: &mut State, count: usize) {
    for _ in 0..count {
        push_output(s);
    }
}

pub(crate) fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    s.tick_preferences(clock);
    crate::sync_native_menus(s);
    let mut changed = s.expire_copy_notice(clock);
    let updates = s.backend.as_ref().map(Backend::drain).unwrap_or_default();
    for update in updates {
        s.apply_backend_update(update);
        changed = true;
    }
    if s.mode == Mode::Backend && !s.display_fixture && s.project_path.is_some() {
        if s.next_sync_at <= 0.0 {
            s.next_sync_at = clock + SYNC_INTERVAL;
            tx.wake_after(Duration::from_secs_f64(SYNC_INTERVAL));
        } else if clock + 1e-9 >= s.next_sync_at {
            s.sync_now();
            s.next_sync_at = clock + SYNC_INTERVAL;
            tx.wake_after(Duration::from_secs_f64(SYNC_INTERVAL));
            changed = true;
        }
    }
    if s.mode == Mode::Backend
        && !s.run_events_pending
        && clock >= s.next_run_poll
        && s.selected_run().is_some_and(run_is_active)
    {
        s.request_run_events();
        s.next_run_poll = clock + RUN_POLL_INTERVAL;
    }
    if s.streaming && clock + 1e-9 >= s.next_output_at {
        push_output(s);
        s.next_output_at = clock + OUTPUT_INTERVAL.as_secs_f64();
        tx.wake_after(OUTPUT_INTERVAL);
        changed = true;
    }
    changed
}
