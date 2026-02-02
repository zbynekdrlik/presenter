//! RTF decoding for ProPresenter slide text.

use anyhow::Result;
use encoding_rs::{Encoding, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1254};
use rtf_parser::RtfDocument;

pub fn decode_rtf(bytes: &[u8]) -> Result<String> {
    let raw = String::from_utf8_lossy(bytes).to_string();
    let encoding = detect_rtf_encoding(&raw);

    if let Ok(doc) = RtfDocument::try_from(raw.clone()) {
        let text = doc.get_text();
        if !contains_control_range(&text) {
            return Ok(clean_text(text));
        }
    }

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
                    chars.next();
                    if let Some(decoded) = decode_rtf_unicode(&mut chars) {
                        result.push(decoded);
                    }
                    skip_unicode_fallback(&mut chars, unicode_skip);
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

fn contains_control_range(text: &str) -> bool {
    text.chars().any(|ch| matches!(ch as u32, 0x80..=0x9F))
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
    strip_header_lines(cleaned)
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
}
