//! Helpers shared by the terminal infoview's integration tests.
use lean4_ide_proxy::tui::app::App;
use ratatui::{Terminal, backend::TestBackend, layout::Size};

/// The terminal `screen` draws on.
const SMALL: Size = Size::new(60, 20);

/// The app drawn on a 60x20 terminal, one line per row, trailing spaces trimmed.
pub fn screen(app: &App) -> String {
    screen_of(app, SMALL)
}

/// The app drawn on a terminal of `size`, one line per row, trailing spaces trimmed.
pub fn screen_of(app: &App, size: Size) -> String {
    let mut terminal = Terminal::new(TestBackend::new(size.width, size.height)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buf = terminal.backend().buffer();
    (0..size.height)
        .map(|y| {
            let row: String = (0..size.width).map(|x| buf[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
