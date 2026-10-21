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
    // A final chunk cut short, or a non-final chunk dropped.
    assert!(read(&bytes[..bytes.len() - 5], |_| Some(key())).is_err());
    let mut dropped = bytes[..HEADER_LEN].to_vec();
    dropped.extend_from_slice(&bytes[HEADER_LEN + FRAME..]);
    assert!(read(&dropped, |_| Some(key())).is_err());
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
