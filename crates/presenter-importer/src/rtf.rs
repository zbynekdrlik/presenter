//! RTF decoding for ProPresenter slide text.

use anyhow::Result;
use encoding_rs::{Encoding, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1254};

use crate::nfc;

pub fn decode_rtf(bytes: &[u8]) -> Result<String> {
    let raw = String::from_utf8_lossy(bytes).to_string();
    let encoding = detect_rtf_encoding(&raw);
    Ok(clean_text(fallback_rtf_to_text(&raw, encoding)))
}

fn detect_rtf_encoding(raw: &str) -> &'static Encoding {
    if let Some(idx) = raw.find("\\ansicpg") {
        let mut digits = String::new();
        for ch in raw[idx + 8..].chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
            } else {
                break;
            }
        }
        if let Ok(codepage) = digits.parse::<u32>() {
            return match codepage {
                1250 => WINDOWS_1250,
                1251 => WINDOWS_1251,
                1252 => WINDOWS_1252,
                1254 => WINDOWS_1254,
                _ => WINDOWS_1250,
            };
        }
    }
    WINDOWS_1250
}

fn fallback_rtf_to_text(raw: &str, encoding: &'static Encoding) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut unicode_skip = 1usize;

    while let Some(ch) = chars.next() {
        match ch {
            '{' | '}' => continue,
            '\\' => match chars.peek().copied() {
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('{') => {
                    chars.next();
                    result.push('{');
                }
                Some('}') => {
                    chars.next();
                    result.push('}');
                }
                Some('\'') => {
                    chars.next();
                    if let Some(decoded) = decode_rtf_hex(&mut chars, encoding) {
                        result.push(decoded);
                    }
                }
                Some('u') => {
                    chars.next(); // consume 'u'
                    match chars.peek() {
                        Some('-') | Some('0'..='9') => {
                            // Unicode escape: \u1234? or \u-3913?
                            if let Some(decoded) = decode_rtf_unicode(&mut chars) {
                                result.push(decoded);
                            }
                            skip_unicode_fallback(&mut chars, unicode_skip);
                            // Consume delimiter space (when uc=0, no fallback eats it)
                            if unicode_skip == 0 {
                                if let Some(' ') = chars.peek() {
                                    chars.next();
                                }
                            }
                        }
                        _ => {
                            // Control word starting with 'u': \up0, \ul, \ulnone, etc.
                            let word = read_control_word(&mut chars);
                            let full_word = format!("u{word}");
                            if full_word.starts_with("uc") {
                                unicode_skip = full_word
                                    .trim_start_matches("uc")
                                    .parse::<usize>()
                                    .unwrap_or(1);
                            }
                            if let Some(' ') = chars.peek() {
                                chars.next();
                            }
                        }
                    }
                }
                Some(_) => {
                    let word = read_control_word(&mut chars);
                    match word.as_str() {
                        "par" | "line" => result.push('\n'),
                        word if word.starts_with("uc") => {
                            unicode_skip =
                                word.trim_start_matches("uc").parse::<usize>().unwrap_or(1);
                        }
                        _ => {}
                    }
                    if let Some(' ') = chars.peek() {
                        chars.next();
                    }
                }
                None => {}
            },
            _ => result.push(ch),
        }
    }

    result
}

fn decode_rtf_hex<I>(chars: &mut I, encoding: &'static Encoding) -> Option<char>
where
    I: Iterator<Item = char>,
{
    let mut hex = String::new();
    for _ in 0..2 {
        if let Some(c) = chars.next() {
            hex.push(c);
        } else {
            return None;
        }
    }
    let byte = u8::from_str_radix(&hex, 16).ok()?;
    let binding = [byte];
    let (converted, _, had_errors) = encoding.decode(&binding);
    if had_errors {
        Some(byte as char)
    } else {
        converted.chars().next()
    }
}

fn decode_rtf_unicode<I>(chars: &mut std::iter::Peekable<I>) -> Option<char>
where
    I: Iterator<Item = char>,
{
    let mut numeric = String::new();
    if let Some('-') = chars.peek().copied() {
        numeric.push('-');
        chars.next();
    }
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            numeric.push(c);
            chars.next();
        } else {
            break;
        }
    }
    let value = numeric.parse::<i32>().ok()?;
    if value >= 0 {
        char::from_u32(value as u32)
    } else {
        char::from_u32((65536 + value) as u32)
    }
}

