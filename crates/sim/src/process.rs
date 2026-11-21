//! The game's cooperative process scheduler.
//!
//! Every system of the game (drawing, fonts, the field, menus, events...) runs as a *process*: a function
//! called once per tick. Processes live in three lists, each ordered by priority (lower runs first):
//! *starting* (waiting out a start delay), *running*, and *ending* (still called while a shutdown delay
//! counts down). A process function may return a new function to call from the next tick on, which is how
//! the game writes its state machines.
//!
//! The list operations follow the game's exactly, including the order in which a process that ends or
//! kills other processes during a tick affects who else runs in that tick.

use std::any::Any;

/// Handle to a process. Handles stay valid after the process ends; queries then report it as not running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(u32);

/// A process function. Returning `Some(f)` makes `f` the function called from the next tick on.
pub struct ProcessFn<C>(pub fn(&mut C, &mut Scheduler<C>, ProcessId) -> Option<ProcessFn<C>>);

impl<C> Clone for ProcessFn<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for ProcessFn<C> {}

impl<C> std::fmt::Debug for ProcessFn<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProcessFn({:p})", self.0 as *const ())
    }
}

/// What the scheduler needs from the game while it runs processes.
pub trait ProcessHost {
    /// Called before each process runs: `true` if the process may see the player's input, `false` if
    /// it must see a neutral pad.
    fn process_input(&mut self, enabled: bool);
}

/// List state, stored in the low four bits of the flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    None = 0,
    Starting = 1,
    Running = 2,
    Ending = 3,
}

/// The process sees a neutral pad instead of the player's input.
pub const NO_INPUT: u32 = 0x10;
/// The process is skipped (not called, run time not counted).
pub const SUSPENDED: u32 = 0x20;
/// Flag bits a caller may set or clear (the state bits are protected).
pub const USER_FLAGS: u32 = 0x0FFF_FFF0;

/// The answer a process function writes to ask for its own end.
pub const ANSWER_END: i32 = -1;

const STATE_MASK: u32 = 0xF;
const NAME_LEN: usize = 24;

/// Which processes a flag change applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagScope {
    /// Only the target.
    Target = 0,
    /// Every process except the target.
    Others = 1,
    /// The target and all its descendants.
    Tree = 2,
    /// Every process.
    All = 3,
}

/// A flag change over [`FlagScope::Others`] or [`FlagScope::All`] stops part way, as in the original:
/// only the first process of the first non-empty list is changed, and the calling code is interrupted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interrupted;

struct Process<C> {
    name: [u8; NAME_LEN],
    name_hash: u32,
    flags: u32,
    priority: u32,
    start_time: u32,
    run_time: u32,
    id: i32,
    start_delay: i32,
    shutdown_delay: i32,
    func: Option<ProcessFn<C>>,
    shutdown: Option<ProcessFn<C>>,
    answer: i32,
    data: Option<Box<dyn Any>>,
    next: Option<ProcessId>,
    prev: Option<ProcessId>,
    parent: Option<ProcessId>,
    child: Option<ProcessId>,
    sibling: Option<ProcessId>,
}

impl<C> Process<C> {
    fn state(&self) -> u32 {
        self.flags & STATE_MASK
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct List {
    first: Option<ProcessId>,
    last: Option<ProcessId>,
    count: i32,
}

/// The scheduler. `C` is the game context passed to every process function.
pub struct Scheduler<C> {
    procs: Vec<Process<C>>,
    /// Starting, running, ending.
    lists: [List; 3],
    current: Option<ProcessId>,
    next_id: i32,
}

impl<C> std::fmt::Debug for Scheduler<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("starting", &self.lists[0].count)
            .field("running", &self.lists[1].count)
            .field("ending", &self.lists[2].count)
            .finish()
    }
}

impl<C> Default for Scheduler<C> {
    fn default() -> Self {
        Self::new()
    }
}

fn list_index(state: u32) -> Option<usize> {
    match state {
        1 => Some(0),
        2 => Some(1),
        3 => Some(2),
        _ => None,
    }
}

