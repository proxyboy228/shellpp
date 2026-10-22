//! Session recordings (docs/storage-format.md, section 8; D16, D75): the
//! `recordings/<file_id>.shpprec` file of one session's output.
//!
//! - The header (63 bytes) names the file, the personal vault and the epoch
//!   of the key, and holds the random STREAM nonce prefix.
//! - The body is asciicast v2 (one JSON line per output event, input never)
//!   in chunks of 64 KiB of plaintext, each sealed with XChaCha20-Poly1305
//!   under `recording_key[file]`: nonce = prefix (19) ‖ be32(index) ‖ last
//!   flag, AAD = the header. The final chunk (0 to 65 535 bytes) is always
//!   written on close, so a file without it was not closed and reads as
//!   incomplete; chunks out of order, cut or changed fail to open.
//! - Output waits in memory until a chunk is full: a crash loses at most
//!   the last 64 KiB. Each sealed chunk's plaintext and the waiting buffer
//!   are zeroized when they go (copies made while a line is built are
//!   not; the terminal holds the same text anyway).
//! - A write that fails stops the recording: nothing more is written, so
//!   the chunks already in the file still open (as an incomplete file).
//! - A recording holds at most [`MAX_CHUNKS`] chunks (256 MiB of text): then
//!   a marker event (`"m"`, "size limit") ends what is recorded, and the
//!   rest of the session's output is not written (section 12 review: a
//!   session left running for days grew the file, and opening it, without
//!   bound).
//!   Reading keeps the chunks that open and stops at the first one that
//!   does not (incomplete); only a file whose first chunk fails is refused.

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::{self, Key32};

/// The file's magic.
pub const MAGIC: &[u8; 7] = b"shpprec";
/// The format version.
pub const VERSION: u8 = 1;
/// Header bytes `[0, 63)`.
pub const HEADER_LEN: usize = 63;
/// Plaintext bytes of a chunk that is not the final one.
pub const CHUNK: usize = 65_536;
/// The tag of each sealed chunk.
const TAG: usize = 16;
/// Chunks a recording holds at most (256 MiB of plaintext).
pub const MAX_CHUNKS: u32 = 4096;
/// A non-final chunk as stored.
const FRAME: usize = CHUNK + TAG;
/// The directory of the recordings in the data directory.
pub const DIR_NAME: &str = "recordings";
/// The file name extension.
pub const EXTENSION: &str = "shpprec";

/// Why a recording could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("recording file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a recording file")]
    Format,
    #[error("the recording does not open with its key")]
    Damaged,
    #[error("no key for the recording")]
    NoKey,
}

/// The clear header of a recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub file_id: Uuid,
    pub vault_id: Uuid,
    pub key_epoch: u32,
    prefix: [u8; 19],
}

impl Header {
    /// A new header with a random nonce prefix.
    pub fn new(file_id: Uuid, vault_id: Uuid, key_epoch: u32) -> Result<Self, RecordingError> {
        let prefix = crypto::random_array::<19>().map_err(|_| RecordingError::NoKey)?;
        Ok(Self {
            file_id,
            vault_id,
            key_epoch,
            prefix,
        })
    }

    pub fn bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..7].copy_from_slice(MAGIC);
        out[7] = VERSION;
        out[8..24].copy_from_slice(self.file_id.as_bytes());
        out[24..40].copy_from_slice(self.vault_id.as_bytes());
        out[40..44].copy_from_slice(&self.key_epoch.to_be_bytes());
        out[44..63].copy_from_slice(&self.prefix);
        out
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, RecordingError> {
        let header = bytes.get(..HEADER_LEN).ok_or(RecordingError::Format)?;
        if &header[..7] != MAGIC || header[7] != VERSION {
            return Err(RecordingError::Format);
        }
        let id = |range: std::ops::Range<usize>| {
            <[u8; 16]>::try_from(&header[range]).map(Uuid::from_bytes)
        };
        Ok(Self {
            file_id: id(8..24).map_err(|_| RecordingError::Format)?,
            vault_id: id(24..40).map_err(|_| RecordingError::Format)?,
            key_epoch: u32::from_be_bytes(
                header[40..44]
                    .try_into()
                    .map_err(|_| RecordingError::Format)?,
            ),
            prefix: header[44..63]
                .try_into()
                .map_err(|_| RecordingError::Format)?,
        })
    }

    fn nonce(&self, index: u32, last: bool) -> [u8; 24] {
        let mut nonce = [0u8; 24];
        nonce[..19].copy_from_slice(&self.prefix);
        nonce[19..23].copy_from_slice(&index.to_be_bytes());
        nonce[23] = u8::from(last);
        nonce
    }
}

