//! Rows wrapped to the terminal width, cached between renders, and where the
//! last render put them, so clicks can find the row and character under them.
use std::{cell::RefCell, ops::Range, rc::Rc};

use ratatui::{
    layout::{Position, Rect},
    text::Line,
};

use super::view::{self, Row};

/// A screen line: its row, the row characters `start..end` it shows, and its
/// wrap indentation.
#[derive(Clone, Copy)]
pub(super) struct Segment {
    pub(super) row: usize,
    start: usize,
    end: usize,
    indent: usize,
}

impl Segment {
    /// The character of the row's `line` under column `col` of this screen line.
    pub(super) fn char_at(&self, line: &Line, col: usize) -> Option<usize> {
        col.checked_sub(self.indent)
            .and_then(|col| view::char_at(line, self.start, col))
            .filter(|&c| c < self.end)
    }
}

/// All rows wrapped to one width: the screen lines, the segment of a row each
/// shows, and each row's first screen line.
pub(super) struct Wrapped {
    width: u16,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) segments: Vec<Segment>,
    row_starts: Vec<usize>,
}

impl Wrapped {
    fn new(rows: &[Row], width: u16) -> Self {
        let mut w = Self {
            width,
            lines: Vec::new(),
            segments: Vec::new(),
            row_starts: Vec::new(),
        };
        for (i, row) in rows.iter().enumerate() {
            w.row_starts.push(w.lines.len());
            let chars = row
                .line
                .spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum();
            let wrapped = view::wrap(row.line.clone(), width.into());
            let ends: Vec<usize> = wrapped.iter().skip(1).map(|w| w.1).chain([chars]).collect();
            for ((line, start, indent), end) in wrapped.into_iter().zip(ends) {
                w.segments.push(Segment {
                    row: i,
                    start,
                    end,
                    indent,
                });
                w.lines.push(line);
            }
        }
        w
    }

    /// The screen lines of `row`, or of the last row when there are fewer.
    pub(super) fn lines_of(&self, row: usize) -> Range<usize> {
        let row = row.min(self.row_starts.len().saturating_sub(1));
        let top = self.row_starts.get(row).copied().unwrap_or(0);
        let bottom = self
            .row_starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.lines.len());
        top..bottom
    }
}

/// Where the last render put each visible screen line: the body area and the
/// segment each of its lines shows.
#[derive(Default)]
pub(super) struct Screen {
    pub(super) area: Rect,
    pub(super) lines: Vec<Segment>,
}

impl Screen {
    /// The segment shown at `(x, y)` and the column within it.
    pub(super) fn hit(&self, x: u16, y: u16) -> Option<(Segment, usize)> {
        if !self.area.contains(Position { x, y }) {
            return None;
        }
        let line = usize::from(y - self.area.y);
        let col = usize::from(x - self.area.x);
        self.lines.get(line).map(|&segment| (segment, col))
    }
}

/// Rows and their wrapped lines, rebuilt only after the state they show changes.
#[derive(Default)]
pub(super) struct Cache {
    rows: RefCell<Option<Rc<Vec<Row>>>>,
    wrapped: RefCell<Option<Rc<Wrapped>>>,
}

impl Cache {
    /// Forgets the rows after a change to what they show.
    pub(super) fn clear(&self) {
        self.rows.take();
        self.wrapped.take();
    }

    /// The cached rows, or the ones `build` makes.
    pub(super) fn rows(&self, build: impl FnOnce() -> Vec<Row>) -> Rc<Vec<Row>> {
        self.rows
            .borrow_mut()
            .get_or_insert_with(|| Rc::new(build()))
            .clone()
    }

    /// The cached lines at `width`, or `rows` wrapped to it.
    pub(super) fn wrapped(&self, width: u16, rows: impl FnOnce() -> Rc<Vec<Row>>) -> Rc<Wrapped> {
        if let Some(w) = self.wrapped.borrow().as_ref().filter(|w| w.width == width) {
            return w.clone();
        }
        let w = Rc::new(Wrapped::new(&rows(), width));
        *self.wrapped.borrow_mut() = Some(w.clone());
        w
    }
}
