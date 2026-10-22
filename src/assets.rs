//! Static assets, embedded in the binary so a single file can be deployed.
//!
//! The URLs carry a fingerprint of the file's contents. That is what makes it
//! safe to cache them for a long time: a changed stylesheet is a changed URL, so
//! a browser can never be stuck with the previous version's CSS.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

pub const STYLESHEET: &str = include_str!("../assets/app.css");
pub const SCRIPT: &str = include_str!("../assets/app.js");

/// First eight hex digits of the SHA-256 of an asset. Short on purpose: it only
/// has to change when the file changes.
fn fingerprint(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    digest.iter().take(4).fold(String::new(), |mut out, byte| {
        out.push_str(&format!("{byte:02x}"));
        out
    })
}

static STYLESHEET_URL: OnceLock<String> = OnceLock::new();
static SCRIPT_URL: OnceLock<String> = OnceLock::new();

/// `cell` has to be the caller's own cell: a `static` inside a function is
/// shared by every caller, which would hand the stylesheet's URL to the script.
fn url_with_fingerprint(
    cell: &'static OnceLock<String>,
    path: &str,
    content: &str,
) -> &'static str {
    cell.get_or_init(|| format!("{path}?v={}", fingerprint(content)))
}

pub fn stylesheet_url() -> &'static str {
    url_with_fingerprint(&STYLESHEET_URL, "/static/app.css", STYLESHEET)
}

pub fn script_url() -> &'static str {
    url_with_fingerprint(&SCRIPT_URL, "/static/app.js", SCRIPT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_short_and_content_dependent() {
        assert_eq!(fingerprint("a").len(), 8);
        assert_eq!(fingerprint("a"), fingerprint("a"));
        assert_ne!(fingerprint("a"), fingerprint("b"));
        assert!(fingerprint(STYLESHEET) != fingerprint(SCRIPT));
    }

    #[test]
    fn the_two_assets_do_not_share_a_url() {
        assert!(stylesheet_url().starts_with("/static/app.css?v="));
        assert!(script_url().starts_with("/static/app.js?v="));
        assert!(stylesheet_url().ends_with(&fingerprint(STYLESHEET)));
        assert!(script_url().ends_with(&fingerprint(SCRIPT)));
    }

    /// Regression: labels inside the create form used to be hidden by a blanket
    /// `form.inline label { display: none }` rule, which silently made the
    /// colour picker invisible whenever the HTML was newer than the cached CSS.
    #[test]
    fn no_blanket_rule_hides_labels_inside_forms() {
        assert!(
            !STYLESHEET.contains("form.inline label"),
            "hide labels one by one, not with a rule that catches everything"
        );
    }

    #[test]
    fn the_client_prefers_the_stream_and_can_fall_back_to_polling() {
        assert!(SCRIPT.contains("EventSource"));
        assert!(SCRIPT.contains("data-stream"));
        assert!(SCRIPT.contains("resync"));
        // The fallback has to stay: it is the only thing old browsers get.
        assert!(SCRIPT.contains("data-fallback"));
        assert!(SCRIPT.contains("data-refresh"));
    }

    #[test]
    fn the_stone_picker_is_visible_and_clickable() {
        // The labels are displayed; only the radio inside them is hidden.
        assert!(STYLESHEET.contains(".stone-option {"));
        let block = STYLESHEET
            .split(".stone-option {")
            .nth(1)
            .expect("a .stone-option rule");
        assert!(block.contains("display: inline-flex"), "got {block}");
        let radio = STYLESHEET
            .split(".stone-option input {")
            .nth(1)
            .expect("a hidden-radio rule");
        assert!(radio.contains("opacity: 0"), "the radio is drawn by us");
        assert!(STYLESHEET.contains(".stone-dot-black"));
        assert!(STYLESHEET.contains(".stone-dot-white"));
        // The selection must not depend on :has(), because the radio itself is
        // invisible: a browser without :has() would then show no selection.
        assert!(STYLESHEET.contains(".stone-option input:checked + .stone-dot"));
        assert!(STYLESHEET.contains(".stone-option input:checked ~ .stone-caption"));
        assert!(STYLESHEET.contains(".stone-option input:focus-visible + .stone-dot"));
    }
}
