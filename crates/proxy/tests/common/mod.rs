//! Helpers shared by the terminal infoview's integration tests.
use lean4_ide_proxy::tui::app::App;
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 60;
const HEIGHT: u16 = 20;

/// The app drawn on a 60x20 terminal, one line per row, trailing spaces trimmed.
pub fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buf = terminal.backend().buffer();
    (0..HEIGHT)
        .map(|y| {
            let row: String = (0..WIDTH).map(|x| buf[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
