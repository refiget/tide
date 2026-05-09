use crossterm::style::Color;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyledText {
    pub spans: Vec<StyledSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyledSpan {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl StyledText {
    pub fn plain(text: impl Into<String>) -> Self {
        let t = text.into();
        if t.is_empty() {
            return Self::default();
        }
        Self {
            spans: vec![StyledSpan {
                text: t,
                style: TextStyle::default(),
            }],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| s.text.is_empty())
    }
}

pub fn styled_width(text: &StyledText) -> usize {
    use unicode_width::UnicodeWidthStr;
    text.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.text.as_str()))
        .sum()
}

pub fn styled_to_plain(text: &StyledText) -> String {
    text.spans.iter().map(|s| s.text.as_str()).collect()
}

pub fn truncate_styled_to_width(text: &StyledText, max_width: usize) -> StyledText {
    use unicode_width::UnicodeWidthChar;
    let mut remaining = max_width;
    let mut spans = Vec::new();
    for span in &text.spans {
        if remaining == 0 {
            break;
        }
        let mut truncated = String::new();
        for ch in span.text.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if w > remaining {
                break;
            }
            truncated.push(ch);
            remaining -= w;
        }
        if !truncated.is_empty() {
            spans.push(StyledSpan {
                text: truncated,
                style: span.style.clone(),
            });
        }
    }
    StyledText { spans }
}

/// Parse raw PTY bytes into per-line styled text.
/// Only SGR (color/style) sequences are interpreted; all other control
/// sequences (cursor movement, erase, OSC, alternate screen) are consumed
/// and discarded — they never appear as literal text in the output.
/// Style state carries across newlines until a reset is encountered.
pub fn parse_ansi_lines(bytes: &[u8]) -> Vec<StyledText> {
    let mut lines: Vec<StyledText> = Vec::new();
    let mut current_style = TextStyle::default();
    let mut current_spans: Vec<StyledSpan> = Vec::new();
    let mut current_text = String::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'\x1b' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    // CSI sequence
                    i += 1;
                    let start = i;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        break;
                    }
                    let final_byte = bytes[i];
                    let payload = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                    i += 1;
                    if final_byte == b'm' {
                        flush_span(&mut current_text, &current_style, &mut current_spans);
                        apply_sgr(payload, &mut current_style);
                    }
                    // Other CSI (cursor movement, erase, etc.): consumed, not output
                }
                b']' => {
                    // OSC: consume until BEL (\x07) or ST (\x1b\\)
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\x07' {
                            i += 1;
                            break;
                        }
                        if bytes[i] == b'\x1b' && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => {
                    i += 1; // other escape: skip
                }
            }
        } else if b == b'\n' {
            flush_span(&mut current_text, &current_style, &mut current_spans);
            lines.push(StyledText {
                spans: std::mem::take(&mut current_spans),
            });
            i += 1;
        } else if b == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                // \r\n: PTY ONLCR line ending — skip the \r, let \n handle the flush
                i += 1;
            } else {
                // Bare \r: carriage-return overwrite (progress bars) — clear current line
                current_text.clear();
                current_spans.clear();
                i += 1;
            }
        } else if b.is_ascii_control() {
            i += 1; // skip other control chars
        } else {
            let ch_len = utf8_char_len(b);
            if i + ch_len <= bytes.len() {
                if let Ok(s) = std::str::from_utf8(&bytes[i..i + ch_len]) {
                    current_text.push_str(s);
                }
            }
            i += ch_len;
        }
    }

    // Flush any trailing content (no final \n)
    flush_span(&mut current_text, &current_style, &mut current_spans);
    if !current_spans.is_empty() {
        lines.push(StyledText {
            spans: current_spans,
        });
    }

    lines
}

fn flush_span(text: &mut String, style: &TextStyle, spans: &mut Vec<StyledSpan>) {
    if !text.is_empty() {
        spans.push(StyledSpan {
            text: std::mem::take(text),
            style: style.clone(),
        });
    }
}

fn utf8_char_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

fn apply_sgr(payload: &str, style: &mut TextStyle) {
    let parts: Vec<&str> = payload.split(';').collect();
    let mut idx = 0;
    while idx < parts.len() {
        let n: u16 = parts[idx].parse().unwrap_or(0);
        match n {
            0 => *style = TextStyle::default(),
            1 => style.bold = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.reverse = true,
            22 => style.bold = false,
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.reverse = false,
            30..=37 => style.fg = Some(ansi_color(n - 30, false)),
            38 => {
                if let Some(c) = parse_extended_color(&parts, &mut idx) {
                    style.fg = Some(c);
                }
            }
            39 => style.fg = None,
            40..=47 => style.bg = Some(ansi_color(n - 40, false)),
            48 => {
                if let Some(c) = parse_extended_color(&parts, &mut idx) {
                    style.bg = Some(c);
                }
            }
            49 => style.bg = None,
            90..=97 => style.fg = Some(ansi_color(n - 90, true)),
            100..=107 => style.bg = Some(ansi_color(n - 100, true)),
            _ => {}
        }
        idx += 1;
    }
}

