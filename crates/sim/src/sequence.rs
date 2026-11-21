//! Game sequences: the top-level modes (title, field, battle, event, menus...) and the switching between them.
//!
//! A single process, the *director*, runs the current sequence. Each sequence may provide three hooks:
//! `start` (entering it), `end` (polled when leaving it until it reports done), and `check` (polled every tick;
//! non-zero asks to leave). Code requests the next sequence with [`call`]; a sequence that leaves without a
//! request goes back to the previous one if it was entered with *restore*, otherwise to sequence 0.
//!
//! Switching takes a fixed number of ticks, as in the original: the director first polls the outgoing
//! sequence's `end`, then waits out a countdown before starting the next one.

use std::any::Any;
use std::rc::Rc;

use crate::process::{ProcessFn, ProcessHost, ProcessId, Scheduler};

/// Name of the director process.
pub const DIRECTOR_NAME: &str = "Admini";
/// Director priority (runs right after the highest-priority system processes).
pub const DIRECTOR_PRIORITY: u32 = 2;
/// Ticks the director keeps running after it is killed.
pub const DIRECTOR_SHUTDOWN_DELAY: i32 = 1;

/// Data handed to a sequence when it is called. Calls with data of 257 bytes or more are ignored.
pub type CallData = Rc<dyn Any>;

const HISTORY: i32 = 8;
const MAX_CALL_DATA_SIZE: u32 = 0x100;

/// A call is pending.
const CALL: u32 = 0x1;
/// The current sequence asked to leave.
const EXIT: u32 = 0x2;
/// Mark the outgoing sequence as one to come back to.
const MARK_RESTORE: u32 = 0x4;
/// Start the next sequence without advancing the history.
const KEEP_HISTORY: u32 = 0x8;
/// The pending call goes back to the previous sequence.
const GOING_BACK: u32 = 0x10000;

/// Entering a sequence. `back` is true when returning to a sequence marked for restore.
pub type StartHook<C> = fn(&mut C, &mut Scheduler<C>, back: bool, data: Option<CallData>) -> i32;

/// Hooks of one sequence.
pub struct SequenceHooks<C> {
    /// Entering.
    pub start: Option<StartHook<C>>,
    /// Leaving: called every tick until it returns a value `>= 0`, which then adds that many ticks of waiting.
    pub end: Option<fn(&mut C, &mut Scheduler<C>) -> i32>,
    /// Called every tick while running: non-zero asks to leave.
    pub check: Option<fn(&mut C, &mut Scheduler<C>) -> i32>,
}

impl<C> Clone for SequenceHooks<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for SequenceHooks<C> {}

impl<C> Default for SequenceHooks<C> {
    fn default() -> Self {
        Self { start: None, end: None, check: None }
    }
}

impl<C> std::fmt::Debug for SequenceHooks<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SequenceHooks")
            .field("start", &self.start.is_some())
            .field("end", &self.end.is_some())
            .field("check", &self.check.is_some())
            .finish()
    }
}

/// The game provides the hooks of each sequence.
pub trait SequenceHost: ProcessHost + Sized {
    /// Hooks for a sequence number, or `None` if there is no such sequence.
    fn sequence(&self, number: i32) -> Option<SequenceHooks<Self>>;
}

/// The director's state, kept as its process data.
#[derive(Debug, Clone)]
pub struct Director {
    status: u32,
    /// Ticks the current sequence has been running.
    pub run_time: u32,
    /// The running sequence, -1 if none.
    pub current: i32,
    /// The requested sequence, -1 if none.
    pub next: i32,
    history_pos: i32,
    history: [i32; HISTORY as usize],
    restore_marks: [u8; HISTORY as usize],
    call_data: Option<CallData>,
    call_data_size: i32,
    countdown: i32,
}

impl Default for Director {
    fn default() -> Self {
        Self {
            status: 0,
            run_time: 0,
            current: -1,
            next: -1,
            history_pos: 0,
            history: [-1; HISTORY as usize],
            restore_marks: [0; HISTORY as usize],
            call_data: None,
            call_data_size: 0,
            countdown: 0,
        }
    }
}

