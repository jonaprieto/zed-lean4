//! How to type a Lean symbol: the `\` abbreviations the extension's snippets
//! define, looked up by the symbol so hovers can teach them.
use std::{collections::HashMap, sync::OnceLock};

use serde_json::Value;

/// The extension's snippets, whose `\name` prefixes are the abbreviations.
const SNIPPETS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../snippets/lean 4.json"
));

/// Symbol -> its abbreviations, shortest first.
pub struct Abbreviations {
    by_symbol: HashMap<String, Vec<String>>,
    /// Characters in the longest symbol.
    longest: usize,
}

/// The abbreviations from the shipped snippets, read once.
pub fn shipped() -> &'static Abbreviations {
    static SHIPPED: OnceLock<Abbreviations> = OnceLock::new();
    SHIPPED.get_or_init(|| Abbreviations::from_snippets(SNIPPETS))
}

impl Abbreviations {
    /// Reads the snippets whose prefix starts with `\` and whose body is a
    /// plain symbol: no tab stops or escapes, and not ASCII.
    pub fn from_snippets(json: &str) -> Self {
        let snippets: Value = serde_json::from_str(json).unwrap_or_default();
        let mut by_symbol: HashMap<String, Vec<String>> = HashMap::new();
        for snippet in snippets.as_object().into_iter().flat_map(|o| o.values()) {
            let (Some(prefix), Some(body)) = (snippet["prefix"].as_str(), snippet["body"].as_str())
            else {
                continue;
            };
            let symbol = !body.is_ascii() && !body.contains(['$', '\\']);
            if prefix.starts_with('\\') && symbol {
                by_symbol
                    .entry(body.to_owned())
                    .or_default()
                    .push(prefix.to_owned());
            }
        }
        for ways in by_symbol.values_mut() {
            ways.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        }
        let longest = by_symbol
            .keys()
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0);
        Self { by_symbol, longest }
    }

    /// The symbol in `line` that covers byte `at`, the longest one when
    /// several do, and its abbreviations.
    pub fn at(&self, line: &str, at: usize) -> Option<(&str, &[String])> {
        if at >= line.len() || !line.is_char_boundary(at) {
            return None;
        }
        // Symbols are short, so try every span of up to `longest` characters around `at`.
        let before = line[..at]
            .char_indices()
            .rev()
            .take(self.longest.saturating_sub(1))
            .map(|(i, _)| i);
        let mut best: Option<(&str, &[String])> = None;
        for start in std::iter::once(at).chain(before) {
            let ends = line[start..]
                .char_indices()
                .skip(1)
                .map(|(i, _)| start + i)
                .chain(std::iter::once(line.len()))
                .take(self.longest)
                .filter(|&end| end > at);
            for end in ends {
                let span = &line[start..end];
                if let Some((symbol, ways)) = self.by_symbol.get_key_value(span)
                    && best.is_none_or(|(b, _)| symbol.len() > b.len())
                {
                    best = Some((symbol, ways));
                }
            }
        }
        best
    }
}

/// A hover line teaching how to type `symbol`.
pub fn note(symbol: &str, abbreviations: &[String]) -> String {
    let ways: Vec<String> = abbreviations.iter().map(|a| format!("`{a}`")).collect();
    let ways = match ways.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    };
    format!("Type `{symbol}` with {ways}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_map_to_their_abbreviations_shortest_first() {
        let snippets = r#"{
            "lean-alpha": {"prefix": "\\alpha", "body": "α"},
            "lean-a": {"prefix": "\\a", "body": "α"},
            "lean-inv": {"prefix": "\\inv", "body": "⁻¹"},
            "lean-sup": {"prefix": "\\-", "body": "⁻"},
            "lean-()": {"prefix": "\\()", "body": "($0)"},
            "lean-n": {"prefix": "\\n", "body": "\\n"},
            "calc": {"prefix": "calc", "body": ["calc"]}
        }"#;
        let abbreviations = Abbreviations::from_snippets(snippets);
        let line = "x⁻¹ = α";
        let alpha = line.find('α').unwrap();
        assert_eq!(
            abbreviations.at(line, alpha),
            Some(("α", ["\\a".to_owned(), "\\alpha".to_owned()].as_slice()))
        );
        // `¹` alone has no abbreviation, but the `⁻¹` it ends does.
        let sup = line.find('¹').unwrap();
        assert_eq!(
            abbreviations.at(line, sup),
            Some(("⁻¹", ["\\inv".to_owned()].as_slice()))
        );
        assert_eq!(abbreviations.at(line, 0), None, "x");
        assert_eq!(abbreviations.at(line, line.len()), None, "past the end");
    }

    #[test]
    fn the_shipped_snippets_know_common_symbols() {
        let (symbol, ways) = shipped().at("⊢ p", 0).unwrap();
        assert_eq!(symbol, "⊢");
        assert!(ways.contains(&"\\vdash".to_owned()), "{ways:?}");
    }

    #[test]
    fn the_note_lists_every_way_to_type_the_symbol() {
        let ways = ["\\a", "\\Ga", "\\alpha"].map(String::from);
        assert_eq!(
            note("α", &ways),
            "Type `α` with `\\a`, `\\Ga` or `\\alpha`."
        );
        assert_eq!(note("⁻¹", &ways[..1]), "Type `⁻¹` with `\\a`.");
    }
}
