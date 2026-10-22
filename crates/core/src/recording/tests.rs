use super::*;

fn key() -> Key32 {
    Key32::from_bytes([7u8; 32])
}

fn header() -> Header {
    Header::new(Uuid::now_v7(), Uuid::now_v7(), 3).unwrap()
}

fn write(dir: &Path, header: &Header, events: &[(f64, &[u8])], finish: bool) -> Vec<u8> {
    let path = dir.join(format!("{}.{EXTENSION}", header.file_id));
    let mut writer =
        RecordingWriter::create(&path, header.clone(), key(), (120, 40), 1_792_595_049).unwrap();
    for (seconds, data) in events {
        writer.output_at(*seconds, data).unwrap();
    }
    if finish {
        writer.finish().unwrap();
    } else {
        // A crash: the final chunk is never written.
        writer.finished = true;
        drop(writer);
    }
    std::fs::read(&path).unwrap()
}

#[test]
fn a_recording_reads_back_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let header = header();
    let bytes = write(
        dir.path(),
        &header,
        &[
            (0.25, b"\x1b[32mok\x1b[0m\r\n"),
            // A UTF-8 character cut between two outputs.
            (0.5, "при".as_bytes()),
            (0.75, "вет\n".as_bytes()),
        ],
        true,
    );
    assert_eq!(&bytes[..7], MAGIC);
    assert_eq!(Header::parse(&bytes).unwrap(), header);
    let recording = read(&bytes, |found| {
        assert_eq!(found.key_epoch, 3);
        Some(key())
    })
    .unwrap();
    assert!(recording.complete);
    assert_eq!(recording.size, (120, 40));
    assert_eq!(
        recording.events[0],
        (0.25, "\x1b[32mok\x1b[0m\r\n".to_owned())
    );
    let text: String = recording
        .events
        .iter()
        .map(|(_, text)| text.as_str())
        .collect();
    assert!(text.ends_with("привет\n"), "{text:?}");
    assert_eq!(recording.plain_text(), "ok\nпривет\n");
    // Nothing of the output is in the file in the clear.
    let file = String::from_utf8_lossy(&bytes);
    assert!(!file.contains("ok") && !file.contains("привет"));
}

/// Several chunks; a file without its final chunk reads as incomplete;
/// a wrong key, a changed byte, a cut chunk or a changed header fail.
#[test]
fn chunks_are_sealed_in_order_and_a_cut_file_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let header = header();
    let line = vec![b'x'; 1_000];
    let events: Vec<(f64, &[u8])> = (0..200).map(|i| (f64::from(i), line.as_slice())).collect();
    let bytes = write(dir.path(), &header, &events, true);
    // Over 3 chunks of 64 KiB, then the final one.
    assert!(bytes.len() > HEADER_LEN + 3 * FRAME);
    let recording = read(&bytes, |_| Some(key())).unwrap();
    assert!(recording.complete);
    assert_eq!(recording.events.len(), 200);

    let crashed = write(
        dir.path(),
        &Header::new(Uuid::now_v7(), Uuid::now_v7(), 1).unwrap(),
        &events,
        false,
    );
    assert_eq!((crashed.len() - HEADER_LEN) % FRAME, 0);
    let partial = read(&crashed, |_| Some(key())).unwrap();
    assert!(!partial.complete);
    assert!(!partial.events.is_empty() && partial.events.len() < 200);

    assert!(matches!(
        read(&bytes, |_| Some(Key32::from_bytes([8u8; 32]))),
        Err(RecordingError::Damaged)
    ));
    assert!(matches!(read(&bytes, |_| None), Err(RecordingError::NoKey)));
    let mut changed = bytes.clone();
    changed[HEADER_LEN + 10] ^= 1;
    assert!(matches!(
        read(&changed, |_| Some(key())),
        Err(RecordingError::Damaged)
    ));
    // The header is the AAD of every chunk.
    let mut other = bytes.clone();
    other[41] ^= 1;
    assert!(matches!(
        read(&other, |_| Some(key())),
        Err(RecordingError::Damaged)
    ));
    // A final chunk cut short, or a non-final chunk dropped: the chunks
    // before it still open, as an incomplete recording, and nothing after
    // the gap is shown.
    let cut = read(&bytes[..bytes.len() - 5], |_| Some(key())).unwrap();
    assert!(!cut.complete && cut.events.len() < 200);
    let mut dropped = bytes[..HEADER_LEN + FRAME].to_vec();
    dropped.extend_from_slice(&bytes[HEADER_LEN + 2 * FRAME..]);
    let gap = read(&dropped, |_| Some(key())).unwrap();
    assert!(!gap.complete);
    let first_chunk = read(&bytes[..HEADER_LEN + FRAME], |_| Some(key())).unwrap();
    assert_eq!(gap.events, first_chunk.events);
    assert!(matches!(
        read(b"not a recording", |_| Some(key())),
        Err(RecordingError::Format)
    ));
}

