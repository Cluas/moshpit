//! B102: the work the parser does per line of an open `%begin` block. A
//! backfill reply (`capture-pane -S -2000`, what the user waits on when a
//! pane opens) is thousands of lines, and the parser used to format both
//! terminator needles again for every one of them — three allocations per
//! line where one (the line itself) is needed, and 3–6 MB/s on such a reply
//! against 66–300 MB/s elsewhere. Counted, not timed: allocations are
//! deterministic on any machine and under any load.
//!
//! Its own test binary, because the counting allocator is global; it counts
//! only on the thread that asked.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use moshpit_tmux::{ControlParser, Event};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: forwards every call to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: the caller's contract, passed on.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocations `body` makes on this thread.
fn allocations(body: impl FnOnce()) -> usize {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.with(|counting| counting.set(true));
    body();
    COUNTING.with(|counting| counting.set(false));
    ALLOCATIONS.load(Ordering::Relaxed)
}

const LINES: usize = 4000;

/// One allocation per block line (the line's own String), plus the lines
/// vector growing by doubling and a few for the block itself.
#[test]
fn a_block_line_costs_one_allocation() {
    // The boot block, then a reply block whose lines look like a coloured
    // backfill — some of them even mention a terminator mid-line, which is
    // content.
    let mut parser = ControlParser::new();
    let _ = parser.feed_collect(b"%begin 1700000000 1 0\n%end 1700000000 1 0\n");
    let mut reply = b"%begin 1700000000 2 1\n".to_vec();
    for n in 0..LINES {
        if n % 100 == 0 {
            reply.extend_from_slice(
                format!("log: saw %end 1700000000 2 1 from the other client, line {n}\n")
                    .as_bytes(),
            );
        } else {
            reply.extend_from_slice(
                format!("\x1b[1;34mdrwxr-xr-x\x1b[0m  12 user  staff   384 Nov 12 10:{n:02} src\n")
                    .as_bytes(),
            );
        }
    }
    let body = reply.clone();
    let mut events = Vec::with_capacity(4);
    let counted = allocations(|| parser.feed(&body, |event| events.push(event)));
    assert!(events.is_empty(), "the block is still open: {events:?}");
    // ⌈log2(4000)⌉ = 12 vector growths; leave room for the block's own.
    let budget = LINES + 32;
    assert!(
        counted <= budget,
        "{counted} allocations for {LINES} block lines (budget {budget}): \
         the per-line path allocates more than the line"
    );
    let events = parser.feed_collect(b"%end 1700000000 2 1\n");
    match &events[..] {
        [Event::CommandResponse(response)] => {
            assert!(!response.is_error);
            assert_eq!(response.lines.len(), LINES);
        }
        other => panic!("expected one response, got {other:?}"),
    }
}

/// The windowed search still finds an embedded terminator after a long
/// line of content (a truncated line glued to the block's own `%end`), and
/// still ignores one followed by more than a flags' worth of text.
#[test]
fn an_embedded_terminator_is_found_at_the_end_of_a_long_line() {
    let mut parser = ControlParser::new();
    let _ = parser.feed_collect(b"%begin 1700000000 1 0\n%end 1700000000 1 0\n");
    let long = "x".repeat(10_000);
    let mid = format!("{long}%end 1700000000 2 1 and then more text\n");
    let glued = format!("{long}é%end 1700000000 2 1\n");
    let mut stream = b"%begin 1700000000 2 1\n".to_vec();
    stream.extend_from_slice(mid.as_bytes());
    stream.extend_from_slice(glued.as_bytes());
    let events = parser.feed_collect(&stream);
    match &events[..] {
        [Event::CommandResponse(response)] => {
            assert!(!response.is_error);
            assert_eq!(response.lines.len(), 2);
            assert_eq!(response.lines[0], mid.trim_end_matches('\n'));
            assert_eq!(response.lines[1], format!("{long}é"));
        }
        other => panic!("expected one response, got {other:?}"),
    }
}
