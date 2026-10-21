//! End-to-end checks of the text pipeline (`prepare_markdown`) over real documents.

use std::fs;
use std::path::Path;
use workman::html_prep::{preprocess_html, preprocess_html_with};
use workman::markdown::{
    content_heading_positions, extract_toc_and_inject_ids, prepare_markdown, validate_code_fences,
};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every TOC entry must be reachable (its `{#slug}` is in the content and positioned),
/// slugs must be unique, and the whole thing must not panic.
fn check_document(name: &str, doc: &str, search_collapsible: bool) {
    let (content, toc) = prepare_markdown(doc, root(), search_collapsible);
    let mut seen = std::collections::HashSet::new();
    for entry in &toc {
        assert!(
            content.contains(&format!("{{#{}}}", entry.slug)),
            "{name}: slug {:?} not in prepared content",
            entry.slug
        );
        assert!(
            seen.insert(entry.slug.clone()),
            "{name}: duplicate slug {:?}",
            entry.slug
        );
        assert!(!entry.slug.is_empty(), "{name}: empty slug");
    }
    let positions = content_heading_positions(&content, &toc);
    assert_eq!(
        positions.len(),
        toc.len(),
        "{name}: heading positions out of step"
    );
    for ((start, slug), entry) in positions.iter().zip(&toc) {
        assert_eq!(slug, &entry.slug, "{name}");
        assert!(content.is_char_boundary(*start), "{name}");
    }
}

#[test]
fn test_cases_document_survives_the_pipeline() {
    let doc = fs::read_to_string(root().join("TEST_CASES.md")).unwrap();
    check_document("TEST_CASES.md", &doc, false);
    check_document("TEST_CASES.md (expanded details)", &doc, true);
}

#[test]
fn readme_survives_the_pipeline() {
    let doc = fs::read_to_string(root().join("README.md")).unwrap();
    check_document("README.md", &doc, false);
}

#[test]
fn readme_code_fences_are_balanced() {
    let doc = fs::read_to_string(root().join("README.md")).unwrap();
    assert!(validate_code_fences(&doc).is_ok());
}

#[test]
fn preprocessing_is_a_no_op_on_documents_without_html_or_headings() {
    let doc = "plain text\n\n* a list\n* of things\n";
    assert_eq!(preprocess_html(doc), doc);
    let (out, toc) = extract_toc_and_inject_ids(doc);
    assert_eq!(out, doc);
    assert!(toc.is_empty());
}

/// A tiny deterministic generator (xorshift) so the fuzz test needs no dependency and is
/// reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        let len = items.len() as u64;
        items[(self.next() % len) as usize]
    }
}

const FRAGMENTS: &[&str] = &[
    "<",
    ">",
    "</",
    "/>",
    "<b>",
    "</b>",
    "<i>",
    "<br>",
    "<br/>",
    "<a href=\"",
    "\">",
    "</a>",
    "<img src=\"",
    "<img src=x alt=y>",
    "<table>",
    "<tr>",
    "<td>",
    "<th>",
    "</td>",
    "</tr>",
    "</table>",
    "<details>",
    "<summary>",
    "</summary>",
    "</details>",
    "<sup>",
    "</sup>",
    "<sub>",
    "<script>",
    "</script>",
    "<!--",
    "-->",
    "&amp;",
    "&",
    "\n",
    "\n\n",
    "\r\n",
    " ",
    "# ",
    "## ",
    "### {#id}",
    "```",
    "```rust\n",
    "~~~",
    "    ",
    "> ",
    "- ",
    "* ",
    "1. ",
    "|",
    "---",
    "===",
    "![",
    "](",
    ")",
    "[",
    "]",
    "`",
    "**",
    "_",
    "\u{fc}",
    "\u{65e5}\u{672c}",
    "\u{1f680}",
    "{#",
    "}",
    "text",
    "word",
];

#[test]
fn random_documents_never_panic_and_stay_consistent() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..1500 {
        let parts = 1 + rng.next() % 40;
        let doc: String = (0..parts).map(|_| rng.pick(FRAGMENTS)).collect();

        let _ = preprocess_html(&doc);
        let _ = preprocess_html_with(&doc, true);
        let _ = validate_code_fences(&doc);

        let (content, toc) = prepare_markdown(&doc, root(), case % 2 == 0);
        let positions = content_heading_positions(&content, &toc);
        assert_eq!(
            positions.len(),
            toc.len(),
            "case {case}: heading positions out of step for {doc:?}"
        );
        for (start, _) in &positions {
            assert!(content.is_char_boundary(*start), "case {case}: {doc:?}");
        }
    }
}
