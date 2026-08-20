//! Diagnostics helpers — every error carries the offending token's OWN span, and the canonical
//! unknown-construct error carries the full supported list plus a did-you-mean (§6.1).

use proc_macro2::Span;

/// The construct keywords THIS aether version supports, in the §6.1 registry order. Rung A0
/// registers two; the LIST names the whole v1 surface so the canonical diagnostic reads the same
/// on every rung and a reader learns what is coming rather than what happens to be implemented.
///
/// `plugin` is in the list because the block parser DISPATCHES on it (§3.3's header is a
/// construct as far as the keyword table is concerned). Leaving it out — the shape this list
/// shipped with — made `pluging P;` report the unknown-construct error with no did-you-mean and
/// a supported-list that misstated the surface.
pub const CONSTRUCT_KEYWORDS: &[&str] =
    &["component", "tag", "bundle", "system", "event", "plugin", "machine", "material", "scene"];

/// A `syn::Error` at `span` — the one constructor every parse path uses, so no diagnostic can
/// silently fall back to `Span::call_site()`.
pub fn err(span: Span, msg: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(span, msg)
}

/// The canonical extensibility diagnostic (§6.1): unknown construct + supported list
/// + did-you-mean when one keyword is within edit distance 2.
pub fn unknown_construct(span: Span, found: &str) -> syn::Error {
    let mut msg = format!(
        "unknown construct `{found}`; this aether supports: {}",
        CONSTRUCT_KEYWORDS.join(", ")
    );
    if let Some(sugg) = did_you_mean(found, CONSTRUCT_KEYWORDS) {
        msg.push_str(&format!(" (did you mean `{sugg}`?)"));
    }
    err(span, msg)
}

/// The nearest candidate within Levenshtein distance ≤ 2, ties to the first in registry order.
/// Small and exact rather than clever: the candidate set is eight short literals.
pub fn did_you_mean<'a>(found: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = levenshtein(found, c);
        if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c)
}

/// Textbook two-row Levenshtein — the inputs are keyword-sized, so simplicity wins.
fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        core::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_you_mean_finds_close_keywords_and_refuses_far_ones() {
        assert_eq!(did_you_mean("compnent", CONSTRUCT_KEYWORDS), Some("component"));
        assert_eq!(did_you_mean("tga", CONSTRUCT_KEYWORDS), Some("tag"));
        assert_eq!(did_you_mean("shader", CONSTRUCT_KEYWORDS), None);
    }

    /// The keyword the block parser dispatches on must be IN the list the diagnostic prints,
    /// or a near-miss on it gets neither a suggestion nor an honest surface description.
    #[test]
    fn the_registry_list_covers_every_dispatched_keyword() {
        assert_eq!(did_you_mean("pluging", CONSTRUCT_KEYWORDS), Some("plugin"));
        assert!(unknown_construct(Span::call_site(), "pluging").to_string().contains("plugin"));
        // `material` began dispatching at rung A5 — a keyword the parser routes but the list
        // omits is exactly the hole this test exists to catch.
        assert_eq!(did_you_mean("materia", CONSTRUCT_KEYWORDS), Some("material"));
    }
}
