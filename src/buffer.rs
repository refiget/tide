use crate::app::BlockId;

#[derive(Debug, Clone, Default)]
pub struct ShellBuffer {
    pub lines: Vec<ShellLine>,
    current_line: String,
    current_col: usize,
    current_line_chars: usize,
    current_block_id: Option<BlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellLine {
    pub text: String,
    pub block_id: Option<BlockId>,
}

impl ShellBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, bytes: &[u8], block_id: Option<BlockId>) {
        self.current_block_id = block_id;
        let text = String::from_utf8_lossy(bytes);
        let mut chars = text.chars().peekable();

        while let Some(ch) = chars.next() {
            match ch {
                '\x1b' => self.apply_escape_sequence(&mut chars),
                '\r' => {
                    self.current_col = 0;
                }
                '\n' => {
                    self.push_current_line();
                }
                '\x08' => {
                    self.backspace();
                }
                '\t' => {
                    for _ in 0..4 {
                        self.put_char(' ');
                    }
                }
                ch if ch.is_control() => {}
                ch => self.put_char(ch),
            }
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn snapshot(&self) -> Vec<ShellLine> {
        let mut lines = self.lines.clone();
        if !self.current_line.is_empty() {
            lines.push(ShellLine {
                text: self.current_line.trim_end().to_string(),
                block_id: self.current_block_id,
            });
        }
        lines
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        (self.lines.len(), self.current_col)
    }

    fn push_current_line(&mut self) {
        self.lines.push(ShellLine {
            text: std::mem::take(&mut self.current_line)
                .trim_end()
                .to_string(),
            block_id: self.current_block_id,
        });
        self.current_col = 0;
        self.current_line_chars = 0;
    }

    fn put_char(&mut self, ch: char) {
        if self.current_col == self.current_line_chars {
            self.current_line.push(ch);
            self.current_col += 1;
            self.current_line_chars += 1;
            return;
        }

        if self.current_col > self.current_line_chars {
            let padding = self.current_col - self.current_line_chars;
            self.current_line.push_str(&" ".repeat(padding));
            self.current_line.push(ch);
            self.current_col += 1;
            self.current_line_chars += padding + 1;
            return;
        }

        if let Some((byte_offset, old_ch)) = self.current_line.char_indices().nth(self.current_col) {
            self.current_line.replace_range(byte_offset..byte_offset + old_ch.len_utf8(), &ch.to_string());
        }
        self.current_col += 1;
    }

    fn backspace(&mut self) {
        if self.current_col == 0 {
            return;
        }

        self.current_col -= 1;
        if let Some((byte_offset, _)) = self.current_line.char_indices().nth(self.current_col) {
            self.current_line.remove(byte_offset);
            self.current_line_chars = self.current_line_chars.saturating_sub(1);
        }
    }

    fn apply_escape_sequence(&mut self, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
        let Some(first) = chars.next() else {
            return;
        };

        if first == ']' {
            for ch in chars.by_ref() {
                if ch == '\x07' {
                    break;
                }
            }
            return;
        }

        if first != '[' {
            return;
        }

        let mut payload = String::new();
        let mut final_byte = None;
        for ch in chars.by_ref() {
            if ('@'..='~').contains(&ch) {
                final_byte = Some(ch);
                break;
            }
            payload.push(ch);
        }

        self.apply_csi(payload.as_str(), final_byte);
    }

    fn apply_csi(&mut self, payload: &str, final_byte: Option<char>) {
        let first_number = payload
            .split(';')
            .next()
            .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
            .and_then(|value| value.parse::<usize>().ok());

        match final_byte {
            Some('C') => {
                self.current_col += first_number.unwrap_or(1);
            }
            Some('D') => {
                self.current_col = self.current_col.saturating_sub(first_number.unwrap_or(1));
            }
            Some('G') => {
                self.current_col = first_number.unwrap_or(1).saturating_sub(1);
            }
            Some('K') => match first_number.unwrap_or(0) {
                0 => self.truncate_current_line_at_cursor(),
                1 => {
                    self.current_line = self
                        .current_line
                        .chars()
                        .skip(self.current_col)
                        .collect::<String>();
                    self.current_col = 0;
                    self.current_line_chars = self.current_line.chars().count();
                }
                2 => {
                    self.current_line.clear();
                    self.current_col = 0;
                    self.current_line_chars = 0;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn truncate_current_line_at_cursor(&mut self) {
        self.current_line = self
            .current_line
            .chars()
            .take(self.current_col)
            .collect::<String>();
        self.current_line_chars = self.current_line.chars().count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_char_append() {
        let mut buffer = ShellBuffer::new();
        buffer.put_char('a');
        buffer.put_char('b');
        assert_eq!(buffer.current_line, "ab");
        assert_eq!(buffer.current_col, 2);
        assert_eq!(buffer.current_line_chars, 2);
    }

    #[test]
    fn test_put_char_replace() {
        let mut buffer = ShellBuffer::new();
        buffer.put_char('a');
        buffer.put_char('b');
        buffer.put_char('c');
        buffer.current_col = 1;
        buffer.put_char('x');
        assert_eq!(buffer.current_line, "axc");
        assert_eq!(buffer.current_col, 2);
        assert_eq!(buffer.current_line_chars, 3);
    }

    #[test]
    fn test_put_char_multibyte() {
        let mut buffer = ShellBuffer::new();
        buffer.put_char('🦀');
        for ch in " Ferris".chars() {
            buffer.put_char(ch);
        }
        buffer.current_col = 0;
        buffer.put_char('🦞');
        assert_eq!(buffer.current_line, "🦞 Ferris");
        assert_eq!(buffer.current_col, 1);
        assert_eq!(buffer.current_line_chars, 8); // 🦀 + " Ferris" = 1 + 7 = 8 chars

        buffer.current_col = 1;
        buffer.put_char('!');
        assert_eq!(buffer.current_line, "🦞!Ferris");
    }

    #[test]
    fn test_backspace() {
        let mut buffer = ShellBuffer::new();
        buffer.put_char('a');
        buffer.put_char('b');
        buffer.put_char('c');
        buffer.backspace();
        assert_eq!(buffer.current_line, "ab");
        assert_eq!(buffer.current_col, 2);
        assert_eq!(buffer.current_line_chars, 2);

        buffer.current_col = 1;
        buffer.backspace();
        assert_eq!(buffer.current_line, "b");
        assert_eq!(buffer.current_col, 0);
        assert_eq!(buffer.current_line_chars, 1);
    }

    #[test]
    fn test_backspace_multibyte() {
        let mut buffer = ShellBuffer::new();
        buffer.put_char('🦀');
        buffer.put_char('🦞');
        buffer.backspace();
        assert_eq!(buffer.current_line, "🦀");
        assert_eq!(buffer.current_col, 1);
        assert_eq!(buffer.current_line_chars, 1);
    }
}
