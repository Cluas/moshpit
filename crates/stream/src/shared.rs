//! One pane's channel: the state both ends share.
//!
//! Locks: `output` and `input` guard the two queues; `waker` guards the
//! view's waker and is held while the waker runs, so detaching waits for a
//! wake already going out and none starts after it. The only nesting is
//! `waker` then `output` (attach, detach, and a waker that reads); a push
//! releases `output` before it takes `waker`.

use std::{
    collections::VecDeque,
    task::{Context, Poll},
};

use crate::{
    InputEvent, Mark, Stats,
    queue::ByteQueue,
    sync::{Mutex, lock},
};

/// Tells the view there is output to read. Called on any thread, at most
/// once until the view has drained the queue.
pub(crate) type Waker = Box<dyn Fn() + Send + Sync>;

pub(crate) struct Shared {
    output: Mutex<Output>,
    waker: Mutex<WakerSlot>,
    input: Mutex<InputQueue>,
}

/// Core → view.
#[derive(Default)]
struct Output {
    bytes: ByteQueue,
    /// Marks, each at the output offset (in `stats.pushed`) it was put at:
    /// what the view does between two bytes, in the bytes' order.
    marks: VecDeque<(u64, Mark)>,
    /// The view's current attachment (0: none). Every attach takes a new
    /// number, so a wake decided for an earlier one never reaches a later
    /// waker.
    attachment: u64,
    /// The last attachment number handed out.
    attachments: u64,
    /// A wake went out for `attachment` and the view has not drained since.
    wake_pending: bool,
    /// The core's end is gone.
    closed: bool,
    stats: Stats,
}

#[derive(Default)]
struct WakerSlot {
    attachment: u64,
    waker: Option<Waker>,
}

/// View → core: bytes, and resizes and pastes at the byte offsets they
/// came at.
#[derive(Default)]
struct InputQueue {
    bytes: ByteQueue,
    /// Bytes written so far.
    written: u64,
    /// Bytes taken so far.
    taken: u64,
    /// Resizes and pastes, each at the offset (in `written`) it came after.
    events: VecDeque<(u64, InputEvent)>,
    waker: Option<std::task::Waker>,
    /// The core's end is gone: writes are refused.
    closed: bool,
}

/// What [`Shared::close`] and [`Shared::detach`] leave for the registry.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unregister {
    /// The other end still uses the channel.
    Keep,
    /// Both ends are done with it.
    Remove,
}

impl Shared {
    pub(crate) fn new() -> Shared {
        Shared {
            output: Mutex::new(Output::default()),
            waker: Mutex::new(WakerSlot::default()),
            input: Mutex::new(InputQueue::default()),
        }
    }

    // --- Output: the core pushes, the view reads -----------------------------

    /// Appends `data` to the output and wakes the view if no wake is
    /// outstanding.
    pub(crate) fn push(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let wake = {
            let mut out = lock(&self.output);
            out.bytes.push(data);
            out.stats.pushes += 1;
            out.stats.pushed += data.len() as u64;
            out.stats.peak_queued = out.stats.peak_queued.max(out.bytes.len() as u64);
            out.claim_wake()
        };
        if let Some(attachment) = wake {
            self.wake(attachment);
        }
    }

    /// Calls the waker of `attachment`, if it is still the view's.
    fn wake(&self, attachment: u64) {
        let slot = lock(&self.waker);
        if slot.attachment == attachment
            && let Some(waker) = &slot.waker
        {
            waker();
        }
    }

    /// Puts `mark` after everything pushed so far and wakes the view if no
    /// wake is outstanding.
    pub(crate) fn push_mark(&self, mark: Mark) {
        let wake = {
            let mut out = lock(&self.output);
            let at = out.stats.pushed;
            out.marks.push_back((at, mark));
            out.stats.marks += 1;
            out.claim_wake()
        };
        if let Some(attachment) = wake {
            self.wake(attachment);
        }
    }

    /// Moves up to `buf.len()` bytes of output into `buf`, passing over any
    /// mark (a reader that doesn't ask for marks has nothing to do with
    /// them). Once the queue is empty the next push wakes the view again.
    pub(crate) fn read(&self, buf: &mut [u8]) -> usize {
        let mut out = lock(&self.output);
        let n = out.bytes.read(buf);
        out.stats.read += n as u64;
        let read = out.stats.read;
        while out.marks.front().is_some_and(|&(at, _)| at <= read) {
            out.marks.pop_front();
        }
        out.note_drained();
        n
    }

    /// Moves up to `buf.len()` bytes of output into `buf`, stopping at the
    /// next mark: the bytes before it, then the mark itself (taken). The
    /// queue is drained when this returns fewer bytes than asked and no
    /// mark; then the next push (or mark) wakes the view again.
    pub(crate) fn read_marked(&self, buf: &mut [u8]) -> (usize, Option<Mark>) {
        let mut out = lock(&self.output);
        let read = out.stats.read;
        let cap = match out.marks.front() {
            Some(&(at, _)) => buf.len().min((at - read) as usize),
            None => buf.len(),
        };
        let n = out.bytes.read(&mut buf[..cap]);
        out.stats.read += n as u64;
        let read = out.stats.read;
        let mark = match out.marks.front() {
            Some(&(at, mark)) if at == read => {
                out.marks.pop_front();
                Some(mark)
            }
            _ => None,
        };
        out.note_drained();
        (n, mark)
    }

