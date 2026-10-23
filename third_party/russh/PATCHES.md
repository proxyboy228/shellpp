# russh 0.64.1, patched for Shell++

This directory is russh 0.64.1 as published on crates.io (Apache-2.0,
<https://github.com/warp-tech/russh>), used through `[patch.crates-io]` in the
root `Cargo.toml` (D100, item 3). `LICENSE` is the Apache License 2.0 that
russh is distributed under.

Changes to the published crate:

1. `src/client/encrypted.rs`, the `CHANNEL_CLOSE` handler: when the server's
   `CHANNEL_CLOSE` arrives for a channel the client already closed, the
   handler returned before removing the channel's sender from the session's
   `channels` map, so every channel the client closed first stayed in memory
   until the connection ended (D98). The patch removes it before returning
   (marked "Shell++ patch").
2. `Cargo.toml`: the `[[example]]`, `[[test]]` and `[[bench]]` targets are
   removed, because their files are not copied here.

The text for the upstream issue and pull request is in
`docs/upstream/russh-channel-leak.md`. Remove this directory and the
`[patch.crates-io]` entry once a russh release contains the fix (CLAUDE.md).
