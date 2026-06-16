//! Deterministic colour system for task tags, plus the shared coloured `TagChip`.
//!
//! Well-known tags (mvp, launch, bug, …) get a fixed, meaningful colour so they
//! are instantly recognisable across the app and the TV board. Any unknown tag
//! is hashed into a small stable palette, so a given tag always renders with the
//! same colour without anyone having to register it first.

use leptos::prelude::*;

/// Background / foreground colour pair for a tag chip. Each pair is chosen to
/// read clearly on both the light app surfaces and the dark board.
pub fn tag_colors(tag: &str) -> (&'static str, &'static str) {
    match tag.trim().to_ascii_lowercase().as_str() {
        "mvp" => ("#c9ef6a", "#1d2b07"),      // lime / green
        "launch" => ("#4d8df6", "#ffffff"),   // blue
        "bug" => ("#e5484d", "#ffffff"),      // red
        "feature" => ("#9b6bf0", "#ffffff"),  // purple
        "writing" => ("#e0a52e", "#2a1d02"),  // amber
        "business" => ("#19b8a6", "#04221e"), // teal
        "personal" => ("#9aa3a0", "#181d1a"), // gray
        other => {
            // Stable hash → palette index so unknown tags keep one colour.
            const PALETTE: [(&str, &str); 8] = [
                ("#6d9bd1", "#08203a"),
                ("#d68a5c", "#2e1804"),
                ("#7dba8a", "#0c2a14"),
                ("#c77db5", "#300829"),
                ("#bcab63", "#272204"),
                ("#79b4b0", "#062623"),
                ("#a98fd0", "#1a0a33"),
                ("#cf9a5a", "#2c1a04"),
            ];
            let sum: usize = other.bytes().map(usize::from).sum();
            PALETTE[sum % PALETTE.len()]
        }
    }
}

/// A single coloured tag chip. Self-contained colours (set via CSS variables)
/// mean the same component works on light pages and the dark board alike.
#[component]
pub fn TagChip(#[prop(into)] tag: String) -> impl IntoView {
    let (bg, fg) = tag_colors(&tag);
    view! {
        <span class="tag-chip-color" style=format!("--tag-bg:{bg};--tag-fg:{fg}")>{tag}</span>
    }
}