#[test]
fn plain_text_drops_escape_sequences_and_controls() {
    assert_eq!(
        plain_text(
            "\x1b]0;title\x07a\x1b[1;31mb\x1b[0m\x1b(Bc\r\nd\x08e\x1b]8;;http://x\x1b\\f\tg"
        ),
        "abc\ndef\tg"
    );
}

/// The review of D75: a UTF-8 sequence cut at the end next to a full chunk
/// keeps the final chunk under 64 KiB; a write that fails stops the
/// recording and leaves the chunks before it readable.
#[test]
fn the_end_of_a_recording_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    for pad in 0..8 {
        let header = header();
        let path = dir.path().join(format!("{}.{EXTENSION}", header.file_id));
        let mut writer = RecordingWriter::create(&path, header, key(), (80, 24), 1).unwrap();
        // Lines that leave the buffer just under a full chunk, then the
        // first bytes of a character the session never finishes.
        let big = vec![b'y'; CHUNK - 120 + pad * 7];
        writer.output_at(0.0, &big).unwrap();
        writer.output_at(1.0, &"ж".as_bytes()[..1]).unwrap();
        writer.finish().unwrap();
        let recording = read(&std::fs::read(&path).unwrap(), |_| Some(key())).unwrap();
        assert!(recording.complete, "pad {pad}");
    }
    // A writer whose write failed writes nothing more, also on drop.
    let header = header();
    let path = dir.path().join(format!("{}.{EXTENSION}", header.file_id));
    let mut writer = RecordingWriter::create(&path, header, key(), (80, 24), 1).unwrap();
    writer
        .output_at(
            0.0, b"before
",
        )
        .unwrap();
    writer.output_at(0.5, &vec![b'z'; CHUNK + 10]).unwrap();
    writer.failed = true;
    assert!(writer.output_at(1.0, b"more").is_err());
    drop(writer);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), HEADER_LEN + FRAME);
    let recording = read(&bytes, |_| Some(key())).unwrap();
    assert!(!recording.complete);
    assert_eq!(
        recording.events[0],
        (
            0.0,
            "before
"
            .to_owned()
        )
    );
}

/// A recording stops at its size limit with a marker; the session's later
/// output is not written, and the file still closes and opens (section 12
/// review).
#[test]
fn a_recording_stops_at_its_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let header = header();
    let path = dir.path().join(format!("{}.{EXTENSION}", header.file_id));
    let mut writer = RecordingWriter::create(&path, header, key(), (80, 24), 1).unwrap();
    writer.max_chunks = 2;
    let block = vec![b'x'; CHUNK / 2];
    for second in 0..20 {
        writer.output_at(f64::from(second), &block).unwrap();
    }
    writer.finish().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    // Two full chunks, then the final one with what waited and the marker.
    assert!(bytes.len() <= HEADER_LEN + 3 * FRAME, "{}", bytes.len());
    let recording = read(&bytes, |_| Some(key())).unwrap();
    assert!(recording.complete);
    let written: usize = recording.events.iter().map(|(_, text)| text.len()).sum();
    assert!(written < 4 * CHUNK, "{written}");
    assert!(written >= 2 * CHUNK - 4096, "{written}");
}
