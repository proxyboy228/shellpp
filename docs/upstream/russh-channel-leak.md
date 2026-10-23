# russh: a channel the client closes first is never removed from `Session::channels`

Text for an issue and a pull request in <https://github.com/warp-tech/russh>,
prepared in run 7 (D100, item 3). The owner sends them. Shell++ carries the
same change as a path patch (`third_party/russh/PATCHES.md`) until a release
contains it.

## Issue

**Title:** Client leaks the channel entry when it closes a channel before the server does

**russh version:** 0.64.1 (also current `main` at the time of writing, as far
as `src/client/encrypted.rs` shows).

**Description.** When the client closes a channel first (`Channel::close()`,
or dropping a `ChannelStream`), `Session::close()` sends `CHANNEL_CLOSE` and
removes the channel from the encrypted state's table (`enc.channels`). When
the server's `CHANNEL_CLOSE` arrives, the handler in
`client/encrypted.rs` checks `is_established_channel(channel_num)`, which
looks the channel up in that same table, finds nothing and returns early,
before `self.channels.remove(&channel_num)`. The client-side entry (its
`ChannelRef`, with the sender of the channel's message queue) stays in
`Session::channels` until the connection ends.

Long-lived connections that open many short channels and close them first
grow without bound: in our case port forwarding (one `direct-tcpip` channel
per forwarded connection) grew by about 4 MB a minute under steady traffic,
and the same pattern applies to exec channels and to forwarded agent
channels (one per use of the agent on the server).

**Reproduction.**

1. Connect a russh client to any server (OpenSSH, or a russh server, which
   answers a `CHANNEL_CLOSE` with its own as RFC 4254, section 5.3 asks).
2. Open a session channel, send `exec` (no reply wanted), and call
   `channel.close()` at once, keeping the `Channel` value.
3. Wait for `channel.wait()` to return `None`.

Expected: once the server's `CHANNEL_CLOSE` has arrived, the entry is gone,
the queue's sender is dropped and `wait()` returns `None`.

Actual: `wait()` never returns; the entry stays in `Session::channels`.

The same happens for a channel the server opens (`auth-agent@openssh.com`,
`forwarded-tcpip`) when the client closes it first.

A test for this, written against russh's own test server would look like:

```rust
let mut channel = handle.channel_open_session().await?;
channel.exec(false, "true").await?;
channel.close().await?;
tokio::time::timeout(Duration::from_secs(10), async {
    while channel.wait().await.is_some() {}
})
.await
.expect("the channel's entry is removed after the server's CLOSE");
```

(Our tests: `crates/ssh/tests/channel_close.rs` in Shell++, for an exec channel
and a forwarded agent channel; both time out on 0.64.1 and pass with the fix.)

## Pull request

**Title:** client: remove the channel entry when the server closes a channel the client already closed

**Body.**

When the client closes a channel first, `Session::close()` removes it from
`enc.channels`, so the server's later `CHANNEL_CLOSE` fails the
`is_established_channel` check and the handler returns before removing the
channel from `Session::channels`. Every channel closed first by the client
then stays in memory (its `ChannelRef` and queue sender) for the life of the
connection, which a long-running client with many short channels (port
forwarding, exec, agent forwarding) sees as steady memory growth.

This removes the entry before the early return. The channel is already
closed on both sides at that point, so nothing else needs to happen; the
channel's own queue now ends (`wait()` returns `None`), which is what a
consumer that closed it first expects.

**Patch:**

```diff
--- a/russh/src/client/encrypted.rs
+++ b/russh/src/client/encrypted.rs
@@ -506,6 +506,9 @@
                 let channel_num = map_err!(ChannelId::decode(&mut r))?;
                 map_err!(ensure_end(&r))?;
                 if !self.common.is_established_channel(channel_num) {
+                    // A channel the client closed first is already gone from the
+                    // encrypted table; drop its entry here as well.
+                    self.channels.remove(&channel_num);
                     return Ok(());
                 }
                 if let Some(ref mut enc) = self.common.encrypted {
```

A regression test (client closes an exec channel first, then waits for the
channel's queue to end, with a timeout) goes into `russh/tests/` next to the
existing channel tests.
