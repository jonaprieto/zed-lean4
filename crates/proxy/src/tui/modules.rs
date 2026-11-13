//! The shown file's place in the module hierarchy: what it imports and what
//! imports it, asked of Lean only while the section is open.
use ratatui::text::Line;
use serde_json::{Value, json};

use super::{
    app::{Effect, clear_if_awaiting},
    lean,
    rpc::Rpc,
    theme,
    view::Rows,
};

/// The fold key of the section, which starts folded.
pub const KEY: &str = "modules";

/// A module and the file it lives in.
#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub name: String,
    pub uri: String,
}

impl Module {
    fn from(value: &Value) -> Option<Self> {
        Some(Self {
            name: value["name"].as_str()?.to_owned(),
            uri: value["uri"].as_str()?.to_owned(),
        })
    }
}

/// The lists for one file and the requests still out for them.
#[derive(Default)]
pub struct Hierarchy {
    /// The file the lists are for, once asked.
    uri: Option<String>,
    prepare: Option<String>,
    imports_id: Option<String>,
    imported_by_id: Option<String>,
    imports: Option<Vec<Module>>,
    imported_by: Option<Vec<Module>>,
}

impl Hierarchy {
    /// Asks Lean for the module of `uri` unless the lists are already for it.
    pub fn follow(&mut self, uri: &str, rpc: &mut Rpc) -> Vec<Effect> {
        if self.uri.as_deref() == Some(uri) {
            return vec![];
        }
        *self = Self {
            uri: Some(uri.to_owned()),
            ..Self::default()
        };
        let params = json!({"textDocument":{"uri":uri}});
        let (id, call) = request(rpc, uri, lean::PREPARE_MODULE_HIERARCHY, params);
        self.prepare = Some(id);
        vec![call]
    }

    /// Takes a reply to one of its requests and says what to send next;
    /// `None` when `id` is not one of them. Lean versions without the
    /// hierarchy answer with an error, which leaves the lists empty.
    pub fn answer(&mut self, id: &str, reply: &Value, rpc: &mut Rpc) -> Option<Vec<Effect>> {
        let result = &reply["result"];
        if clear_if_awaiting(&mut self.prepare, id) {
            let (Some(uri), true) = (self.uri.clone(), result.is_object()) else {
                self.imports = Some(vec![]);
                self.imported_by = Some(vec![]);
                return Some(vec![]);
            };
            let params = json!({"module":result});
            let (imports_id, imports) = request(rpc, &uri, lean::MODULE_IMPORTS, params.clone());
            let (by_id, by) = request(rpc, &uri, lean::MODULE_IMPORTED_BY, params);
            (self.imports_id, self.imported_by_id) = (Some(imports_id), Some(by_id));
            return Some(vec![imports, by]);
        }
        let list = if clear_if_awaiting(&mut self.imports_id, id) {
            &mut self.imports
        } else if clear_if_awaiting(&mut self.imported_by_id, id) {
            &mut self.imported_by
        } else {
            return None;
        };
        *list = Some(
            result
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|import| Module::from(&import["module"]))
                .collect(),
        );
        Some(vec![])
    }

    /// The section: folded it is only its header; open, each list under its
    /// title, with rows that open their module.
    pub fn rows(&self, rows: &mut Rows) {
        rows.push(Line::raw(""));
        let heading = theme::current().heading;
        rows.open_as(KEY.into(), Line::styled("Modules", heading), true);
        for (title, list) in [
            ("Imports", &self.imports),
            ("Imported by", &self.imported_by),
        ] {
            rows.push(Line::styled(title, heading));
            match list.as_deref() {
                None => rows.push(Line::raw("  ⋯")),
                Some([]) => rows.push(Line::raw("  none")),
                Some(modules) => {
                    for module in modules {
                        rows.push_target(Line::raw(format!("  {}", module.name)), &module.uri);
                    }
                }
            }
        }
        rows.close();
    }
}

/// A Lean request about `uri` through the page bridge.
fn request(rpc: &mut Rpc, uri: &str, method: &str, params: Value) -> (String, Effect) {
    rpc.call(lean::SEND_CLIENT_REQUEST, json!([uri, method, params]))
}
