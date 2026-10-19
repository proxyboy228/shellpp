//! Remote charset of SSH sessions (host `charset`, UI 3.1; stage 2 plan,
//! decision 5): output is decoded from the host's charset to UTF-8 before it
//! reaches xterm.js, typed input is encoded back. `utf-8` (the default) passes
//! bytes through unchanged: xterm.js decodes UTF-8 itself, also across chunk
//! boundaries. Characters the remote charset cannot represent become `?`.
//! Binary input (xterm.js `onBinary`) bypasses the encoder.

use encoding_rs::{CoderResult, Decoder, Encoder, EncoderResult, Encoding, UTF_8};

/// The encoding for a WHATWG label; unknown labels, every UTF-16 variant (a
/// terminal byte stream is never UTF-16) and ISO-2022-JP fall back to
/// UTF-8. ISO-2022-JP switches character sets with ESC sequences: its
/// decoder turns every other ESC (all CSI and OSC output) into U+FFFD and
/// its encoder cannot encode ESC, so no key with an escape sequence would
/// reach the host.
pub fn encoding_for(label: &str) -> &'static Encoding {
    match Encoding::for_label(label.trim().as_bytes()) {
        Some(encoding)
            if encoding.output_encoding() == encoding && encoding != encoding_rs::ISO_2022_JP =>
        {
            encoding
        }
        _ => UTF_8,
    }
}

/// Streaming decoder of session output.
pub struct OutputDecoder {
    decoder: Option<Decoder>,
}

impl std::fmt::Debug for OutputDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputDecoder")
            .field("identity", &self.decoder.is_none())
            .finish()
    }
}

impl OutputDecoder {
    pub fn new(encoding: &'static Encoding) -> Self {
        let decoder = (encoding != UTF_8).then(|| encoding.new_decoder_without_bom_handling());
        Self { decoder }
    }

    /// Decodes one chunk to UTF-8. Incomplete multi-byte sequences at the end
    /// of `input` are kept for the next chunk.
    pub fn decode(&mut self, input: Vec<u8>) -> Vec<u8> {
        let Some(decoder) = self.decoder.as_mut() else {
            return input;
        };
        let mut out = String::with_capacity(
            decoder
                .max_utf8_buffer_length(input.len())
                .unwrap_or(input.len().saturating_mul(3)),
        );
        let mut rest = &input[..];
        loop {
            let (result, read, _replaced) = decoder.decode_to_string(rest, &mut out, false);
            rest = &rest[read..];
            match result {
                CoderResult::InputEmpty => break,
                CoderResult::OutputFull => out.reserve(rest.len().saturating_mul(3).max(16)),
            }
        }
        out.into_bytes()
    }
}

/// Encoder of typed input.
pub struct InputEncoder {
    encoder: Option<Encoder>,
}

impl std::fmt::Debug for InputEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputEncoder")
            .field("identity", &self.encoder.is_none())
            .finish()
    }
}

impl InputEncoder {
    pub fn new(encoding: &'static Encoding) -> Self {
        let encoder = (encoding != UTF_8).then(|| encoding.new_encoder());
        Self { encoder }
    }

    /// Encodes UTF-8 text into the remote charset; unmappable characters become `?`.
    pub fn encode(&mut self, text: &str) -> Vec<u8> {
        let Some(encoder) = self.encoder.as_mut() else {
            return text.as_bytes().to_vec();
        };
        let mut out = Vec::with_capacity(
            encoder
                .max_buffer_length_from_utf8_without_replacement(text.len())
                .unwrap_or(text.len().saturating_mul(4))
                .max(8),
        );
        let mut rest = text;
        loop {
            let (result, read) =
                encoder.encode_from_utf8_to_vec_without_replacement(rest, &mut out, false);
            rest = &rest[read..];
            match result {
                EncoderResult::InputEmpty => break,
                EncoderResult::Unmappable(_) => out.push(b'?'),
                EncoderResult::OutputFull => out.reserve(rest.len().saturating_mul(4).max(16)),
            }
        }
        out
    }
}

/// Bytes of xterm.js `onBinary` data: every UTF-16 code unit is one byte.
/// `None` when a code unit is above 0xFF.
pub fn binary_bytes(data: &str) -> Option<Vec<u8>> {
    data.encode_utf16()
        .map(|unit| u8::try_from(unit).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_identity() {
        let encoding = encoding_for("utf-8");
        assert_eq!(encoding, UTF_8);
        let mut decoder = OutputDecoder::new(encoding);
        let bytes = "wide: \u{65e5}\u{672c} ok".as_bytes().to_vec();
        assert_eq!(decoder.decode(bytes.clone()), bytes);
        // Split sequences pass through unchanged as well.
        assert_eq!(decoder.decode(vec![0xe6]), vec![0xe6]);
        let mut encoder = InputEncoder::new(encoding);
        assert_eq!(encoder.encode("\u{65e5}"), "\u{65e5}".as_bytes());
    }

    #[test]
    fn labels() {
        assert_eq!(encoding_for("UTF8"), UTF_8);
        assert_eq!(encoding_for(""), UTF_8);
        assert_eq!(encoding_for("no-such-charset"), UTF_8);
        assert_eq!(encoding_for("utf-16le"), UTF_8);
        assert_eq!(encoding_for("iso-2022-jp"), UTF_8);
        assert_eq!(encoding_for("csISO2022JP"), UTF_8);
        assert_eq!(encoding_for("iso-2022-kr"), UTF_8);
        assert_eq!(encoding_for("euc-jp"), encoding_rs::EUC_JP);
        assert_eq!(encoding_for("windows-1251"), encoding_rs::WINDOWS_1251);
        assert_eq!(encoding_for("koi8-r"), encoding_rs::KOI8_R);
        assert_eq!(encoding_for(" latin1 "), encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn windows_1251_round_trip() {
        let encoding = encoding_for("windows-1251");
        let mut encoder = InputEncoder::new(encoding);
        let text = "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} ok";
        let encoded = encoder.encode(text);
        assert_eq!(
            encoded,
            vec![0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2, b' ', b'o', b'k']
        );
        let mut decoder = OutputDecoder::new(encoding);
        assert_eq!(decoder.decode(encoded), text.as_bytes());
    }

    #[test]
    fn unmappable_input_becomes_question_marks() {
        let mut encoder = InputEncoder::new(encoding_for("windows-1251"));
        assert_eq!(encoder.encode("a\u{65e5}b\u{1f44d}"), b"a?b?".to_vec());
    }

    #[test]
    fn multi_byte_sequences_split_across_chunks() {
        let encoding = encoding_for("shift_jis");
        let (bytes, _, _) = encoding.encode("\u{65e5}\u{672c}");
        let bytes = bytes.into_owned();
        assert_eq!(bytes.len(), 4);
        let mut decoder = OutputDecoder::new(encoding);
        let mut out = decoder.decode(bytes[..1].to_vec());
        out.extend(decoder.decode(bytes[1..3].to_vec()));
        out.extend(decoder.decode(bytes[3..].to_vec()));
        assert_eq!(out, "\u{65e5}\u{672c}".as_bytes());
    }

    #[test]
    fn binary_data() {
        assert_eq!(
            binary_bytes("\u{1b}[M \u{ff}"),
            Some(vec![0x1b, b'[', b'M', b' ', 0xff])
        );
        assert_eq!(binary_bytes("\u{100}"), None);
        assert_eq!(binary_bytes(""), Some(vec![]));
    }
}
