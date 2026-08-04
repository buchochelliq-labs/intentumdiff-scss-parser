//! SCSS parser plugin - full-parse mode.
//!
//! Handles `.scss` and `.sass` files.
//! Parses source with tree-sitter-scss directly.
//!
//! SCSS is a superset of CSS.  This parser includes all CSS semantic types
//! and adds SCSS-specific constructs: variables, mixins, functions, includes,
//! extends, control-flow, and placeholder selectors.

use intentumdiff_plugin_sdk::{
    cst::CstNode,
    hash::structural_hash_with_memo,
    tree::{SemanticNode, SemanticNodeBuilder},
};

wit_bindgen::generate!({
    path: "wit/plugin.wit",
    world: "parser-plugin",
});

use crate::exports::intentumdiff::plugin::parser::ExamplePair;
use crate::exports::intentumdiff::plugin::parser::Guest;
use crate::exports::intentumdiff::plugin::parser::LanguageInfoRecord;
use crate::exports::intentumdiff::plugin::parser::ParserMode;

const PLUGIN_METADATA: &str = include_str!("../plugin_metadata.info");

fn language_info_for(ids: Vec<String>) -> Vec<LanguageInfoRecord> {
    let metadata = intentumdiff_plugin_sdk::metadata::parse_plugin_metadata(PLUGIN_METADATA);
    ids.into_iter()
        .map(|language_id| {
            let info = metadata.language_or_default(&language_id);
            LanguageInfoRecord {
                language_id: info.language_id,
                language_name: info.language_name,
                language_short_name: info.language_short_name,
                monaco_language: info.monaco_language,
                default_filename: info.default_filename,
                language_file_extensions: info.language_file_extensions,
                author: metadata.author().to_string(),
                plugin_version: metadata.plugin_version().to_string(),
                last_updated: metadata.last_updated().to_string(),
            }
        })
        .collect()
}
struct ScssParser;

const TRIVIA: &[&str] = &["comment", "single_line_comment", "whitespace"];

const SEMANTIC_TYPES: &[&str] = &[
    // ── CSS base ────────────────────────────────────────────────────────────
    "stylesheet",
    "rule_set",
    "declaration",
    "media_statement",
    "keyframes_statement",
    "keyframe_block",
    "import_statement",
    "supports_statement",
    "charset_statement",
    "namespace_statement",
    "at_rule",
    // CSS Selectors
    "class_selector",
    "id_selector",
    "type_selector",
    "pseudo_class_selector",
    "pseudo_element_selector",
    "attribute_selector",
    "universal_selector",
    "child_selector",
    "sibling_selector",
    "adjacent_sibling_selector",
    "descendant_selector",
    "selectors",
    // ── SCSS extensions ─────────────────────────────────────────────────────
    // Variable declaration: $var: value;
    "variable_declaration",
    // Mixin definition: @mixin name($args) { }
    "mixin_statement",
    "parameters",
    "parameter",
    // Mixin inclusion: @include mixin-name($args);
    "include_statement",
    // Function definition: @function name($args) { @return ...; }
    "function_statement",
    // Placeholder selector: %placeholder { }
    "placeholder_selector",
    // Extend: @extend %placeholder;
    "extend_statement",
    // Control flow
    "if_statement",
    "else_clause",
    "each_statement",
    "for_statement",
    "while_statement",
    // Return: @return value;
    "return_statement",
    // Error / warn / debug
    "error_statement",
    "warn_statement",
    "debug_statement",
];

fn is_semantic(node_type: &str) -> bool {
    SEMANTIC_TYPES.contains(&node_type)
}