/// A process name as the game stores it: ASCII, zero-padded to 24 bytes, the last byte always zero.
/// Returns the buffer and the sum of the copied bytes, or `None` if the name is longer than 24.
fn encode_name(name: &str) -> Option<([u8; NAME_LEN], u32)> {
    let mut buf = [0u8; NAME_LEN];
    let mut hash = 0u32;
    for (len, ch) in name.chars().enumerate() {
        if len >= NAME_LEN {
            return None;
        }
        let b = if ch.is_ascii() { ch as u8 } else { b'?' };
        buf[len] = b;
        hash = hash.wrapping_add(b as u32);
    }
    buf[NAME_LEN - 1] = 0;
    Some((buf, hash))
}

impl<C> Scheduler<C> {
    pub fn new() -> Self {
        Self { procs: Vec::new(), lists: [List::default(); 3], current: None, next_id: 1 }
    }

    fn p(&self, id: ProcessId) -> &Process<C> {
        &self.procs[id.0 as usize]
    }

    fn pm(&mut self, id: ProcessId) -> &mut Process<C> {
        &mut self.procs[id.0 as usize]
    }

    // ---- lists -------------------------------------------------------------------------------------

    fn list_remove(&mut self, id: ProcessId) {
        let Some(li) = list_index(self.p(id).state()) else { return };
        let (prev, next) = (self.p(id).prev, self.p(id).next);
        match prev {
            None => self.lists[li].first = next,
            Some(pv) => self.pm(pv).next = next,
        }
        match next {
            None => self.lists[li].last = prev,
            Some(nx) => self.pm(nx).prev = prev,
        }
        let p = self.pm(id);
        p.next = None;
        p.prev = None;
        self.lists[li].count -= 1;
    }

    fn list_insert(&mut self, id: ProcessId) {
        let Some(li) = list_index(self.p(id).state()) else { return };
        let prio = self.p(id).priority;
        let mut cur = self.lists[li].first;
        if cur.is_none() {
            self.lists[li].first = Some(id);
            self.lists[li].last = Some(id);
            let p = self.pm(id);
            p.prev = None;
            p.next = None;
        } else {
            let mut placed = false;
            while let Some(c) = cur {
                if prio < self.p(c).priority {
                    let before = self.p(c).prev;
                    match before {
                        None => self.lists[li].first = Some(id),
                        Some(b) => self.pm(b).next = Some(id),
                    }
                    let p = self.pm(id);
                    p.prev = before;
                    p.next = Some(c);
                    self.pm(c).prev = Some(id);
                    placed = true;
                    break;
                }
                cur = self.p(c).next;
            }
            if !placed {
                let last = self.lists[li].last;
                if let Some(l) = last {
                    self.pm(l).next = Some(id);
                }
                let p = self.pm(id);
                p.prev = last;
                p.next = None;
                self.lists[li].last = Some(id);
            }
        }
        self.lists[li].count += 1;
    }

    fn set_state(&mut self, id: ProcessId, state: State) {
        let p = self.pm(id);
        p.flags = (p.flags & !STATE_MASK) | state as u32;
    }

    // ---- lifecycle ---------------------------------------------------------------------------------

