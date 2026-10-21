//! The logic of the SFTP transfer queue (night plan, stage 4, item 3; UI 5;
//! night decision D46), without I/O: the panel and the runner call it and
//! show what it says.
//!
//! - A transfer is queued, running, waiting for a name conflict answer,
//!   paused, failed (with its reason; Retry queues it again and the runner
//!   resumes from its `.part`), done or cancelled.
//! - At most [`TransferQueue::parallel`] transfers run at once (setting
//!   `sftp.parallel_transfers`, 1 to 10, default 3); the oldest queued one
//!   starts first.
//! - A name conflict follows the policy (setting `sftp.name_conflict`): ask,
//!   replace, skip or rename. "Ask" waits for an answer, which may apply to
//!   every later conflict of the queue ("apply to all"). A waiting transfer
//!   does not hold a running slot.
//! - Speed is measured over the last [`SPEED_WINDOW`]; the time left follows
//!   from it.
//! - [`renamed`] picks the name of a "rename" answer: ` (2)`, ` (3)`, …
//!   before the extension.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Allowed values of `sftp.parallel_transfers`.
pub const PARALLEL_RANGE: std::ops::RangeInclusive<usize> = 1..=10;
/// Default of `sftp.parallel_transfers`.
pub const DEFAULT_PARALLEL: usize = 3;
/// The window the speed is measured over.
pub const SPEED_WINDOW: Duration = Duration::from_secs(5);

/// Where the bytes go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
    /// Between two servers (remote ↔ remote panels).
    RemoteToRemote,
}

/// What to do when the target name exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Replace,
    Skip,
    Rename,
}

/// Setting `sftp.name_conflict`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    Ask,
    Always(ConflictChoice),
}

impl ConflictPolicy {
    /// The policy of a setting value (`ask`, `replace`, `skip`, `rename`);
    /// anything else asks.
    pub fn from_setting(value: &str) -> Self {
        match value {
            "replace" => Self::Always(ConflictChoice::Replace),
            "skip" => Self::Always(ConflictChoice::Skip),
            "rename" => Self::Always(ConflictChoice::Rename),
            _ => Self::Ask,
        }
    }
}

/// Where a transfer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Running,
    /// The target exists and the user is asked.
    Conflict,
    Paused,
    /// Stopped by an error (a dropped connection, no permission): Retry.
    Failed(String),
    Done,
    /// Skipped because of a name conflict.
    Skipped,
    Cancelled,
}

impl State {
    /// Whether the transfer still has work to do.
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            State::Queued | State::Running | State::Conflict | State::Paused | State::Failed(_)
        )
    }
}

/// The id of a transfer in its queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferId(u64);

/// One transfer.
#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: TransferId,
    pub name: String,
    pub direction: Direction,
    /// Bytes in all; `None` until known.
    pub total: Option<u64>,
    /// Bytes done (a resumed transfer starts at its `.part` length).
    pub done: u64,
    pub state: State,
    /// (when, bytes done) of the recent progress reports.
    samples: VecDeque<(Instant, u64)>,
}

impl Transfer {
    /// Bytes per second over the last [`SPEED_WINDOW`]; `None` while
    /// there is too little to tell, or when it does not run.
    pub fn speed(&self) -> Option<f64> {
        if self.state != State::Running {
            return None;
        }
        let (first_at, first_done) = *self.samples.front()?;
        let (last_at, last_done) = *self.samples.back()?;
        let seconds = last_at.saturating_duration_since(first_at).as_secs_f64();
        (seconds > 0.0).then(|| last_done.saturating_sub(first_done) as f64 / seconds)
    }

    /// Time left at the current speed.
    pub fn time_left(&self) -> Option<Duration> {
        let speed = self.speed().filter(|speed| *speed > 0.0)?;
        let left = self.total?.saturating_sub(self.done);
        Some(Duration::from_secs_f64(left as f64 / speed))
    }

    /// Done, from 0 to 1, when the size is known.
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total?;
        Some(if total == 0 {
            1.0
        } else {
            (self.done as f64 / total as f64).min(1.0)
        })
    }
}

/// What the runner does after reporting a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictOutcome {
    /// Go on with this choice.
    Decided(ConflictChoice),
    /// Wait: the user is asked ([`TransferQueue::answer_conflict`]).
    Ask,
}

