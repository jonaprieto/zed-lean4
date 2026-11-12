//! The settings Zed hands the terminal infoview: `lsp.lean4-ide.settings.infoview`.
use serde_json::Value;

/// `infoview.open`: which infoview the gutter run button opens.
const OPEN: &str = "open";
/// The value of `infoview.open` that sends the run button to the browser page.
const BROWSER: &str = "browser";

/// Whether the run button should open the browser infoview instead of this one.
pub fn opens_browser(infoview: &Value) -> bool {
    infoview[OPEN] == BROWSER
}