fn skip_unicode_fallback<I>(chars: &mut std::iter::Peekable<I>, count: usize)
where
    I: Iterator<Item = char>,
{
    for _ in 0..count {
        if !consume_fallback_char(chars) {
            break;
        }
    }
    while matches!(chars.peek(), Some('?')) {
        chars.next();
    }
}

fn consume_fallback_char<I>(chars: &mut std::iter::Peekable<I>) -> bool
where
    I: Iterator<Item = char>,
{
    match chars.peek().copied() {
        Some('\\') => {
            chars.next();
            match chars.peek().copied() {
                Some('\'') => {
                    chars.next();
                    for _ in 0..2 {
                        if chars.next().is_none() {
                            break;
                        }
                    }
                    true
                }
                Some(_) => {
                    let _ = read_control_word(chars);
                    true
                }
                None => false,
            }
        }
        Some(_) => {
            chars.next();
            true
        }
        None => false,
    }
}

fn read_control_word<I>(chars: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = char>,
{
    let mut word = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    while let Some(&c) = chars.peek() {
        if c == '-' || c.is_ascii_digit() {
            word.push(c);
            chars.next();
        } else {
            break;
        }
    }
    word
}

fn clean_text(text: String) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut prev_alpha = false;
    let mut prev_boundary = true;
    let mut prev_lower = false;
    let mut iter = text.chars().peekable();
    while let Some(ch) = iter.next() {
        if ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t' {
            continue;
        }
        if is_formatting_char(ch) {
            continue;
        }
        if ch == '\u{00A0}' {
            cleaned.push(' ');
            prev_alpha = false;
            prev_boundary = true;
            prev_lower = false;
            continue;
        }
        if ch == '0' {
            if let Some(&next) = iter.peek() {
                if next > '\u{007F}' && next.is_alphabetic() && (prev_alpha || prev_boundary) {
                    continue;
                }
            }
        }
        let mut output = ch;
        if ch.is_uppercase() && prev_lower {
            let mut lower = ch.to_lowercase();
            if let Some(first) = lower.next() {
                if lower.next().is_none() {
                    output = first;
                }
            }
        }
        prev_alpha = output.is_alphabetic();
        prev_lower = output.is_lowercase();
        prev_boundary = output.is_whitespace()
            || output.is_ascii_punctuation()
            || matches!(output, '\n' | '\r');
        cleaned.push(output);
    }
    nfc::to_nfc(&strip_header_lines(cleaned))
}

fn is_formatting_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'
            | '\u{202B}'
            | '\u{202C}'
            | '\u{202D}'
            | '\u{202E}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
            | '\u{00AD}'
    )
}

fn strip_header_lines(input: String) -> String {
    let mut lines = Vec::new();
    let mut seen_content = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if !seen_content {
            if trimmed.is_empty() || is_header_line(trimmed) {
                continue;
            }
            seen_content = true;
        }
        lines.push(line);
    }
    if !seen_content {
        return String::new();
    }
    lines.join("\n")
}