fn label_for(node: &CstNode) -> String {
    if node.is_leaf() {
        return node.text_or_empty().to_string();
    }
    // Literal containers label with their captured source text (SDK-shared, issue #47).
    if let Some(label) = intentumdiff_plugin_sdk::ts_convert::literal_label(node) {
        return label;
    }
    match node.node_type.as_str() {
        "rule_set" => {
            for child in &node.children {
                if child.node_type == "selectors" {
                    return selector_text(child);
                }
            }
        }
        "declaration" => {
            for child in &node.children {
                if child.node_type == "property_name" {
                    return child.text_or_empty().to_string();
                }
            }
        }
        "variable_declaration" => {
            // "$var-name: value" — first child is typically the variable name
            for child in &node.children {
                if child.node_type == "variable_name" || child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() && t != ":" {
                        return t.to_string();
                    }
                }
            }
        }
        "mixin_statement" | "function_statement" => {
            // @mixin name(...) — look for the name identifier
            let mut name: Option<String> = None;
            let mut parameters: Option<String> = None;
            for child in &node.children {
                if child.node_type == "parameters" {
                    let params = label_for(child);
                    if !params.is_empty() {
                        parameters = Some(params);
                    }
                    continue;
                }
                if child.node_type == "name" || child.node_type == "identifier" {
                    let t = child.text_or_empty();
                    if !t.is_empty() {
                        name = Some(t.to_string());
                    }
                    continue;
                }
                if child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() && !matches!(t, "@mixin" | "@function" | "{" | "}" | "(") {
                        name = Some(t.to_string());
                    }
                }
            }
            if let Some(name) = name {
                if let Some(parameters) = parameters {
                    return format!("{}({})", name, parameters);
                }
                return name;
            }
        }
        "parameters" => {
            let parts: Vec<String> = node
                .children
                .iter()
                .map(label_for)
                .filter(|s| !s.is_empty())
                .collect();
            if !parts.is_empty() {
                return parts.join(", ");
            }
        }
        "parameter" => {
            for child in &node.children {
                if child.node_type == "variable" || child.node_type == "variable_name" {
                    return child.text_or_empty().to_string();
                }
                if child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() {
                        return t.to_string();
                    }
                }
            }
        }
        "include_statement" => {
            // @include mixin-name(...)
            for child in &node.children {
                if child.node_type == "name" || child.node_type == "identifier" {
                    return child.text_or_empty().to_string();
                }
                if child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() && !matches!(t, "@include" | "(") {
                        return t.to_string();
                    }
                }
            }
        }
        "extend_statement" => {
            // @extend %placeholder or @extend .selector
            for child in &node.children {
                if child.node_type == "placeholder_selector"
                    || child.node_type == "class_selector"
                    || child.is_leaf()
                {
                    let t = child.text_or_empty();
                    if !t.is_empty() && t != "@extend" {
                        return t.to_string();
                    }
                }
            }
        }
        "placeholder_selector" => {
            // %name
            for child in &node.children {
                if child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() && t != "%" {
                        return format!("%{}", t);
                    }
                }
            }
        }
        "media_statement" => {
            for child in &node.children {
                if matches!(
                    child.node_type.as_str(),
                    "keyword_query" | "feature_query" | "binary_query" | "media_query"
                ) {
                    return child.text_or_empty().to_string();
                }
            }
        }
        "keyframes_statement" => {
            for child in &node.children {
                if child.node_type == "keyframes_name" {
                    return child.text_or_empty().to_string();
                }
            }
        }
        "class_selector" => {
            for child in &node.children {
                if child.node_type == "class_name" || child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() && t != "." {
                        return format!(".{}", t);
                    }
                }
            }
        }
        "id_selector" => {
            for child in &node.children {
                if child.node_type == "id_name" || child.is_leaf() {
                    let t = child.text_or_empty();
                    if !t.is_empty() && t != "#" {
                        return format!("#{}", t);
                    }
                }
            }
        }
        _ => {}
    }
    // Fallback: first non-empty leaf text
    for child in &node.children {
        if child.is_leaf() {
            let t = child.text_or_empty();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    node.node_type.clone()
}

fn selector_text(node: &CstNode) -> String {
    if node.is_leaf() {
        return node.text_or_empty().to_string();
    }
    let parts: Vec<String> = node
        .children
        .iter()
        .map(|c| {
            if c.is_leaf() {
                c.text_or_empty().to_string()
            } else {
                label_for(c)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        node.node_type.clone()
    } else {
        parts.join(", ")
    }
}

fn convert(
    node: &CstNode,
    id_prefix: &str,
    memo: &mut std::collections::HashMap<usize, String>,
) -> Option<SemanticNode> {
    convert_semantic_strict(
        node,
        id_prefix,
        memo,
        &|t| TRIVIA.contains(&t),
        &is_semantic,
        &label_for,
    )
}



use intentumdiff_plugin_sdk::ts_convert::{convert_semantic_strict, node_to_cst};

fn parse_source(source: &str) -> Result<CstNode, String> {
    let mut parser = tree_sitter::Parser::new();
    let lang = tree_sitter_scss::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|_| "Failed to load SCSS grammar".to_string())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Parse failed".to_string())?;
    Ok(node_to_cst(tree.root_node(), source.as_bytes()))
}

fn process_impl(source: &str) -> String {
    let root: CstNode = match parse_source(source) {
        Ok(n) => n,
        Err(e) => return format!(r#"{{"error":"{}"}}"#, e),
    };
    let mut memo: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let sem = match convert(&root, "0", &mut memo) {
        Some(n) => n,
        None => return r#"{"error":"Empty semantic tree"}"#.to_string(),
    };
    match serde_json::to_string(&sem) {
        Ok(s) => s,
        Err(e) => format!(r#"{{"error":"Serialisation error: {}"}}"#, e),
    }
}

impl Guest for ScssParser {
    fn get_parser_mode() -> ParserMode {
        ParserMode::FullParse
    }
    fn grammar_id() -> String {
        "scss".to_string()
    }
    fn detect_language(filename: String, _content: String) -> String {
        let lower = filename.to_lowercase();
        if lower.ends_with(".scss") || lower.ends_with(".sass") {
            return "scss".to_string();
        }
        String::new()
    }
    fn preprocess_source(source: String) -> String {
        source
    }
    fn example(_language: String) -> ExamplePair {
        ExamplePair {
            old: "$primary: blue;\n\n.button {\n  background: $primary;\n  color: white;\n  padding: 10px;\n}\n".to_string(),
            new: "$primary:     #2563eb;\n$primary-dark: #1d4ed8;\n$white:        #ffffff;\n\n@mixin button-base {\n  border: none;\n  border-radius: 6px;\n  cursor: pointer;\n  transition: background-color 0.2s ease;\n}\n\n.button {\n  @include button-base;\n  background-color: $primary;\n  color: $white;\n  padding: 8px 16px;\n\n  &:hover { background-color: $primary-dark; }\n  &:focus { outline: 2px solid #93c5fd; }\n}\n".to_string(),
        }
    }
    fn process(input: String, _language: String, _filename: String) -> String {
        process_impl(&input)
    }
    fn trivia_node_types() -> Vec<String> {
        TRIVIA.iter().map(|s| s.to_string()).collect()
    }
    fn language_ids() -> Vec<String> {
        vec!["scss".to_string()]
    }
    fn language_info() -> Vec<LanguageInfoRecord> {
        language_info_for(Self::language_ids())
    }
    fn priority() -> i32 {
        0
    }
}

export!(ScssParser);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::intentumdiff::plugin::parser::Guest;
    use intentumdiff_plugin_sdk::testing as t;

    #[test]
    fn grammar_id_nonempty() {
        assert!(!ScssParser::grammar_id().is_empty());
    }

    #[test]
    fn language_ids_contain_grammar_id() {
        let gid = ScssParser::grammar_id();
        let ids = ScssParser::language_ids();
        assert!(
            ids.contains(&gid),
            "language_ids {:?} must contain {:?}",
            ids,
            gid
        );
    }

    #[test]
    fn detect_language_scss() {
        assert_eq!(
            ScssParser::detect_language("styles.scss".to_string(), "".to_string()),
            "scss"
        );
    }

    #[test]
    fn detect_language_sass() {
        assert_eq!(
            ScssParser::detect_language("theme.sass".to_string(), "".to_string()),
            "scss"
        );
    }

    #[test]
    fn detect_language_unknown() {
        let r = ScssParser::detect_language("main.css".to_string(), "".to_string());
        assert_eq!(r.as_str(), "");
    }

    #[test]
    fn process_impl_empty_returns_valid_json() {
        let out = process_impl("");
        t::assert_valid_json(&out, "process(empty)");
    }

    #[test]
    fn process_impl_whitespace_returns_valid_json() {
        let out = process_impl("   \n  ");
        t::assert_valid_json(&out, "process(whitespace)");
    }
}