/// Writes one session's output into its recording file.
pub struct RecordingWriter {
    file: File,
    key: Key32,
    header: Header,
    aad: [u8; HEADER_LEN],
    index: u32,
    /// Plaintext waiting for a full chunk.
    buffer: Zeroizing<Vec<u8>>,
    /// The end of the last output when it stopped inside a UTF-8 sequence.
    carry: Vec<u8>,
    started: Instant,
    finished: bool,
    /// A write failed: the recording stopped there.
    failed: bool,
    /// The size limit ([`MAX_CHUNKS`]; smaller in tests).
    max_chunks: u32,
    /// The limit was reached: later output is not written.
    full: bool,
}

impl std::fmt::Debug for RecordingWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingWriter")
            .field("file_id", &self.header.file_id)
            .field("chunks", &self.index)
            .finish_non_exhaustive()
    }
}

impl RecordingWriter {
    /// Creates the file at `path` (it must not exist) and starts the
    /// asciicast with the terminal's size and the start time.
    pub fn create(
        path: &Path,
        header: Header,
        key: Key32,
        size: (u16, u16),
        unix_seconds: u64,
    ) -> Result<Self, RecordingError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        let aad = header.bytes();
        file.write_all(&aad)?;
        let mut writer = Self {
            file,
            key,
            header,
            aad,
            index: 0,
            buffer: Zeroizing::new(Vec::with_capacity(CHUNK * 2)),
            carry: Vec::new(),
            started: Instant::now(),
            finished: false,
            failed: false,
            max_chunks: MAX_CHUNKS,
            full: false,
        };
        let (width, height) = size;
        let first = serde_json::json!({
            "version": 2,
            "width": width,
            "height": height,
            "timestamp": unix_seconds,
        });
        writer.push_line(&first.to_string())?;
        Ok(writer)
    }

    pub fn file_id(&self) -> Uuid {
        self.header.file_id
    }

    /// Output of the session (UTF-8), timed from the start.
    pub fn output(&mut self, data: &[u8]) -> Result<(), RecordingError> {
        let seconds = self.started.elapsed().as_secs_f64();
        self.output_at(seconds, data)
    }

    /// Output at `seconds` from the start.
    pub fn output_at(&mut self, seconds: f64, data: &[u8]) -> Result<(), RecordingError> {
        if self.failed || self.finished {
            return Err(RecordingError::Format);
        }
        if data.is_empty() || self.full {
            return Ok(());
        }
        if self.index >= self.max_chunks {
            self.full = true;
            self.carry.clear();
            let line = serde_json::to_string(&(seconds, "m", "size limit"))
                .map_err(|_| RecordingError::Format)?;
            return self.push_line(&line);
        }
        let mut bytes = std::mem::take(&mut self.carry);
        bytes.extend_from_slice(data);
        // A sequence cut at the end waits for the next output.
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text.to_owned(),
            Err(error) if error.error_len().is_none() => {
                let (whole, rest) = bytes.split_at(error.valid_up_to());
                self.carry = rest.to_vec();
                String::from_utf8_lossy(whole).into_owned()
            }
            Err(_) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        if text.is_empty() {
            return Ok(());
        }
        let seconds = (seconds * 1_000_000.0).round() / 1_000_000.0;
        let line =
            serde_json::to_string(&(seconds, "o", text)).map_err(|_| RecordingError::Format)?;
        self.push_line(&line)
    }

    fn push_line(&mut self, line: &str) -> Result<(), RecordingError> {
        self.buffer.extend_from_slice(line.as_bytes());
        self.buffer.push(b'\n');
        while self.buffer.len() >= CHUNK {
            let chunk = Zeroizing::new(self.buffer.drain(..CHUNK).collect::<Vec<u8>>());
            self.seal(&chunk, false)?;
        }
        Ok(())
    }

    /// Seals `plain` as the next chunk. A failure stops the recording.
    fn seal(&mut self, plain: &[u8], last: bool) -> Result<(), RecordingError> {
        let sealed = (|| {
            let nonce = self.header.nonce(self.index, last);
            let mut sealed = Vec::with_capacity(plain.len() + TAG);
            crypto::xchacha_seal_append(&self.key, &nonce, &self.aad, plain, &mut sealed)
                .map_err(|_| RecordingError::Damaged)?;
            self.file.write_all(&sealed)?;
            self.index = self.index.checked_add(1).ok_or(RecordingError::Format)?;
            Ok(())
        })();
        if sealed.is_err() {
            self.failed = true;
        }
        sealed
    }

    /// Writes the final chunk and closes the file.
    pub fn finish(mut self) -> Result<(), RecordingError> {
        self.finish_inner()
    }

    fn finish_inner(&mut self) -> Result<(), RecordingError> {
        if self.finished || self.failed {
            return Ok(());
        }
        if !self.carry.is_empty() {
            let rest = std::mem::take(&mut self.carry);
            let seconds = self.started.elapsed().as_secs_f64();
            let text = String::from_utf8_lossy(&rest).into_owned();
            let line =
                serde_json::to_string(&(seconds, "o", text)).map_err(|_| RecordingError::Format)?;
            // Through the same split: the final chunk stays under 64 KiB.
            self.push_line(&line)?;
        }
        self.finished = true;
        let last = Zeroizing::new(std::mem::take(&mut *self.buffer));
        self.seal(&last, true)?;
        self.file.flush()?;
        Ok(())
    }
}

