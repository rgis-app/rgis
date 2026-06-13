//! Checked-in Luau scripting definitions.

const SCRIPT_DEFINITIONS: &str = include_str!("../luau/eguidev.d.luau");

/// Return the checked-in Luau definitions that describe the scripting API.
pub fn script_definitions() -> &'static str {
    SCRIPT_DEFINITIONS
}

/// Render the checked-in Luau definitions in a markdown code fence.
pub fn render_script_docs_markdown() -> String {
    format!("```luau\n{}\n```\n", SCRIPT_DEFINITIONS.trim())
}
