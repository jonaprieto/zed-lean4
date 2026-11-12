//! Names and codes of the protocols the terminal infoview speaks: the proxy's
//! infoview page calls and events, the LSP methods behind them, and Lean's
//! widget RPC methods.
use serde_json::Value;

// Calls the infoview page makes to the proxy.
pub const HELLO: &str = "hello";
pub const SUBSCRIBE_SERVER_NOTIFICATIONS: &str = "subscribeServerNotifications";
pub const CREATE_RPC_SESSION: &str = "createRpcSession";
pub const SEND_CLIENT_REQUEST: &str = "sendClientRequest";
pub const SEND_CLIENT_NOTIFICATION: &str = "sendClientNotification";
pub const SHOW_DOCUMENT: &str = "showDocument";
pub const INSERT_TEXT: &str = "insertText";

// Events the proxy sends the infoview page.
pub const CHANGED_CURSOR_LOCATION: &str = "changedCursorLocation";
pub const GOT_SERVER_NOTIFICATION: &str = "gotServerNotification";
pub const SENT_CLIENT_NOTIFICATION: &str = "sentClientNotification";

// LSP methods.
pub const PUBLISH_DIAGNOSTICS: &str = "textDocument/publishDiagnostics";
pub const RPC_CALL: &str = "$/lean/rpc/call";
pub const RPC_RELEASE: &str = "$/lean/rpc/release";
pub const HOVER: &str = "textDocument/hover";
pub const DID_CHANGE_CONFIGURATION: &str = "workspace/didChangeConfiguration";

// Lean's widget RPC methods.
pub const GET_INTERACTIVE_GOALS: &str = "Lean.Widget.getInteractiveGoals";
pub const GET_INTERACTIVE_TERM_GOAL: &str = "Lean.Widget.getInteractiveTermGoal";
pub const GET_INTERACTIVE_DIAGNOSTICS: &str = "Lean.Widget.getInteractiveDiagnostics";
pub const LAZY_TRACE_CHILDREN_TO_INTERACTIVE: &str = "Lean.Widget.lazyTraceChildrenToInteractive";
pub const INFO_TO_INTERACTIVE: &str = "Lean.Widget.InteractiveDiagnostics.infoToInteractive";
pub const GET_GO_TO_LOCATION: &str = "Lean.Widget.getGoToLocation";

// The `kind` of a `getGoToLocation` call.
pub const GO_TO_DEFINITION: &str = "definition";
pub const GO_TO_TYPE: &str = "type";

/// Lean's `RpcNeedsReconnect`: the file's worker restarted and dropped the session.
pub const RPC_NEEDS_RECONNECT: i64 = -32900;
/// LSP `ContentModified`: an edit overtook the request.
pub const CONTENT_MODIFIED: i64 = -32801;
/// LSP `RequestCancelled`.
pub const REQUEST_CANCELLED: i64 = -32800;

/// An LSP diagnostic's severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    /// The severity of `diagnostic`; a missing one counts as an error.
    pub fn of(diagnostic: &Value) -> Self {
        match diagnostic["severity"].as_u64() {
            Some(2) => Self::Warning,
            Some(3) => Self::Information,
            Some(4) => Self::Hint,
            _ => Self::Error,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }
}