impl Drop for RecordingWriter {
    fn drop(&mut self) {
        if let Err(error) = self.finish_inner() {
            tracing::warn!(%error, "recording not closed");
        }
    }
}

/// A recording read back.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub header: Header,
    /// The terminal size of the asciicast header.
    pub size: (u16, u16),
    /// The output events: seconds from the start and the text.
    pub events: Vec<(f64, String)>,
    /// The final chunk is there (the file was closed).
    pub complete: bool,
}

impl Recording {
    /// The output as plain text: escape sequences and control characters
    /// removed (the viewer and "Export to .txt", UI 11.2).
    pub fn plain_text(&self) -> String {
        let len = self.events.iter().map(|(_, text)| text.len()).sum();
        let mut output = zeroize::Zeroizing::new(String::with_capacity(len));
        for (_, text) in &self.events {
            output.push_str(text);
        }
        plain_text(&output)
    }
}

/// The decrypted output is wiped with the recording (docs/security.md 9.5).
impl Drop for Recording {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        for (_, text) in &mut self.events {
            text.zeroize();
        }
    }
}

/// Opens `bytes` of a recording file with the key `key_for` gives for its
/// header.
pub fn read(
    bytes: &[u8],
    key_for: impl FnOnce(&Header) -> Option<Key32>,
) -> Result<Recording, RecordingError> {
    let header = Header::parse(bytes)?;
    let key = key_for(&header).ok_or(RecordingError::NoKey)?;
    let aad = header.bytes();
    let body = &bytes[HEADER_LEN..];
    // Frames of FRAME bytes are non-final chunks; a shorter rest is the
    // final one. A body that is a whole number of frames was not closed.
    let whole = body.len() / FRAME;
    let rest = body.len() % FRAME;
    let complete = rest > 0 || body.is_empty();
    let mut plain = Zeroizing::new(Vec::with_capacity(body.len()));
    // The chunks that open, in order; the first that does not ends the
    // reading (the recording is then incomplete).
    let mut opened_all = true;
    let mut opened_any = false;
    for index in 0..whole {
        let frame = &body[index * FRAME..(index + 1) * FRAME];
        let index = u32::try_from(index).map_err(|_| RecordingError::Format)?;
        match crypto::xchacha_open(&key, &header.nonce(index, false), &aad, frame) {
            Ok(opened) => {
                plain.extend_from_slice(&opened);
                opened_any = true;
            }
            Err(_) => {
                opened_all = false;
                break;
            }
        }
    }
    if rest > 0 && opened_all {
        let index = u32::try_from(whole).map_err(|_| RecordingError::Format)?;
        let last = &body[whole * FRAME..];
        match (rest >= TAG)
            .then(|| crypto::xchacha_open(&key, &header.nonce(index, true), &aad, last).ok())
            .flatten()
        {
            Some(opened) => {
                plain.extend_from_slice(&opened);
                opened_any = true;
            }
            None => opened_all = false,
        }
    }
    if !opened_any && !body.is_empty() {
        return Err(RecordingError::Damaged);
    }
    let complete = complete && opened_all;
    // One copy of the size of the text, wiped when parsed.
    let text = zeroize::Zeroizing::new(String::from_utf8_lossy(&plain).into_owned());
    let mut lines = text.lines();
    let size = lines
        .next()
        .and_then(|first| serde_json::from_str::<serde_json::Value>(first).ok())
        .map_or((80, 24), |first| {
            let dimension = |name: &str, default: u16| {
                first
                    .get(name)
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .unwrap_or(default)
            };
            (dimension("width", 80), dimension("height", 24))
        });
    let events = lines
        .filter_map(|line| serde_json::from_str::<(f64, String, String)>(line).ok())
        .filter(|(_, kind, _)| kind == "o")
        .map(|(seconds, _, text)| (seconds, text))
        .collect();
    Ok(Recording {
        header,
        size,
        events,
        complete: complete && !body.is_empty(),
    })
}

/// Terminal output as plain text: CSI, OSC and other escape sequences and
/// control characters other than line breaks and tabs removed; `\r\n`
/// becomes a line break.
pub fn plain_text(output: &str) -> String {
    let mut out = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    // CSI: parameters, then one final byte 0x40-0x7e.
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']' | 'P' | '_' | '^') => {
                    // OSC, DCS, APC, PM: to BEL or ST (ESC \).
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some('(' | ')' | '*' | '+' | '#') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