fn is_header_line(line: &str) -> bool {
    if line.is_empty() {
        return true;
    }
    if line.chars().all(|c| matches!(c, ';' | '*' | '-')) {
        return true;
    }
    if line.ends_with(';') && !line.contains(' ') {
        let body = &line[..line.len() - 1];
        if !body.is_empty()
            && body
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_rtf_handles_slovak_characters() {
        let raw = br"{\rtf1\ansi\ansicpg1250 Slov\'e1k\par}";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(text.contains("Slovák"));
    }

    #[test]
    fn decode_rtf_fallback_handles_hex_and_unicode() {
        let raw = br"Slov\'e1k \u353?peci\'e1l";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(text.contains("Slovák"));
        assert!(text.contains("špeciál"));
    }

    #[test]
    fn decode_rtf_retains_central_european_letters() {
        let raw = br"{\rtf1\ansi\ansicpg1252 Ka\'9ed\'fd dobr\'fd \u250\'fadel a ka\'9ed\'fd dokonal\'fd dar poch\'e1dza zhora od Otca svetiel, v ktorom niet premeny ani tie\uc0\u328 a zmeny.}";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(text.contains("Každý dobrý údel"), "decoded={text}");
        assert!(
            text.contains("niet premeny ani tieňa zmeny"),
            "decoded={text}"
        );
    }

    #[test]
    fn clean_text_drops_spurious_zero_before_caron() {
        let sample = "Monaco;\n\nA niet v 0ňom žiadne klamstvá";
        let cleaned = clean_text(sample.to_string());
        assert!(cleaned.contains("A niet v ňom"), "cleaned={cleaned}");
        assert!(!cleaned.contains("0ň"), "cleaned={cleaned}");
    }

    #[test]
    fn clean_text_strips_leading_font_headers() {
        let sample = "Monaco;\n;;;\n*;;;\n\n\nA kto to zmeniť chce musí sa znova narodiť";
        let cleaned = clean_text(sample.to_string());
        assert_eq!(
            cleaned, "A kto to zmeniť chce musí sa znova narodiť",
            "cleaned={cleaned}"
        );
    }

    #[test]
    fn decode_rtf_consecutive_hex_escapes_preserve_both_chars() {
        // \'cd = Í (0xCD in cp1252), \'8e = Ž (0x8E in cp1252)
        let raw = br"{\rtf1\ansi\ansicpg1252 PRIBL\'cd\'8eIM SA K TEBE VIAC}";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(
            text.contains("PRIBLÍŽIM"),
            "expected PRIBLÍŽIM but got: {text}"
        );
    }

    #[test]
    fn decode_rtf_cp1252_uppercase_and_lowercase_caron_z() {
        // \'8e = Ž, \'9e = ž in Windows-1252
        let upper = br"{\rtf1\ansi\ansicpg1252 \'8eIVOT}";
        let lower = br"{\rtf1\ansi\ansicpg1252 t\'fa\'9ebu}";
        let text_upper = decode_rtf(upper).expect("rtf to decode");
        let text_lower = decode_rtf(lower).expect("rtf to decode");
        assert!(text_upper.contains('Ž'), "expected Ž but got: {text_upper}");
        assert!(text_lower.contains('ž'), "expected ž but got: {text_lower}");
    }

    #[test]
    fn decode_rtf_full_rtf_header_with_consecutive_hex() {
        let raw = br"{\rtf1\ansi\ansicpg1252\cocoartf2639
\cocoatextscaling0\cocoaplatform0{\fonttbl\f0\fnil\fcharset0 HelveticaNeue;}
{\colortbl;\red255\green255\blue255;}
{\*\expandedcolortbl;;}
\pard\tx560\tx1120\pardirnatural\qc\partightenfactor0

\f0\fs120 \cf1 PRIBL\'cd\'8eIM SA K TEBE VIAC}";
        let text = decode_rtf(raw).expect("rtf to decode");
        assert!(
            text.contains("PRIBLÍŽIM"),
            "expected PRIBLÍŽIM in full RTF but got: {text}"
        );
    }

    #[test]
    fn fallback_rtf_up0_not_misinterpreted_as_unicode() {
        let raw = r"{\rtf1\ansi\ansicpg1250\f0\fs166 \cf2 \up0 MILUJEM TA BOŽE MÔJ}";
        let text = decode_rtf(raw.as_bytes()).expect("rtf to decode");
        assert!(
            !text.starts_with('0'),
            "should not start with '0', got: {text}"
        );
        assert!(
            text.contains("MILUJEM"),
            "should contain MILUJEM, got: {text}"
        );
    }

    #[test]
    fn fallback_rtf_ulnone_not_misinterpreted() {
        let raw = r"{\rtf1\ansi\ansicpg1250 \ulnone Hello}";
        let text = decode_rtf(raw.as_bytes()).expect("rtf to decode");
        assert!(
            !text.contains("lnone"),
            "should not leak 'lnone', got: {text}"
        );
        assert!(text.contains("Hello"), "should contain Hello, got: {text}");
    }

    #[test]
    fn fallback_rtf_unicode_escape_still_works() {
        let raw = r"{\rtf1\ansi\ansicpg1250 \u353?peci\u225?l}";
        let text = decode_rtf(raw.as_bytes()).expect("rtf to decode");
        assert!(
            text.contains("špeciál"),
            "unicode escapes broken, got: {text}"
        );
    }

    #[test]
    fn fallback_rtf_negative_unicode_still_works() {
        let raw = r"{\rtf1\ansi\ansicpg1250 \u-3913?x}";
        let text = decode_rtf(raw.as_bytes()).expect("rtf to decode");
        assert!(!text.is_empty(), "should produce output");
    }

    #[test]
    fn clean_text_normalizes_nfd_to_nfc() {
        // ProPresenter Mac source produces NFD: 'z' + U+030C, 'i' + U+0301.
        let input = String::from("Po Tebe Pane z\u{30c}i\u{301}znim");
        let cleaned = clean_text(input);
        // Expect single-codepoint 'ž' and 'í' in the output.
        assert_eq!(cleaned, "Po Tebe Pane \u{17e}\u{ed}znim");
    }
}
