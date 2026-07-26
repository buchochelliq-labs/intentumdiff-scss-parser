//! SCSS grammar bindings, modernized to the tree-sitter-language ABI for the
//! IntentDiff wasi patch (the upstream bindings need a full tree-sitter dep;
//! see docs/WASM_BUILD_PATCHES.md).

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_scss() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for this grammar.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_scss) };

pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");
