//! The memory of the 100 000-row feed: a few bytes for each key, and a bounded
//! amount for each live row. CI runs it in release mode.

#![expect(unsafe_code, reason = "a counting allocator needs it")]
#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Host, Phase, Tree};
use showcase::Services;
use showcase::feature::feed::{Feed, FeedMsg, ROWS};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::future::pending;

struct Counting;

std::thread_local! {
    /// Bytes allocated on this thread and not yet freed.
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

fn count(layout: Layout, sign: i64) {
    let size = i64::try_from(layout.size()).unwrap_or(i64::MAX);
    let _during_thread_exit = LIVE.try_with(|live| live.set(live.get() + sign * size));
}

// SAFETY: every call forwards unchanged to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout, 1);
        // SAFETY: the caller upholds the contract of `alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(layout, -1);
        // SAFETY: the caller upholds the contract of `dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

fn live() -> i64 {
    LIVE.with(Cell::get)
}

#[test]
fn the_feed_costs_a_few_bytes_per_key_and_a_bounded_amount_per_live_row() {
    let empty = live();
    let mut tree = Tree::new(NullHost);
    tree.set_phase(Phase::Focused);
    let services = Services::new(|_delay| Box::pin(pending()));
    let feed = tree.mount(move |cx| Feed::new(cx, services));
    tree.send(feed, FeedMsg::Window(0..30)).unwrap();
    let narrow = live();
    tree.send(feed, FeedMsg::Window(0..330)).unwrap();
    let wide = live();

    let rows = i64::try_from(ROWS).unwrap();
    let per_live_row = (wide - narrow) / 300;
    let per_key = (narrow - empty - 30 * per_live_row) / rows;
    println!("{per_key} bytes per key, {per_live_row} bytes per live row");
    // Measured at 7 bytes per key and 435 per live row on aarch64 macOS, in debug
    // and release alike; the margins allow for other targets and layout changes.
    assert!(per_key <= 16, "{per_key} bytes per key");
    assert!(per_live_row <= 650, "{per_live_row} bytes per live row");
}
