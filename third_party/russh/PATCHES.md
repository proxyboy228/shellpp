# russh 0.64.1, patched for Shell++

This directory is russh 0.64.1 as published on crates.io (Apache-2.0,
<https://github.com/warp-tech/russh>), used through `[patch.crates-io]` in the
root `Cargo.toml` (D100, item 3). `LICENSE` is the Apache License 2.0 that
russh is distributed under.

Changes to the published crate (each code change is marked "Shell++ patch"):

1. **The channel leak** (D98, D100). `src/client/encrypted.rs`, the
   `CHANNEL_CLOSE` handler: when the server's `CHANNEL_CLOSE` arrives for a
   channel the client already closed, the handler returned before removing
   the channel's sender from the session's `channels` map, so every channel
   the client closed first stayed in memory until the connection ended. The
   patch removes it before returning. Upstream text:
   `docs/upstream/russh-channel-leak.md`; tests:
   `crates/ssh/tests/channel_close.rs`.
2. **Empty data read as the end of the stream** (D191).
   `src/channels/io/rx.rs`, `ChannelRx::poll_read` (the reader behind
   `Channel::into_stream`, `make_reader` and `make_reader_ext`): an empty
   `CHANNEL_DATA` or `CHANNEL_EXTENDED_DATA`, which is valid SSH, was handed
   to the reader as a read of 0 bytes, and an `AsyncRead` user takes that
   for the end of the stream; an SFTP session ended on such a packet. The
   patch skips an empty message like any other message the reader does not
   want (two `!data.is_empty()` guards). Upstream text:
   `docs/upstream/russh-empty-data.md`; tests:
   `crates/ssh/tests/empty_channel_data.rs`.
3. `Cargo.toml`: the `[[example]]`, `[[test]]` and `[[bench]]` targets are
   removed, because their files are not copied here.

## Removing a patch

When a russh release fixes one of them, the Shell++ tests named above stay
and must pass on that release.

- **Both fixed:** set `russh` in the root `Cargo.toml` to that release and
  remove this directory, the `[patch.crates-io]` entry of the root
  `Cargo.toml`, the `third_party/russh` allowlist of `.gitleaks.toml` and the
  "Patched crates" paragraph of `THIRD_PARTY_NOTICES.md`.
- **Only one fixed:** the copy stays. Replace its source with the fixed
  release's (and its version in the root `Cargo.toml`, this file and
  `THIRD_PARTY_NOTICES.md`), apply the other patch again and remove the
  fixed one from the list above.