    /// Makes `waker` the view's; wakes it right away if output is waiting.
    pub(crate) fn attach(&self, waker: Waker) {
        let mut slot = lock(&self.waker);
        let (attachment, wake) = {
            let mut out = lock(&self.output);
            out.attachments += 1;
            out.attachment = out.attachments;
            out.wake_pending = false;
            (out.attachment, out.claim_wake().is_some())
        };
        slot.attachment = attachment;
        let waker = slot.waker.insert(waker);
        if wake {
            waker();
        }
    }

    /// Drops the view's waker. Once this returns it is not called again.
    pub(crate) fn detach(&self) -> Unregister {
        let mut slot = lock(&self.waker);
        let unregister = {
            let mut out = lock(&self.output);
            out.attachment = 0;
            out.wake_pending = false;
            if out.closed {
                Unregister::Remove
            } else {
                Unregister::Keep
            }
        };
        let waker = slot.waker.take();
        slot.attachment = 0;
        drop(slot);
        drop(waker);
        unregister
    }

    // --- Input: the view writes, the core takes ------------------------------

    /// Appends `data` to the input; `false` once the core's end is gone.
    pub(crate) fn write(&self, data: &[u8]) -> bool {
        let waker = {
            let mut input = lock(&self.input);
            if input.closed {
                return false;
            }
            if data.is_empty() {
                return true;
            }
            input.bytes.push(data);
            input.written += data.len() as u64;
            input.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }

    /// Queues a resize after the bytes written so far. Resizes with nothing
    /// written or pasted between them collapse into the last; `false` once
    /// the core's end is gone.
    pub(crate) fn resize(&self, cols: u16, rows: u16) -> bool {
        let waker = {
            let mut input = lock(&self.input);
            if input.closed {
                return false;
            }
            let at = input.written;
            let resize = InputEvent::Resize { cols, rows };
            match input.events.back_mut() {
                Some((last, event @ InputEvent::Resize { .. })) if *last == at => *event = resize,
                _ => input.events.push_back((at, resize)),
            }
            input.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }

    /// Queues a paste after the bytes written so far, never merged with
    /// anything; `false` once the core's end is gone.
    pub(crate) fn paste(&self, text: String) -> bool {
        let waker = {
            let mut input = lock(&self.input);
            if input.closed {
                return false;
            }
            if text.is_empty() {
                return true;
            }
            let at = input.written;
            input.events.push_back((at, InputEvent::Paste(text)));
            input.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }

    /// The next input event: every byte written up to the next resize or
    /// paste, as one chunk, or that resize or paste. `None` once the core's
    /// end is gone and everything was taken.
    pub(crate) fn poll_input(&self, cx: &mut Context<'_>) -> Poll<Option<InputEvent>> {
        let mut input = lock(&self.input);
        let input = &mut *input;
        if input
            .events
            .front()
            .is_some_and(|(at, _)| *at == input.taken)
            && let Some((_, event)) = input.events.pop_front()
        {
            return Poll::Ready(Some(event));
        }
        let until = input.events.front().map_or(input.written, |(at, _)| *at);
        if until > input.taken {
            let mut bytes = vec![0; (until - input.taken) as usize];
            let n = input.bytes.read(&mut bytes);
            debug_assert_eq!(n, bytes.len());
            input.taken = until;
            return Poll::Ready(Some(InputEvent::Bytes(bytes)));
        }
        if input.closed {
            return Poll::Ready(None);
        }
        match &input.waker {
            Some(waker) if waker.will_wake(cx.waker()) => {}
            _ => input.waker = Some(cx.waker().clone()),
        }
        Poll::Pending
    }

    /// Nobody takes input any more: refuse it and drop what is queued.
    pub(crate) fn close_input(&self) {
        let mut input = lock(&self.input);
        input.closed = true;
        input.bytes = ByteQueue::default();
        input.events.clear();
        input.waker = None;
    }

    // --- Both ----------------------------------------------------------------

    /// The core's end is gone: input is refused from now on (what was
    /// written is still taken), the view can still read the output's tail.
    pub(crate) fn close(&self) -> Unregister {
        let waker = {
            let mut input = lock(&self.input);
            input.closed = true;
            input.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        let mut out = lock(&self.output);
        out.closed = true;
        if out.attachment == 0 {
            Unregister::Remove
        } else {
            Unregister::Keep
        }
    }

    pub(crate) fn stats(&self) -> Stats {
        let out = lock(&self.output);
        Stats {
            queued: out.bytes.len() as u64,
            ..out.stats
        }
    }
}

impl Output {
    /// Whether anything waits for the view: bytes or a mark.
    fn waiting(&self) -> bool {
        !self.bytes.is_empty() || !self.marks.is_empty()
    }

    /// After a read: once nothing waits, the next push wakes again.
    fn note_drained(&mut self) {
        if !self.waiting() && self.wake_pending {
            self.wake_pending = false;
            self.stats.drains += 1;
        }
    }

    /// Claims the one outstanding wake if output is waiting, a view is
    /// attached and no wake is out: the attachment to wake.
    fn claim_wake(&mut self) -> Option<u64> {
        if self.attachment == 0 || self.wake_pending || !self.waiting() {
            return None;
        }
        self.wake_pending = true;
        self.stats.wakes += 1;
        Some(self.attachment)
    }
}
