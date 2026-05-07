use std::io::{self, Write};

/// Renders a vt100 terminal grid to the real terminal.
///
/// Uses diff-based rendering: only writes cells that changed since the last
/// frame. Maintains a scrollback-accessible history of all terminal output.
pub struct TermRenderer {
    parser: vt100::Parser,
    cols: u16,
    rows: u16,
    last_rendered: Vec<Vec<CellSnapshot>>,
}

#[derive(Clone, PartialEq, Eq)]
struct CellSnapshot {
    contents: String,
}

impl CellSnapshot {
    fn empty() -> Self {
        Self {
            contents: " ".to_string(),
        }
    }
}

impl TermRenderer {
    pub fn new(rows: u16, cols: u16) -> Self {
        let parser = vt100::Parser::new(rows, cols, 0);
        let last_rendered = build_empty_snapshot(rows, cols);

        Self {
            parser,
            cols,
            rows,
            last_rendered,
        }
    }

    /// Feed raw bytes from the PTY into the terminal parser.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Render the current visible viewport to the writer.
    ///
    /// Compares the current screen state against the last rendered frame and
    /// writes only changed cells. Only renders the visible portion (last N rows).
    pub fn render<W: Write>(&mut self, w: &mut W) -> io::Result<()> {
        let screen = self.parser.screen();

        for screen_row in 0..self.rows {
            let abs_row = screen.scrollback() as u16 + screen_row;
            for col in 0..self.cols {
                let cell = screen.cell(abs_row, col);
                let contents = cell
                    .map(|c| c.contents().to_string())
                    .unwrap_or_else(|| " ".to_string());

                if self.last_rendered[screen_row as usize][col as usize].contents != contents {
                    write!(
                        w,
                        "\x1b[{};{}H{}",
                        screen_row + 1,
                        col + 1,
                        contents
                    )?;
                    self.last_rendered[screen_row as usize][col as usize] =
                        CellSnapshot { contents };
                }
            }
        }

        w.flush()
    }

    /// Force a full redraw on the next render call.
    pub fn mark_dirty(&mut self) {
        self.last_rendered = build_empty_snapshot(self.rows, self.cols);
    }

    /// Resize the terminal grid. Called on SIGWINCH.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(rows, cols);
        self.rows = rows;
        self.cols = cols;
        self.last_rendered = build_empty_snapshot(rows, cols);
    }

    /// Total number of lines in the grid, including scrollback.
    pub fn total_lines(&self) -> usize {
        self.parser.screen().scrollback() + self.rows as usize
    }

    /// Number of visible screen lines.
    pub fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    /// Scrollback line count.
    pub fn scrollback_len(&self) -> usize {
        self.parser.screen().scrollback()
    }
}

fn build_empty_snapshot(rows: u16, cols: u16) -> Vec<Vec<CellSnapshot>> {
    vec![vec![CellSnapshot::empty(); cols as usize]; rows as usize]
}
