# russh: an empty `CHANNEL_DATA` ends a channel's `AsyncRead` stream

Text for an issue and a pull request in <https://github.com/warp-tech/russh>,
prepared in run 9 (D191). The owner sends them. Shell++ carries the same
change as the second path patch of `third_party/russh/PATCHES.md` until a
release contains it.

## Issue

**Title:** An empty `CHANNEL_DATA` is read as end of stream by `ChannelStream` / `make_reader`

**russh version:** 0.64.1 (also current `main` at the time of writing, as far
as `src/channels/io/rx.rs` shows).

**Description.** RFC 4254, section 5.2, puts no lower bound on the data of
`SSH_MSG_CHANNEL_DATA` and `SSH_MSG_CHANNEL_EXTENDED_DATA`, and some servers
send an empty one now and then. The client's `CHANNEL_DATA` handler passes
it on as `ChannelMsg::Data` with empty `data`, and `ChannelRx::poll_read`
copies its 0 bytes into the caller's buffer and returns `Poll::Ready(Ok(()))`.
Under the `AsyncRead` contract, a successful read that fills nothing into a
buffer with room means end of stream, so every consumer stops there:
`read_to_end` returns early, `copy` ends, and `russh-sftp` (over
`Channel::into_stream()`) sees an unexpected EOF and closes the SFTP session,
although the channel is still open and the server goes on sending. The same
holds for an empty `CHANNEL_EXTENDED_DATA` read with `make_reader_ext`.

**Reproduction.** russh itself never sends an empty data packet (its
`data_noqueue` loops while there is data left), so the server needs help:

1. Run a russh server and a russh client that both prefer the `none` cipher,
   MAC and compression (test only), with a TCP proxy between them that
   forwards the server's packets and, after the first `CHANNEL_DATA` of a
   channel, adds an empty one for the same recipient channel (payload
   `[94, channel (u32), 0u32]`, padded to 8 bytes as RFC 4253, section 6,
   asks).
2. On the server, answer `exec` with `data("first\n")`, `data("second\n")`,
   `eof` and `close`.
3. On the client, `exec`, then `channel.into_stream().read_to_end(&mut out)`.

Expected: `out == "first\nsecond\n"`.

Actual: `out == "first\n"`; the read ends at the empty packet. With an SFTP
subsystem in place of `exec`, the next request after the empty packet fails
with "session closed".

(Our tests: `crates/ssh/tests/empty_channel_data.rs` in Shell++, with that
proxy: an SFTP session, a stream of an exec channel and a stderr reader; all
three fail on 0.64.1 and pass with the fix.)

## Pull request

**Title:** channels: skip empty data in `ChannelRx` instead of reporting end of stream

**Body.**

An empty `CHANNEL_DATA` (or `CHANNEL_EXTENDED_DATA` of the stream being
read) is valid SSH, but `ChannelRx::poll_read` returned it as a successful
read of 0 bytes, which `AsyncRead` users take for the end of the stream:
reads through `Channel::into_stream()`, `make_reader()` and
`make_reader_ext()` ended at the first empty packet, and an SFTP session on
top of them closed.

This makes an empty message take the arm that already skips messages the
reader does not want (window adjustments, requests, the other stream), so
the next poll reads the next message. A message with data is read as
before, and `Eof` still ends the stream.

**Patch:**

```diff
--- a/russh/src/channels/io/rx.rs
+++ b/russh/src/channels/io/rx.rs
@@ -43,7 +43,9 @@ where
         };
 
         match (&msg, self.ext) {
-            (ChannelMsg::Data { data }, None) => {
+            // Empty data is skipped like any other message; returned as a
+            // read of 0 bytes, it would read as the end of the stream.
+            (ChannelMsg::Data { data }, None) if !data.is_empty() => {
                 let readable = buf.remaining().min(data.len() - idx);
 
                 // Clamped to maximum `buf.remaining()` and `data.len() - idx` with `.min`
@@ -57,7 +59,9 @@ where
 
                 Poll::Ready(Ok(()))
             }
-            (ChannelMsg::ExtendedData { data, ext }, Some(target)) if *ext == target => {
+            (ChannelMsg::ExtendedData { data, ext }, Some(target))
+                if *ext == target && !data.is_empty() =>
+            {
                 let readable = buf.remaining().min(data.len() - idx);
 
                 // Clamped to maximum `buf.remaining()` and `data.len() - idx` with `.min`
```

A regression test can sit in the crate's own tests, where the channel's
receiver can be fed directly: push `ChannelMsg::Data` with `b"first"`, an
empty `ChannelMsg::Data`, `b"second"` and `ChannelMsg::Eof` into the
channel's queue and check that `read_to_end` through `make_reader()`
returns `b"firstsecond"`; the same with `ExtendedData` and
`make_reader_ext(Some(1))`.