impl Director {
    fn call(&mut self, number: i32, data: Option<CallData>, size: i32, restore: bool) {
        if data.is_some() && size as u32 > MAX_CALL_DATA_SIZE {
            return;
        }
        self.status = (self.status & !(GOING_BACK | KEEP_HISTORY)) | CALL;
        self.next = number;
        self.countdown = 2;
        if data.is_some() {
            self.call_data = data;
            self.call_data_size = size;
        } else {
            self.call_data = None;
            self.call_data_size = 0;
        }
        if restore {
            self.status |= MARK_RESTORE;
        } else {
            self.status &= !MARK_RESTORE;
        }
    }

    fn prev_pos(&self) -> usize {
        ((self.history_pos + HISTORY - 1) % HISTORY) as usize
    }

    /// The sequences entered, oldest first in ring order, with -1 for empty slots.
    pub fn history(&self) -> [i32; HISTORY as usize] {
        self.history
    }
}

fn director<C>(s: &Scheduler<C>, id: ProcessId) -> &Director {
    s.data(id).and_then(|d| d.downcast_ref::<Director>()).expect("director process without its data")
}

fn director_mut<C>(s: &mut Scheduler<C>, id: ProcessId) -> &mut Director {
    s.data_mut(id).and_then(|d| d.downcast_mut::<Director>()).expect("director process without its data")
}

fn find_director<C>(s: &mut Scheduler<C>) -> Option<&mut Director> {
    let id = s.find_by_name(DIRECTOR_NAME)?;
    s.data_mut(id).and_then(|d| d.downcast_mut::<Director>())
}

/// Create the director process if it does not exist yet.
pub fn init<C: SequenceHost>(s: &mut Scheduler<C>) {
    if s.find_by_name(DIRECTOR_NAME).is_some() {
        return;
    }
    s.start(
        DIRECTOR_NAME,
        DIRECTOR_PRIORITY,
        0,
        DIRECTOR_SHUTDOWN_DELAY,
        Some(ProcessFn(run)),
        Some(ProcessFn(shutdown)),
        Some(Box::new(Director::default())),
    );
}

/// Request sequence `number`. `restore` marks the current sequence so that leaving `number` without a new
/// request comes back to it.
pub fn call<C>(s: &mut Scheduler<C>, number: i32, data: Option<CallData>, size: i32, restore: bool) {
    if let Some(d) = find_director(s) {
        d.call(number, data, size, restore);
    }
}

/// Ask the current sequence to leave.
pub fn exit<C>(s: &mut Scheduler<C>) {
    if let Some(d) = find_director(s) {
        d.status |= EXIT;
    }
}

/// The requested sequence, -1 if none (or no director).
pub fn next_sequence<C>(s: &mut Scheduler<C>) -> i32 {
    find_director(s).map_or(-1, |d| d.next)
}

/// Whether the previous history entry is marked for restore.
pub fn previous_restore_mark<C>(s: &mut Scheduler<C>) -> u8 {
    find_director(s).map_or(0, |d| d.restore_marks[d.prev_pos()])
}

/// The director's state, if it exists.
pub fn state<C>(s: &mut Scheduler<C>) -> Option<Director> {
    find_director(s).map(|d| d.clone())
}

fn hooks<C: SequenceHost>(ctx: &C, number: i32) -> SequenceHooks<C> {
    ctx.sequence(number).unwrap_or_default()
}

