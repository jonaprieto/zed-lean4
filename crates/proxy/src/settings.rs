//! The infoview settings Zed hands the proxy: `lsp.lean4-ide.settings.infoview`.
use serde_json::Value;

/// The part of `lsp.lean4-ide.settings` meant for the infoviews.
pub const INFOVIEW: &str = "infoview";

/// `infoview.open`: which infoview the run button and the code action open.
const OPEN: &str = "open";
/// The value of `infoview.open` that picks the browser page; the default is
/// the terminal infoview.
const BROWSER: &str = "browser";

/// Whether the browser infoview is the one to open, rather than the terminal one.
pub fn opens_browser(infoview: &Value) -> bool {
    infoview[OPEN] == BROWSER
}
