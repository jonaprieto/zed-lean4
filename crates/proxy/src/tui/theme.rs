//! Colors of the terminal infoview. By default they are the terminal's own
//! named colors, which Zed's terminal takes from the editor theme; the
//! `infoview.theme` settings override them role by role.
use std::{cell::Cell, str::FromStr};

use ratatui::style::{Color, Modifier, Style};
use serde_json::Value;

use super::lean::Severity;

/// `infoview.theme`: role name -> a color, or `{fg, bg, modifiers}`.
const THEME: &str = "theme";

/// Declares the roles once: the struct, its defaults and the lookup by setting name.
macro_rules! roles {
    ($($(#[$doc:meta])* $role:ident: $default:expr,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Theme {
            $($(#[$doc])* pub $role: Style,)*
        }

        impl Default for Theme {
            fn default() -> Self {
                Self { $($role: $default,)* }
            }
        }

        impl Theme {
            fn role_mut(&mut self, name: &str) -> Option<&mut Style> {
                match name {
                    $(stringify!($role) => Some(&mut self.$role),)*
                    _ => None,
                }
            }
        }
    };
}

roles! {
    /// Hypothesis names, and locals in highlighted code.
    hypothesis: Style::new().fg(Color::Cyan),
    /// The `⊢` before a goal.
    goal_prefix: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
    /// Inaccessible names such as `a✝`, and the word `case`.
    inaccessible: Style::new().add_modifier(Modifier::DIM),
    case_name: Style::new().add_modifier(Modifier::BOLD),
    /// What the tactic inserted or removed, in goal diffs.
    inserted: Style::new().fg(Color::Black).bg(Color::Green),
    removed: Style::new().fg(Color::Black).bg(Color::Red),
    keyword: Style::new().fg(Color::Magenta),
    literal: Style::new().fg(Color::Green),
    trace_class: Style::new().fg(Color::Blue),
    selection: Style::new().add_modifier(Modifier::REVERSED),
    /// Section titles: tactic state, expected type, messages.
    heading: Style::new().add_modifier(Modifier::BOLD),
    /// Docstrings and other prose.
    docs: Style::new().add_modifier(Modifier::DIM),
    error: Style::new().fg(Color::Red),
    warning: Style::new().fg(Color::Yellow),
    information: Style::new().fg(Color::Blue),
    hint: Style::new().add_modifier(Modifier::DIM),
    /// File progress in the header: all checked, or still checking.
    checked: Style::new().fg(Color::Green),
    checking: Style::new().fg(Color::Yellow),
}

thread_local! {
    // ponytail: one view per process on one thread; tests each get their own default.
    static CURRENT: Cell<Theme> = Cell::new(Theme::default());
}

/// The theme the view draws with.
pub fn current() -> Theme {
    CURRENT.get()
}

pub fn set(theme: Theme) {
    CURRENT.set(theme);
}

impl Theme {
    /// The default theme with `infoview.theme` applied, and what in it was
    /// not understood.
    pub fn from_settings(infoview: &Value) -> (Self, Vec<String>) {
        let mut theme = Self::default();
        let mut errors = Vec::new();
        for (name, value) in infoview[THEME].as_object().into_iter().flatten() {
            let applied = match theme.role_mut(name) {
                Some(style) => apply(style, value),
                None => Err("no such role".into()),
            };
            if let Err(e) = applied {
                errors.push(format!("{THEME}.{name}: {e}"));
            }
        }
        (theme, errors)
    }

    pub fn severity(&self, severity: Severity) -> Style {
        match severity {
            Severity::Error => self.error,
            Severity::Warning => self.warning,
            Severity::Information => self.information,
            Severity::Hint => self.hint,
        }
    }
}

/// A string sets the foreground; an object sets `fg`, `bg` and, when given,
/// replaces the role's `modifiers`.
fn apply(style: &mut Style, value: &Value) -> Result<(), String> {
    match value {
        Value::String(_) => style.fg = Some(color(value)?),
        Value::Object(o) => {
            if let Some(fg) = o.get("fg") {
                style.fg = Some(color(fg)?);
            }
            if let Some(bg) = o.get("bg") {
                style.bg = Some(color(bg)?);
            }
            if let Some(names) = o.get("modifiers") {
                style.add_modifier = modifiers(names)?;
                style.sub_modifier = Modifier::empty();
            }
        }
        _ => return Err("expected a color or {fg, bg, modifiers}".into()),
    }
    Ok(())
}

/// A color name (`red`, `light-blue`), `#rrggbb`, or a 256-color index.
fn color(value: &Value) -> Result<Color, String> {
    let name = value
        .as_str()
        .ok_or_else(|| format!("{value} is not a color name"))?;
    Color::from_str(name).map_err(|_| format!("unknown color {name:?}"))
}

fn modifiers(names: &Value) -> Result<Modifier, String> {
    let mut all = Modifier::empty();
    for name in names.as_array().ok_or("modifiers is not a list")? {
        all |= match name.as_str() {
            Some("bold") => Modifier::BOLD,
            Some("dim") => Modifier::DIM,
            Some("italic") => Modifier::ITALIC,
            Some("underlined") => Modifier::UNDERLINED,
            Some("reversed") => Modifier::REVERSED,
            Some("crossed_out") => Modifier::CROSSED_OUT,
            _ => return Err(format!("unknown modifier {name}")),
        };
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_override_roles_and_report_what_they_do_not_understand() {
        let settings = json!({"theme":{
            "keyword":"#ff8800",
            "selection":{"bg":"236","modifiers":[]},
            "goal_prefix":{"fg":"light-green"},
            "nonsense":"red",
            "literal":"blurple",
            "heading":{"fg":3},
            "docs":{"modifiers":["dim","shiny"]},
        }});
        let (theme, errors) = Theme::from_settings(&settings);
        assert_eq!(theme.keyword, Style::new().fg(Color::Rgb(0xff, 0x88, 0x00)));
        assert_eq!(theme.selection, Style::new().bg(Color::Indexed(236)));
        let prefix = Style::new()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD);
        assert_eq!(theme.goal_prefix, prefix);
        assert_eq!(theme.literal, Theme::default().literal);
        assert_eq!(
            errors,
            [
                "theme.docs: unknown modifier \"shiny\"",
                "theme.heading: 3 is not a color name",
                "theme.literal: unknown color \"blurple\"",
                "theme.nonsense: no such role",
            ]
        );
    }

    #[test]
    fn no_settings_is_the_default_theme() {
        assert_eq!(
            Theme::from_settings(&Value::Null),
            (Theme::default(), vec![])
        );
    }
}