/// Running a sequence: poll its `check`, handle leaving.
fn run<C: SequenceHost>(ctx: &mut C, s: &mut Scheduler<C>, id: ProcessId) -> Option<ProcessFn<C>> {
    let d = director(s, id);
    if d.status & EXIT == 0 && d.current >= 0 {
        let current = d.current;
        if let Some(check) = hooks(ctx, current).check
            && check(ctx, s) != 0
        {
            director_mut(s, id).status |= EXIT;
        }
        let d = director_mut(s, id);
        d.run_time = d.run_time.wrapping_add(1);
    }
    let d = director_mut(s, id);
    if d.status & EXIT != 0 && d.current >= 0 {
        let p = d.prev_pos();
        if d.history[p] >= 0 && d.restore_marks[p] & 1 != 0 {
            d.status |= GOING_BACK | CALL;
            d.next = d.history[p];
            d.countdown = 2;
            d.status &= !MARK_RESTORE;
            d.call_data = None;
            d.call_data_size = 0;
        }
        if d.status & CALL == 0 {
            d.call(0, None, 0, false);
        }
        d.status &= !EXIT;
    }
    let d = director(s, id);
    if d.status & CALL != 0 && d.next >= 0 { Some(ProcessFn(end_check)) } else { None }
}

/// Leaving the current sequence: poll its `end` until done.
fn end_check<C: SequenceHost>(ctx: &mut C, s: &mut Scheduler<C>, id: ProcessId) -> Option<ProcessFn<C>> {
    let current = director(s, id).current;
    if current < 0 {
        return Some(ProcessFn(start_next));
    }
    let Some(end) = hooks(ctx, current).end else {
        return Some(ProcessFn(start_next));
    };
    let r = end(ctx, s);
    if r < 0 {
        return None;
    }
    let d = director_mut(s, id);
    d.current = -1;
    d.countdown = r + 1;
    Some(ProcessFn(start_next))
}

/// Entering the requested sequence after the countdown.
fn start_next<C: SequenceHost>(ctx: &mut C, s: &mut Scheduler<C>, id: ProcessId) -> Option<ProcessFn<C>> {
    let d = director_mut(s, id);
    let old = d.status;
    if old & CALL == 0 || d.next < 0 {
        return Some(ProcessFn(run));
    }
    if d.countdown > 0 {
        d.countdown -= 1;
        return None;
    }
    d.run_time = 0;
    d.current = d.next;
    d.next = -1;
    d.status = old & !(CALL | EXIT);
    if old & MARK_RESTORE != 0 {
        d.restore_marks[d.history_pos as usize] |= 1;
        d.status &= !MARK_RESTORE;
    }
    let back = if d.status & GOING_BACK == 0 {
        if d.status & KEEP_HISTORY != 0 {
            d.status &= !KEEP_HISTORY;
        } else {
            d.history_pos = (d.history_pos + 1) % HISTORY;
        }
        false
    } else {
        d.history_pos = (d.history_pos + HISTORY - 1) % HISTORY;
        d.status &= !GOING_BACK;
        true
    };
    let pos = d.history_pos as usize;
    d.history[pos] = d.current;
    d.restore_marks[pos] = 0;
    let (current, data) = (d.current, d.call_data.clone());
    if let Some(start) = hooks(ctx, current).start {
        start(ctx, s, back, data);
    }
    Some(ProcessFn(run))
}