    /// Create a process. With `start_delay == 0` it runs from the current tick's running pass on (if
    /// that pass has not passed its position yet); otherwise it waits `start_delay` ticks. A negative delay
    /// never starts. `shutdown_delay` is how many ticks it keeps running after being ended.
    ///
    /// Returns `None` if the name is longer than 24 characters.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        name: &str,
        priority: u32,
        start_delay: i32,
        shutdown_delay: i32,
        func: Option<ProcessFn<C>>,
        shutdown: Option<ProcessFn<C>>,
        data: Option<Box<dyn Any>>,
    ) -> Option<ProcessId> {
        let (name, name_hash) = encode_name(name)?;
        let id = ProcessId(self.procs.len() as u32);
        let unique = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.procs.push(Process {
            name,
            name_hash,
            flags: State::Starting as u32,
            priority,
            start_time: 0,
            run_time: 0,
            id: unique,
            start_delay,
            shutdown_delay,
            func,
            shutdown,
            answer: 0,
            data,
            next: None,
            prev: None,
            parent: None,
            child: None,
            sibling: None,
        });
        self.list_insert(id);
        if start_delay == 0 {
            self.begin_running(id);
        }
        Some(id)
    }

    fn begin_running(&mut self, id: ProcessId) {
        self.list_remove(id);
        self.set_state(id, State::Running);
        self.list_insert(id);
        let p = self.pm(id);
        p.start_time = 0;
        p.run_time = 0;
    }

    fn end_process(&mut self, ctx: &mut C, id: ProcessId) {
        let s = self.p(id).state();
        if s != State::Starting as u32 && s != State::Running as u32 {
            return;
        }
        self.list_remove(id);
        self.set_state(id, State::Ending);
        self.list_insert(id);
        if self.p(id).shutdown_delay != 0 {
            return;
        }
        self.teardown(ctx, id);
    }

    /// Remove a process now: end its children, call its shutdown function, detach it from its parent.
    fn teardown(&mut self, ctx: &mut C, id: ProcessId) {
        let mut child = self.p(id).child;
        while let Some(c) = child {
            let next = self.p(c).sibling;
            self.kill(ctx, c, 0);
            child = next;
        }
        self.list_remove(id);
        if let Some(f) = self.p(id).shutdown {
            (f.0)(ctx, self, id);
        }
        self.pm(id).flags &= !STATE_MASK;
        self.unlink(id);
    }

    /// End a process. It keeps being called for `shutdown_delay` more ticks, then its shutdown function
    /// runs; with 0 that happens right away. Returns `false` if the process is not alive or has already
    /// asked to end itself this tick.
    pub fn kill(&mut self, ctx: &mut C, id: ProcessId, shutdown_delay: i32) -> bool {
        let p = self.p(id);
        if p.answer == ANSWER_END {
            return false;
        }
        match p.state() {
            1 | 2 => {
                self.pm(id).shutdown_delay = shutdown_delay;
                self.end_process(ctx, id);
                true
            }
            3 => {
                self.pm(id).shutdown_delay = shutdown_delay;
                if shutdown_delay == 0 {
                    self.teardown(ctx, id);
                }
                true
            }
            _ => false,
        }
    }

    /// [`Scheduler::kill`] the first process with this name.
    pub fn kill_by_name(&mut self, ctx: &mut C, name: &str, shutdown_delay: i32) -> bool {
        match self.find_by_name(name) {
            Some(id) => self.kill(ctx, id, shutdown_delay),
            None => false,
        }
    }

    // ---- running -----------------------------------------------------------------------------------

    /// Run one process. Returns `false` if it ended itself during the call.
    fn run_one(&mut self, ctx: &mut C, id: ProcessId) -> bool
    where
        C: ProcessHost,
    {
        if self.p(id).flags & SUSPENDED != 0 {
            return true;
        }
        self.current = Some(id);
        ctx.process_input(self.p(id).flags & NO_INPUT == 0);
        if let Some(f) = self.p(id).func
            && self.p(id).answer != ANSWER_END
        {
            self.pm(id).answer = 0;
            if let Some(next) = (f.0)(ctx, self, id) {
                self.pm(id).func = Some(next);
            }
            if self.p(id).answer == ANSWER_END && self.p(id).state() == State::Running as u32 {
                self.end_process(ctx, id);
                self.current = None;
                return false;
            }
            self.pm(id).answer = 0;
        }
        let p = self.pm(id);
        p.run_time = p.run_time.wrapping_add(1);
        self.current = None;
        true
    }

    /// Continue a list walk after running `node` (whose previous node was `prev` before the run).
    fn after_run(
        &self,
        li: usize,
        node: ProcessId,
        prev: Option<ProcessId>,
        kept: bool,
    ) -> Option<ProcessId> {
        if kept {
            self.p(node).next
        } else {
            match prev {
                Some(pv) => self.p(pv).next,
                None => self.lists[li].first,
            }
        }
    }

    fn run_lists(&mut self, ctx: &mut C)
    where
        C: ProcessHost,
    {
        let mut r = self.lists[1].first;
        let mut e = self.lists[2].first;
        while let Some(rn) = r {
            let Some(en) = e else { break };
            if self.p(en).priority < self.p(rn).priority {
                let prev = self.p(en).prev;
                let kept = self.run_one(ctx, en);
                e = self.after_run(2, en, prev, kept);
            } else {
                let prev = self.p(rn).prev;
                let kept = self.run_one(ctx, rn);
                r = self.after_run(1, rn, prev, kept);
            }
        }
        if e.is_none() {
            while let Some(rn) = r {
                let prev = self.p(rn).prev;
                let kept = self.run_one(ctx, rn);
                r = self.after_run(1, rn, prev, kept);
            }
            return;
        }
        while let Some(en) = e {
            let prev = self.p(en).prev;
            let kept = self.run_one(ctx, en);
            e = self.after_run(2, en, prev, kept);
        }
    }

    /// One tick: count down start delays, run the running and ending processes in priority order, then
    /// count down shutdown delays.
    pub fn tick(&mut self, ctx: &mut C)
    where
        C: ProcessHost,
    {
        let mut cur = self.lists[0].first;
        while let Some(id) = cur {
            let mut t = self.p(id).start_delay;
            if t > 0 {
                t -= 1;
                self.pm(id).start_delay = t;
            }
            cur = self.p(id).next;
            if t == 0 {
                self.begin_running(id);
            }
        }
        self.run_lists(ctx);
        let mut cur = self.lists[2].first;
        while let Some(id) = cur {
            let mut t = self.p(id).shutdown_delay;
            if t > 0 {
                t -= 1;
                self.pm(id).shutdown_delay = t;
            }
            cur = self.p(id).next;
            if t == 0 {
                self.teardown(ctx, id);
            }
        }
    }

    // ---- per-process data --------------------------------------------------------------------------

    /// The process currently being run, if any.
    pub fn current(&self) -> Option<ProcessId> {
        self.current
    }

    /// Ask for the calling process's end; takes effect when its function returns.
    pub fn request_end(&mut self, id: ProcessId) {
        self.pm(id).answer = ANSWER_END;
    }

    pub fn data(&self, id: ProcessId) -> Option<&dyn Any> {
        self.p(id).data.as_deref()
    }

    pub fn data_mut(&mut self, id: ProcessId) -> Option<&mut (dyn Any + 'static)> {
        self.pm(id).data.as_deref_mut()
    }

    pub fn set_data(&mut self, id: ProcessId, data: Option<Box<dyn Any>>) {
        self.pm(id).data = data;
    }

    pub fn run_time(&self, id: ProcessId) -> u32 {
        self.p(id).run_time
    }

    pub fn flags(&self, id: ProcessId) -> u32 {
        self.p(id).flags
    }

    pub fn priority(&self, id: ProcessId) -> u32 {
        self.p(id).priority
    }

    /// The process's unique number (for code that refers to processes by number).
    pub fn unique_id(&self, id: ProcessId) -> i32 {
        self.p(id).id
    }

    pub fn name(&self, id: ProcessId) -> &str {
        let n = &self.p(id).name;
        let len = n.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        std::str::from_utf8(&n[..len]).unwrap_or("")
    }

    // ---- queries -----------------------------------------------------------------------------------

    fn walk_all(&self) -> impl Iterator<Item = ProcessId> + '_ {
        (0..3).flat_map(move |li| std::iter::successors(self.lists[li].first, move |&id| self.p(id).next))
    }

    /// Number of processes in a state.
    pub fn count(&self, state: State) -> i32 {
        list_index(state as u32).map_or(0, |li| self.lists[li].count)
    }

    /// The `n`-th process (0-based, priority order) in a state's list.
    pub fn nth(&self, state: State, n: i32) -> Option<ProcessId> {
        let li = list_index(state as u32)?;
        let mut cur = self.lists[li].first;
        for _ in 0..n.max(0) {
            cur = self.p(cur?).next;
        }
        cur
    }

    /// Whether the process is in one of the lists.
    pub fn exists(&self, id: ProcessId) -> bool {
        self.walk_all().any(|p| p == id)
    }

    /// The process's state, or [`State::None`] if it is in no list.
    pub fn state(&self, id: ProcessId) -> State {
        if !self.exists(id) {
            return State::None;
        }
        match self.p(id).state() {
            1 => State::Starting,
            2 => State::Running,
            3 => State::Ending,
            _ => State::None,
        }
    }

    /// First process (starting, then running, then ending list) with this name.
    pub fn find_by_name(&self, name: &str) -> Option<ProcessId> {
        let mut buf = [0u8; NAME_LEN];
        for (i, ch) in name.chars().take(NAME_LEN).enumerate() {
            buf[i] = if ch.is_ascii() { ch as u8 } else { b'?' };
        }
        let end = buf.iter().position(|&b| b == 0)?;
        let hash: u32 = buf[..end].iter().map(|&b| b as u32).sum();
        self.walk_all().find(|&id| {
            let p = self.p(id);
            p.name_hash == hash && (0..=end).rev().all(|i| buf[i] == p.name[i])
        })
    }

    /// First process with this priority. Negative priorities never match.
    pub fn find_by_priority(&self, priority: i32) -> Option<ProcessId> {
        self.walk_all().find(|&id| priority >= 0 && self.p(id).priority as i64 == priority as i64)
    }

    /// First process with this unique number.
    pub fn find_by_unique_id(&self, unique: i32) -> Option<ProcessId> {
        self.walk_all().find(|&id| self.p(id).id == unique)
    }

    // ---- tree --------------------------------------------------------------------------------------

    fn detach(&mut self, target: ProcessId) {
        let Some(parent) = self.p(target).parent else { return };
        let first = self.p(parent).child;
        if first == Some(target) {
            self.pm(parent).child = self.p(target).sibling;
        } else {
            let mut cur = first;
            while let Some(c) = cur {
                let next = self.p(c).sibling;
                if next == Some(target) {
                    self.pm(c).sibling = self.p(target).sibling;
                    break;
                }
                cur = next;
            }
        }
        let t = self.pm(target);
        t.parent = None;
        t.sibling = None;
    }

    /// Make `target` the last child of `parent` (detaching it from any previous parent). Ending a parent
    /// ends its children first.
    pub fn link(&mut self, parent: Option<ProcessId>, target: ProcessId) {
        self.detach(target);
        let Some(parent) = parent else { return };
        match self.p(parent).child {
            None => self.pm(parent).child = Some(target),
            Some(mut c) => {
                while let Some(n) = self.p(c).sibling {
                    c = n;
                }
                self.pm(c).sibling = Some(target);
            }
        }
        self.pm(target).parent = Some(parent);
    }

    /// Detach a process from its parent.
    pub fn unlink(&mut self, target: ProcessId) {
        self.detach(target);
    }

    // ---- flags -------------------------------------------------------------------------------------

    /// Set (`on`) or clear user flag bits over a scope. See [`Interrupted`] for the two global scopes.
    pub fn change_flags(
        &mut self,
        target: ProcessId,
        flags: u32,
        on: bool,
        scope: FlagScope,
    ) -> Result<(), Interrupted> {
        let mask = flags & USER_FLAGS;
        let apply = |p: &mut Process<C>| {
            if on {
                p.flags |= mask;
            } else {
                p.flags &= !mask;
            }
        };
        match scope {
            FlagScope::Target => {
                apply(self.pm(target));
                Ok(())
            }
            FlagScope::Tree => {
                self.change_tree(target, mask, on);
                Ok(())
            }
            FlagScope::Others | FlagScope::All => {
                for li in 0..3 {
                    if let Some(first) = self.lists[li].first {
                        if scope == FlagScope::All || first != target {
                            apply(self.pm(first));
                        }
                        return Err(Interrupted);
                    }
                }
                Ok(())
            }
        }
    }

    fn change_tree(&mut self, id: ProcessId, mask: u32, on: bool) {
        let p = self.pm(id);
        if on {
            p.flags |= mask;
        } else {
            p.flags &= !mask;
        }
        let mut child = self.p(id).child;
        while let Some(c) = child {
            self.change_tree(c, mask, on);
            child = self.p(c).sibling;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Ctx {
        log: Vec<String>,
        input: Vec<bool>,
    }

    impl ProcessHost for Ctx {
        fn process_input(&mut self, enabled: bool) {
            self.input.push(enabled);
        }
    }

    fn log_name(ctx: &mut Ctx, s: &mut Scheduler<Ctx>, id: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.log.push(s.name(id).to_string());
        None
    }

    fn start(s: &mut Scheduler<Ctx>, name: &str, prio: u32) -> ProcessId {
        s.start(name, prio, 0, 0, Some(ProcessFn(log_name)), None, None).unwrap()
    }

    #[test]
    fn runs_in_priority_order_with_stable_ties() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        start(&mut s, "c", 30);
        start(&mut s, "a", 10);
        start(&mut s, "b1", 20);
        start(&mut s, "b2", 20);
        s.tick(&mut c);
        assert_eq!(c.log, ["a", "b1", "b2", "c"]);
    }

    #[test]
    fn start_delay_counts_ticks() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let id = s.start("late", 0, 2, 0, Some(ProcessFn(log_name)), None, None).unwrap();
        assert_eq!(s.state(id), State::Starting);
        s.tick(&mut c); // 2 -> 1
        assert!(c.log.is_empty());
        s.tick(&mut c); // 1 -> 0: starts and runs this tick
        assert_eq!(c.log, ["late"]);
        assert_eq!(s.state(id), State::Running);
    }

    #[test]
    fn negative_start_delay_never_starts() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let id = s.start("never", 0, -1, 0, Some(ProcessFn(log_name)), None, None).unwrap();
        for _ in 0..5 {
            s.tick(&mut c);
        }
        assert_eq!(s.state(id), State::Starting);
        assert!(c.log.is_empty());
    }

    fn step_a(ctx: &mut Ctx, _: &mut Scheduler<Ctx>, _: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.log.push("A".into());
        Some(ProcessFn(step_b))
    }

    fn step_b(ctx: &mut Ctx, _: &mut Scheduler<Ctx>, _: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.log.push("B".into());
        None
    }

    #[test]
    fn returned_function_replaces_the_current_one() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        s.start("sm", 0, 0, 0, Some(ProcessFn(step_a)), None, None);
        for _ in 0..3 {
            s.tick(&mut c);
        }
        assert_eq!(c.log, ["A", "B", "B"]);
    }

    fn end_self(ctx: &mut Ctx, s: &mut Scheduler<Ctx>, id: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.log.push(format!("run {}", s.name(id)));
        s.request_end(id);
        None
    }

    fn on_shutdown(ctx: &mut Ctx, s: &mut Scheduler<Ctx>, id: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.log.push(format!("shutdown {}", s.name(id)));
        None
    }

    #[test]
    fn ending_itself_with_and_without_delay() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let now =
            s.start("now", 0, 0, 0, Some(ProcessFn(end_self)), Some(ProcessFn(on_shutdown)), None).unwrap();
        s.tick(&mut c);
        assert_eq!(c.log, ["run now", "shutdown now"]);
        assert_eq!(s.state(now), State::None);

        c.log.clear();
        let later =
            s.start("later", 0, 0, 1, Some(ProcessFn(end_self)), Some(ProcessFn(on_shutdown)), None).unwrap();
        s.tick(&mut c);
        // Ends during the running pass; the 1-tick shutdown delay runs out in the same tick's ending pass.
        assert_eq!(c.log, ["run later", "shutdown later"]);
        assert_eq!(s.state(later), State::None);
    }

    #[test]
    fn ending_processes_keep_running_during_their_delay() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let id = start(&mut s, "slow", 5);
        start(&mut s, "other", 10);
        s.kill(&mut c, id, 3);
        assert_eq!(s.state(id), State::Ending);
        s.tick(&mut c);
        s.tick(&mut c);
        assert_eq!(c.log, ["slow", "other", "slow", "other"]);
        s.tick(&mut c);
        assert_eq!(s.state(id), State::None);
    }

    #[test]
    fn killing_a_parent_kills_its_children_first() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let parent = s.start("parent", 0, 0, 0, None, Some(ProcessFn(on_shutdown)), None).unwrap();
        let child = s.start("child", 1, 0, 0, None, Some(ProcessFn(on_shutdown)), None).unwrap();
        s.link(Some(parent), child);
        assert!(s.kill(&mut c, parent, 0));
        assert_eq!(c.log, ["shutdown child", "shutdown parent"]);
        assert!(!s.exists(child));
        assert!(!s.kill(&mut c, child, 0));
    }

    #[test]
    fn find_by_name_and_priority() {
        let mut s: Scheduler<Ctx> = Scheduler::new();
        let a = s.start("alpha", 7, 0, 0, None, None, None).unwrap();
        let b = s.start("beta", 9, 3, 0, None, None, None).unwrap();
        assert_eq!(s.find_by_name("alpha"), Some(a));
        assert_eq!(s.find_by_name("beta"), Some(b));
        assert_eq!(s.find_by_name("gamma"), None);
        assert_eq!(s.find_by_priority(9), Some(b));
        assert_eq!(s.find_by_priority(-1), None);
        assert!(s.start("a_name_that_is_much_too_long", 0, 0, 0, None, None, None).is_none());
    }

    #[test]
    fn no_input_flag_reaches_the_host() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let a = start(&mut s, "a", 0);
        start(&mut s, "b", 1);
        s.change_flags(a, NO_INPUT, true, FlagScope::Target).unwrap();
        s.tick(&mut c);
        assert_eq!(c.input, [false, true]);
    }

    #[test]
    fn suspended_processes_are_skipped() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        let a = start(&mut s, "a", 0);
        s.change_flags(a, SUSPENDED, true, FlagScope::Target).unwrap();
        s.tick(&mut c);
        assert!(c.log.is_empty());
        assert_eq!(s.run_time(a), 0);
    }

    #[test]
    fn state_bits_cannot_be_changed_by_flags() {
        let mut s: Scheduler<Ctx> = Scheduler::new();
        let a = s.start("a", 0, 0, 0, None, None, None).unwrap();
        s.change_flags(a, 0xF, false, FlagScope::Target).unwrap();
        assert_eq!(s.state(a), State::Running);
    }

    #[test]
    fn global_flag_change_only_reaches_the_first_process() {
        let mut s: Scheduler<Ctx> = Scheduler::new();
        let a = s.start("a", 0, 0, 0, None, None, None).unwrap();
        let b = s.start("b", 1, 0, 0, None, None, None).unwrap();
        assert_eq!(s.change_flags(b, NO_INPUT, true, FlagScope::Others), Err(Interrupted));
        assert_ne!(s.flags(a) & NO_INPUT, 0);
        assert_eq!(s.flags(b) & NO_INPUT, 0);
    }

    fn spawn_low(ctx: &mut Ctx, s: &mut Scheduler<Ctx>, id: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.log.push(s.name(id).to_string());
        if s.find_by_name("spawned").is_none() {
            s.start("spawned", 100, 0, 0, Some(ProcessFn(log_name)), None, None);
        }
        None
    }

    #[test]
    fn a_process_started_during_the_pass_runs_in_it_if_behind() {
        let mut s = Scheduler::new();
        let mut c = Ctx::default();
        s.start("spawner", 0, 0, 0, Some(ProcessFn(spawn_low)), None, None);
        s.tick(&mut c);
        assert_eq!(c.log, ["spawner", "spawned"]);
    }
}