fn ansi_color(n: u16, bright: bool) -> Color {
    match (n, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::DarkRed,
        (2, false) => Color::DarkGreen,
        (3, false) => Color::DarkYellow,
        (4, false) => Color::DarkBlue,
        (5, false) => Color::DarkMagenta,
        (6, false) => Color::DarkCyan,
        (7, false) => Color::Grey,
        (0, true) => Color::DarkGrey,
        (1, true) => Color::Red,
        (2, true) => Color::Green,
        (3, true) => Color::Yellow,
        (4, true) => Color::Blue,
        (5, true) => Color::Magenta,
        (6, true) => Color::Cyan,
        (7, true) => Color::White,
        _ => Color::Reset,
    }
}

fn parse_extended_color(parts: &[&str], idx: &mut usize) -> Option<Color> {
    match parts.get(*idx + 1).and_then(|s| s.parse::<u16>().ok()) {
        Some(5) => {
            let n = parts.get(*idx + 2)?.parse::<u8>().ok()?;
            *idx += 2;
            Some(Color::AnsiValue(n))
        }
        Some(2) => {
            let r = parts.get(*idx + 2)?.parse::<u8>().ok()?;
            let g = parts.get(*idx + 3)?.parse::<u8>().ok()?;
            let b = parts.get(*idx + 4)?.parse::<u8>().ok()?;
            *idx += 4;
            Some(Color::Rgb { r, g, b })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_red() {
        let input = b"\x1b[31mhello\x1b[0m";
        let lines = parse_ansi_lines(input);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].text, "hello");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::DarkRed));
    }

    #[test]
    fn parse_bold_reset() {
        let input = b"\x1b[1mbold\x1b[0m normal";
        let lines = parse_ansi_lines(input);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert!(spans[0].style.bold);
        assert!(!spans[1].style.bold);
    }

    #[test]
    fn parse_256_color() {
        let input = b"\x1b[38;5;196mred\x1b[0m";
        let lines = parse_ansi_lines(input);
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::AnsiValue(196)));
    }

    #[test]
    fn parse_truecolor() {
        let input = b"\x1b[38;2;255;128;0morange\x1b[0m";
        let lines = parse_ansi_lines(input);
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(Color::Rgb {
                r: 255,
                g: 128,
                b: 0
            })
        );
    }

    #[test]
    fn osc_ignored() {
        let input = b"\x1b]0;terminal title\x07hello";
        let lines = parse_ansi_lines(input);
        assert_eq!(lines.len(), 1);
        assert_eq!(styled_to_plain(&lines[0]), "hello");
    }

    #[test]
    fn cursor_movement_ignored() {
        let input = b"before\x1b[5Cafter";
        let lines = parse_ansi_lines(input);
        assert_eq!(styled_to_plain(&lines[0]), "beforeafter");
    }

    #[test]
    fn multiline_style_continues() {
        let input = b"\x1b[31mred\nstill red\x1b[0m";
        let lines = parse_ansi_lines(input);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::DarkRed));
    }

    #[test]
    fn cr_clears_line() {
        let input = b"Loading...\rDone";
        let lines = parse_ansi_lines(input);
        assert_eq!(styled_to_plain(&lines[0]), "Done");
    }

    #[test]
    fn crlf_preserves_content() {
        let input = b"hello\r\nworld\r\n";
        let lines = parse_ansi_lines(input);
        assert_eq!(lines.len(), 2);
        assert_eq!(styled_to_plain(&lines[0]), "hello");
        assert_eq!(styled_to_plain(&lines[1]), "world");
    }

    #[test]
    fn styled_width_ignores_style() {
        let text = StyledText {
            spans: vec![
                StyledSpan {
                    text: "hello".into(),
                    style: TextStyle {
                        bold: true,
                        ..Default::default()
                    },
                },
                StyledSpan {
                    text: " world".into(),
                    style: TextStyle::default(),
                },
            ],
        };
        assert_eq!(styled_width(&text), 11);
    }

    #[test]
    fn truncate_styled_unicode_width() {
        let text = StyledText::plain("hello world");
        let truncated = truncate_styled_to_width(&text, 5);
        assert_eq!(styled_to_plain(&truncated), "hello");
    }

    #[test]
    fn truncate_styled_preserves_style() {
        let text = StyledText {
            spans: vec![
                StyledSpan {
                    text: "hello".into(),
                    style: TextStyle {
                        bold: true,
                        ..Default::default()
                    },
                },
                StyledSpan {
                    text: " world".into(),
                    style: TextStyle::default(),
                },
            ],
        };
        let truncated = truncate_styled_to_width(&text, 7);
        assert_eq!(truncated.spans.len(), 2);
        assert_eq!(truncated.spans[0].text, "hello");
        assert_eq!(truncated.spans[1].text, " w");
        assert!(truncated.spans[0].style.bold);
    }

    #[test]
    fn empty_bytes_returns_empty() {
        let lines = parse_ansi_lines(b"");
        assert!(lines.is_empty());
    }

    #[test]
    fn bare_sgr_reset_is_handled() {
        // ESC[m with no number is a reset
        let input = b"\x1b[31mred\x1b[mnormal";
        let lines = parse_ansi_lines(input);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].style.fg, Some(Color::DarkRed));
        assert_eq!(spans[1].style.fg, None);
    }
}