/// The director is being removed: let the running sequence clean up once.
fn shutdown<C: SequenceHost>(ctx: &mut C, s: &mut Scheduler<C>, id: ProcessId) -> Option<ProcessFn<C>> {
    let current = director(s, id).current;
    if current >= 0
        && let Some(end) = hooks(ctx, current).end
    {
        end(ctx, s);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Ctx {
        log: Vec<String>,
        tick: u32,
        leave_title_at: Option<u32>,
        end_wait: i32,
    }

    impl ProcessHost for Ctx {
        fn process_input(&mut self, _: bool) {}
    }

    fn title_start(c: &mut Ctx, _: &mut Scheduler<Ctx>, back: bool, _: Option<CallData>) -> i32 {
        c.log.push(format!("{} start title back={back}", c.tick));
        0
    }
    fn title_check(c: &mut Ctx, _: &mut Scheduler<Ctx>) -> i32 {
        (c.leave_title_at == Some(c.tick)) as i32
    }
    fn title_end(c: &mut Ctx, _: &mut Scheduler<Ctx>) -> i32 {
        c.log.push(format!("{} end title", c.tick));
        if c.end_wait > 0 {
            c.end_wait -= 1;
            return -1;
        }
        0
    }
    fn zero_start(c: &mut Ctx, _: &mut Scheduler<Ctx>, back: bool, _: Option<CallData>) -> i32 {
        c.log.push(format!("{} start zero back={back}", c.tick));
        0
    }
    fn menu_start(c: &mut Ctx, _: &mut Scheduler<Ctx>, _: bool, data: Option<CallData>) -> i32 {
        let n = data.and_then(|d| d.downcast_ref::<u32>().copied());
        c.log.push(format!("{} start menu {n:?}", c.tick));
        0
    }
    fn menu_check(c: &mut Ctx, _: &mut Scheduler<Ctx>) -> i32 {
        (c.leave_title_at == Some(c.tick)) as i32
    }

    impl SequenceHost for Ctx {
        fn sequence(&self, n: i32) -> Option<SequenceHooks<Self>> {
            match n {
                0 => Some(SequenceHooks { start: Some(zero_start), end: None, check: None }),
                2 => Some(SequenceHooks {
                    start: Some(title_start),
                    end: Some(title_end),
                    check: Some(title_check),
                }),
                9 => Some(SequenceHooks { start: Some(menu_start), end: None, check: Some(menu_check) }),
                _ => None,
            }
        }
    }

    fn run_ticks(s: &mut Scheduler<Ctx>, c: &mut Ctx, n: u32) {
        for _ in 0..n {
            c.tick += 1;
            s.tick(c);
        }
    }

    #[test]
    fn first_call_starts_after_the_switch_delay() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        init(&mut s);
        call(&mut s, 2, None, 0, false);
        run_ticks(&mut s, &mut c, 6);
        // Tick 1: running → leaving check; 2: nothing to end → wait; 3,4: countdown 2→0; 5: start.
        assert_eq!(c.log, ["5 start title back=false"]);
        assert_eq!(state(&mut s).unwrap().current, 2);
    }

    #[test]
    fn leaving_without_a_call_goes_to_sequence_zero() {
        let mut s = Scheduler::new();
        let mut c = Ctx { leave_title_at: Some(8), end_wait: 1, ..Default::default() };
        init(&mut s);
        call(&mut s, 2, None, 0, false);
        run_ticks(&mut s, &mut c, 16);
        assert_eq!(
            c.log,
            [
                "5 start title back=false",
                "9 end title",              // end polled: not done yet
                "10 end title",             // done, waits 0 + 1 ticks
                "12 start zero back=false", // the end result (0) + 1 replaces the call's 2-tick wait
            ]
        );
    }

    #[test]
    fn a_restore_call_comes_back_to_the_caller() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        init(&mut s);
        call(&mut s, 2, None, 0, false);
        run_ticks(&mut s, &mut c, 5);
        call(&mut s, 9, Some(Rc::new(7u32)), 4, true);
        c.leave_title_at = Some(14);
        run_ticks(&mut s, &mut c, 15);
        let back = c.log.iter().filter(|l| l.contains("start title back=true")).count();
        assert_eq!(back, 1, "{:?}", c.log);
        assert!(c.log.iter().any(|l| l.contains("start menu Some(7)")), "{:?}", c.log);
        assert_eq!(previous_restore_mark(&mut s), 0);
    }

    #[test]
    fn oversized_call_data_is_ignored() {
        let mut s: Scheduler<Ctx> = Scheduler::new();
        init(&mut s);
        call(&mut s, 2, Some(Rc::new(0u8)), 0x101, false);
        assert_eq!(next_sequence(&mut s), -1);
        call(&mut s, 2, Some(Rc::new(0u8)), 0x100, false);
        assert_eq!(next_sequence(&mut s), 2);
    }
}
