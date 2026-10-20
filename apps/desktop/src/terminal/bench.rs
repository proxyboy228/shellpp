//! Headless throughput benchmark (decision 30; owner's step 4): the output
//! of `seq 1 3000000` as a PTY delivers it ("N\r\n"), fed in 32 KiB chunks
//! (the SSH packet size) from a producer thread, the way the session layer
//! feeds a terminal.
//!
//! 1. Parser only: from the first byte to "everything parsed".
//! 2. With frames: the same while the test thread draws a 1280x800 window
//!    every 16 ms, as a 60 Hz display would (GPUI's test text system: the
//!    frame cost covers reading the rows under the lock, hashing, the row
//!    cache and the layout, not glyph rasterization). The wakes of the
//!    parser thread are not pumped here, so frames come only from the loop.
//!
//! Run in release (ignored otherwise):
//! `cargo test --release -p shellpp-desktop --lib terminal::bench -- --ignored --nocapture`

#![allow(clippy::print_stderr)]

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Bounds, Focusable as _, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;

use super::model::{CHUNK_BYTES, QUEUE_CHUNKS};
use super::*;
use crate::theme::{self, Appearance};

const LINES: usize = 3_000_000;
const FEED_CHUNK: usize = 32 * 1024;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// `seq 1 3000000` through a PTY.
fn seq_output() -> Vec<u8> {
    let mut out = Vec::with_capacity(27 * 1024 * 1024);
    for number in 1..=LINES {
        out.extend_from_slice(number.to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Feeds `data` from a new thread; returns when everything is parsed.
fn produce(handle: TerminalHandle, data: Arc<Vec<u8>>) -> thread::JoinHandle<Instant> {
    thread::spawn(move || {
        for chunk in data.chunks(FEED_CHUNK) {
            handle.feed_blocking(chunk).expect("terminal open");
        }
        handle.flush_blocking();
        Instant::now()
    })
}

fn report(label: &str, bytes: usize, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    eprintln!(
        "{label}: {LINES} lines, {:.1} MiB in {:.3} s = {:.1} MiB/s, {:.2} M lines/s",
        bytes as f64 / 1_048_576.,
        seconds,
        bytes as f64 / 1_048_576. / seconds,
        LINES as f64 / 1_000_000. / seconds,
    );
}

#[gpui_kit::test]
#[ignore = "benchmark; run in release with --ignored --nocapture"]
fn seq_three_million_lines(cx: &mut TestAppContext) {
    // The parser thread wakes the view from outside the test scheduler.
    cx.executor().allow_parking();
    let data = Arc::new(seq_output());
    eprintln!(
        "queue bound: {QUEUE_CHUNKS} chunks x {} KiB",
        CHUNK_BYTES / 1024
    );

    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(Appearance::Dark, cx);
        super::init(cx);
    });
    let open = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(1280.), px(800.)),
                ))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| TerminalView::new(TerminalOptions::default(), window, cx))
            })
            .expect("window")
        })
    };

    // 1. Parser only.
    let (window, view) = open(cx);
    let handle = cx.update(|cx| view.read(cx).handle());
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let start = Instant::now();
    let done = produce(handle.clone(), data.clone()).join().unwrap();
    report("parser only", data.len(), done - start);
    assert_eq!(handle.history_size(), DEFAULT_SCROLLBACK);
    let screen = handle.screen_text();
    let last = screen.iter().rev().find(|line| !line.is_empty()).unwrap();
    assert_eq!(last, "3000000");

    // 2. With frames at ~60 Hz.
    let (window, view) = open(cx);
    let handle = cx.update(|cx| view.read(cx).handle());
    cx.update_window(window, |_, window, cx| {
        view.read(cx).focus_handle(cx).focus(window, cx);
        window.render_frame(cx);
    })
    .unwrap();
    let start = Instant::now();
    let producer = produce(handle.clone(), data.clone());
    let mut frames = Vec::new();
    while !producer.is_finished() {
        let frame = Instant::now();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        frames.push(frame.elapsed());
        thread::sleep(FRAME_INTERVAL);
    }
    let done = producer.join().unwrap();
    report("with frames", data.len(), done - start);
    let total: Duration = frames.iter().sum();
    let longest = frames.iter().max().copied().unwrap_or_default();
    eprintln!(
        "frames: {} drawn, average {:.2} ms, longest {:.2} ms",
        frames.len(),
        total.as_secs_f64() * 1000. / frames.len().max(1) as f64,
        longest.as_secs_f64() * 1000.,
    );
    let shaped = cx.update(|cx| view.read(cx).shaped_rows());
    eprintln!("rows laid out: {shaped}");
    assert_eq!(handle.history_size(), DEFAULT_SCROLLBACK);
}