/// The queue of one SFTP tab.
#[derive(Debug)]
pub struct TransferQueue {
    transfers: Vec<Transfer>,
    next_id: u64,
    parallel: usize,
    policy: ConflictPolicy,
    /// An "apply to all" answer, for the later conflicts.
    for_all: Option<ConflictChoice>,
}

impl TransferQueue {
    pub fn new(parallel: usize, policy: ConflictPolicy) -> Self {
        Self {
            transfers: Vec::new(),
            next_id: 1,
            parallel: clamp_parallel(parallel),
            policy,
            for_all: None,
        }
    }

    pub fn parallel(&self) -> usize {
        self.parallel
    }

    /// A new limit applies to the next starts; running transfers go on.
    pub fn set_parallel(&mut self, parallel: usize) {
        self.parallel = clamp_parallel(parallel);
    }

    pub fn transfers(&self) -> &[Transfer] {
        &self.transfers
    }

    pub fn get(&self, id: TransferId) -> Option<&Transfer> {
        self.transfers.iter().find(|transfer| transfer.id == id)
    }

    fn get_mut(&mut self, id: TransferId) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|transfer| transfer.id == id)
    }

    /// Queues a transfer.
    pub fn add(&mut self, name: &str, direction: Direction, total: Option<u64>) -> TransferId {
        let id = TransferId(self.next_id);
        self.next_id += 1;
        self.transfers.push(Transfer {
            id,
            name: name.to_owned(),
            direction,
            total,
            done: 0,
            state: State::Queued,
            samples: VecDeque::new(),
        });
        id
    }

    fn running(&self) -> usize {
        self.transfers
            .iter()
            .filter(|transfer| transfer.state == State::Running)
            .count()
    }

    /// The queued transfers that start now, oldest first, marked running.
    pub fn start_next(&mut self) -> Vec<TransferId> {
        let mut room = self.parallel.saturating_sub(self.running());
        let mut started = Vec::new();
        for transfer in &mut self.transfers {
            if room == 0 {
                break;
            }
            if transfer.state == State::Queued {
                transfer.state = State::Running;
                transfer.samples.clear();
                started.push(transfer.id);
                room -= 1;
            }
        }
        started
    }

    /// A progress report of a running transfer (`done` bytes at `now`).
    pub fn progress(&mut self, id: TransferId, done: u64, now: Instant) {
        let Some(transfer) = self.get_mut(id) else {
            return;
        };
        if transfer.state != State::Running {
            return;
        }
        transfer.done = done;
        transfer.samples.push_back((now, done));
        while let Some((at, _)) = transfer.samples.front() {
            if transfer.samples.len() > 2 && now.saturating_duration_since(*at) > SPEED_WINDOW {
                transfer.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// The target of a running transfer exists: the policy, an "apply to
    /// all" answer, or a question.
    pub fn conflict(&mut self, id: TransferId) -> ConflictOutcome {
        let decided = match (self.for_all, self.policy) {
            (Some(choice), _) | (None, ConflictPolicy::Always(choice)) => Some(choice),
            (None, ConflictPolicy::Ask) => None,
        };
        let Some(transfer) = self.get_mut(id) else {
            return ConflictOutcome::Ask;
        };
        match decided {
            Some(ConflictChoice::Skip) => {
                transfer.state = State::Skipped;
                ConflictOutcome::Decided(ConflictChoice::Skip)
            }
            Some(choice) => ConflictOutcome::Decided(choice),
            None => {
                transfer.state = State::Conflict;
                ConflictOutcome::Ask
            }
        }
    }

    /// A new operation starts: an "apply to all" answer of an earlier one
    /// no longer applies (UI 5.7: "the remaining conflicts of the same
    /// operation").
    pub fn forget_for_all(&mut self) {
        self.for_all = None;
    }

    /// The user's answer to a conflict; with `for_all`, it also answers the
    /// later conflicts of this queue. A skipped transfer ends; the others
    /// queue again (the runner starts them with the answer).
    pub fn answer_conflict(&mut self, id: TransferId, choice: ConflictChoice, for_all: bool) {
        if for_all {
            self.for_all = Some(choice);
        }
        let Some(transfer) = self.get_mut(id) else {
            return;
        };
        if transfer.state != State::Conflict {
            return;
        }
        transfer.state = match choice {
            ConflictChoice::Skip => State::Skipped,
            ConflictChoice::Replace | ConflictChoice::Rename => State::Queued,
        };
    }

    /// Waiting transfers that an "apply to all" answer now decides: each
    /// with the choice (skipped ones end here).
    pub fn decided_by_all(&mut self) -> Vec<(TransferId, ConflictChoice)> {
        let Some(choice) = self.for_all else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for transfer in &mut self.transfers {
            if transfer.state == State::Conflict {
                transfer.state = match choice {
                    ConflictChoice::Skip => State::Skipped,
                    _ => State::Queued,
                };
                out.push((transfer.id, choice));
            }
        }
        out
    }

    pub fn finish(&mut self, id: TransferId) {
        if let Some(transfer) = self.get_mut(id) {
            if let Some(total) = transfer.total {
                transfer.done = total;
            }
            transfer.state = State::Done;
        }
    }

    /// Stopped by an error (shown with Retry).
    pub fn fail(&mut self, id: TransferId, reason: &str) {
        if let Some(transfer) = self.get_mut(id)
            && transfer.state.is_open()
        {
            transfer.state = State::Failed(reason.to_owned());
        }
    }

    /// Pause: a queued or running transfer stops where it is.
    pub fn pause(&mut self, id: TransferId) {
        if let Some(transfer) = self.get_mut(id)
            && matches!(transfer.state, State::Queued | State::Running)
        {
            transfer.state = State::Paused;
        }
    }

    /// Continue a paused transfer, or Retry a failed one: it queues again
    /// and resumes from what it has done.
    pub fn resume(&mut self, id: TransferId) {
        if let Some(transfer) = self.get_mut(id)
            && matches!(transfer.state, State::Paused | State::Failed(_))
        {
            transfer.state = State::Queued;
        }
    }

    pub fn cancel(&mut self, id: TransferId) {
        if let Some(transfer) = self.get_mut(id)
            && transfer.state.is_open()
        {
            transfer.state = State::Cancelled;
        }
    }

    /// Removes the transfers that ended (Clear finished).
    pub fn clear_finished(&mut self) {
        self.transfers.retain(|transfer| transfer.state.is_open());
    }
}

fn clamp_parallel(parallel: usize) -> usize {
    parallel.clamp(*PARALLEL_RANGE.start(), *PARALLEL_RANGE.end())
}

/// The name of a "rename" answer: `name (2).ext`, `name (3).ext`, … (the
/// extension is after the last dot; a leading dot, as in `.bashrc`, is part
/// of the name), the first one `taken` says is free.
pub fn renamed(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, extension) = match name.rfind('.') {
        Some(at) if at > 0 => name.split_at(at),
        _ => (name, ""),
    };
    (2u32..)
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(parallel: usize) -> TransferQueue {
        TransferQueue::new(parallel, ConflictPolicy::Ask)
    }

    #[test]
    fn at_most_the_limit_runs_and_the_oldest_starts_first() {
        let mut q = queue(DEFAULT_PARALLEL);
        let ids: Vec<_> = (0..5)
            .map(|n| q.add(&format!("f{n}"), Direction::Download, Some(10)))
            .collect();
        assert_eq!(q.start_next(), ids[..3]);
        assert!(q.start_next().is_empty(), "no room");
        q.finish(ids[1]);
        assert_eq!(q.start_next(), [ids[3]]);
        // A smaller limit lets the running ones finish.
        q.set_parallel(1);
        assert!(q.start_next().is_empty());
        q.finish(ids[0]);
        q.finish(ids[2]);
        q.finish(ids[3]);
        assert_eq!(q.start_next(), [ids[4]]);
        assert_eq!(TransferQueue::new(0, ConflictPolicy::Ask).parallel(), 1);
        assert_eq!(TransferQueue::new(99, ConflictPolicy::Ask).parallel(), 10);
    }

    #[test]
    fn pause_failure_retry_and_cancel() {
        let mut q = queue(2);
        let a = q.add("a", Direction::Upload, Some(100));
        let b = q.add("b", Direction::Upload, Some(100));
        q.start_next();
        let now = Instant::now();
        q.progress(a, 40, now);
        q.fail(a, "the connection was lost");
        assert_eq!(
            q.get(a).unwrap().state,
            State::Failed("the connection was lost".to_owned())
        );
        // Retry queues it again; it goes on from 40 bytes.
        q.resume(a);
        assert_eq!(q.get(a).unwrap().state, State::Queued);
        assert_eq!(q.get(a).unwrap().done, 40);
        q.pause(b);
        assert_eq!(q.start_next(), [a], "a paused transfer does not start");
        q.resume(b);
        assert_eq!(q.start_next(), [b]);
        q.cancel(b);
        q.finish(a);
        assert_eq!(q.get(a).unwrap().done, 100);
        // Ended transfers are not changed any more.
        q.fail(a, "late");
        assert_eq!(q.get(a).unwrap().state, State::Done);
        q.clear_finished();
        assert!(q.transfers().is_empty());
    }

    #[test]
    fn conflicts_follow_the_policy_and_apply_to_all() {
        let mut q = TransferQueue::new(3, ConflictPolicy::from_setting("skip"));
        let a = q.add("a", Direction::Download, None);
        q.start_next();
        assert_eq!(
            q.conflict(a),
            ConflictOutcome::Decided(ConflictChoice::Skip)
        );
        assert_eq!(q.get(a).unwrap().state, State::Skipped);

        let mut q = queue(3);
        let ids: Vec<_> = (0..3)
            .map(|n| q.add(&format!("f{n}"), Direction::Upload, None))
            .collect();
        q.start_next();
        assert_eq!(q.conflict(ids[0]), ConflictOutcome::Ask);
        assert_eq!(q.conflict(ids[1]), ConflictOutcome::Ask);
        assert_eq!(q.get(ids[0]).unwrap().state, State::Conflict);
        // A waiting transfer holds no running slot.
        let d = q.add("d", Direction::Upload, None);
        assert_eq!(q.start_next(), [d]);
        // "Rename, apply to all": the other waiting one and later ones too.
        q.answer_conflict(ids[0], ConflictChoice::Rename, true);
        assert_eq!(q.get(ids[0]).unwrap().state, State::Queued);
        assert_eq!(
            q.decided_by_all(),
            [(ids[1], ConflictChoice::Rename)],
            "the other waiting transfer"
        );
        assert_eq!(
            q.conflict(ids[2]),
            ConflictOutcome::Decided(ConflictChoice::Rename)
        );
    }

    #[test]
    fn speed_and_time_left_come_from_recent_progress() {
        let mut q = queue(1);
        let id = q.add("big", Direction::Download, Some(10_000_000));
        q.start_next();
        let start = Instant::now();
        assert_eq!(q.get(id).unwrap().speed(), None);
        for second in 0..=10u64 {
            q.progress(id, second * 100_000, start + Duration::from_secs(second));
        }
        let transfer = q.get(id).unwrap();
        let speed = transfer.speed().unwrap();
        assert!((speed - 100_000.0).abs() < 1.0, "{speed}");
        // 9 MB left at 100 kB/s.
        assert_eq!(transfer.time_left().unwrap().as_secs(), 90);
        assert_eq!(transfer.fraction(), Some(0.1));
        // Only the last few seconds count: a faster stretch shows at once.
        for second in 11..=16u64 {
            q.progress(
                id,
                1_000_000 + (second - 10) * 1_000_000,
                start + Duration::from_secs(second),
            );
        }
        assert!(q.get(id).unwrap().speed().unwrap() > 900_000.0);
        q.pause(id);
        assert_eq!(q.get(id).unwrap().speed(), None);
    }

    #[test]
    fn renamed_names_keep_their_extension() {
        let taken = |names: &'static [&'static str]| move |name: &str| names.contains(&name);
        assert_eq!(renamed("report.pdf", taken(&[])), "report (2).pdf");
        assert_eq!(
            renamed("report.pdf", taken(&["report (2).pdf"])),
            "report (3).pdf"
        );
        assert_eq!(renamed("archive.tar.gz", taken(&[])), "archive.tar (2).gz");
        assert_eq!(renamed(".bashrc", taken(&[])), ".bashrc (2)");
        assert_eq!(renamed("Makefile", taken(&[])), "Makefile (2)");
        assert_eq!(renamed("отчёт.txt", taken(&[])), "отчёт (2).txt");
    }

    #[test]
    fn apply_to_all_ends_with_its_operation() {
        let mut queue = TransferQueue::new(3, ConflictPolicy::Ask);
        let first = queue.add("a.txt", Direction::Upload, Some(1));
        queue.start_next();
        assert_eq!(queue.conflict(first), ConflictOutcome::Ask);
        queue.answer_conflict(first, ConflictChoice::Replace, true);
        // The next operation asks again.
        queue.forget_for_all();
        let second = queue.add("a.txt", Direction::Upload, Some(1));
        queue.start_next();
        assert_eq!(queue.conflict(second), ConflictOutcome::Ask);
    }
}
