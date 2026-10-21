//! Pure text processing for markdown documents: table-of-contents extraction, heading id
//! injection, image path rewriting, `file://` URIs and code-fence pre-screening.
//!
//! Nothing here touches the GUI, which keeps it unit-testable.

use crate::html_prep::preprocess_html_with;
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ─── TOC / heading extraction ──────────────────────────────────────────────────

/// An entry in the table of contents, derived from one ATX heading in the document.
#[derive(Clone)]
pub struct TocEntry {
    /// Heading depth 1–6.
    pub level: u8,
    /// Display text (raw heading text; may include inline markup such as `**bold**`).
    pub text: String,
    /// The `{#slug}` injected into the rendered content, used as the scroll target.
    pub slug: String,
}

/// Converts heading text to a URL-safe slug: lowercased, non-alphanumeric runs replaced by `-`.
/// Unicode letters and digits are kept (lowercased), so `Über uns` becomes `über-uns`.
///
/// Returns an empty string if the text has no alphanumeric characters.
pub fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut prev_sep = true; // start true to drop any leading hyphens
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            slug.push('-');
            prev_sep = true;
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Parses an ATX heading line and returns `(level, plain_text)`.
/// `plain_text` is the heading content with any trailing `{…}` attribute block stripped.
/// Returns `None` for non-heading lines, indented lines, or malformed ATX syntax.
#[expect(clippy::cast_possible_truncation)]
pub fn parse_heading_line(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    // ATX heading must have a space after the `#` run.
    let rest = line[hashes..].strip_prefix(' ')?;
    let text = rest.trim_end();
    if text.is_empty() {
        return None;
    }
    // Strip any trailing `{#id}` / `{.class}` attribute block.
    let plain = text.rfind('{').map_or(text, |brace| {
        let attr = text[brace..].trim_end();
        if attr.ends_with('}') {
            text[..brace].trim_end()
        } else {
            text
        }
    });
    Some((hashes as u8, plain))
}

/// Returns the explicit `{#id}` from a heading line, if present.
pub fn extract_heading_id(line: &str) -> Option<&str> {
    let brace = line.rfind('{')?;
    let attr = line[brace..].trim_end();
    if attr.starts_with("{#") && attr.ends_with('}') {
        Some(&attr[2..attr.len() - 1])
    } else {
        None
    }
}

/// Scans `raw` markdown, builds a `Vec<TocEntry>` from headings, and returns a version
/// of the content with `{#slug}` attributes injected into every heading that lacks one.
///
/// **Uses pulldown-cmark as the heading oracle** rather than a hand-rolled fence
/// tracker. This guarantees that the TOC and injected IDs are consistent with what
/// the renderer sees. A custom fence tracker diverges from pulldown-cmark in edge
/// cases such as XML-like tags (`<context>`, `<files>`) being treated as type-6 HTML
/// blocks that can swallow a code-fence opener — leading to headings that are
/// unreachable by the scroll mechanism.
pub fn extract_toc_and_inject_ids(raw: &str) -> (String, Vec<TocEntry>) {
    let pc_opts = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES;

    // Pass 1 — ask pulldown-cmark where headings actually begin.
    let heading_starts: Vec<usize> = Parser::new_ext(raw, pc_opts)
        .into_offset_iter()
        .filter_map(|(event, span)| {
            matches!(event, Event::Start(Tag::Heading { .. })).then_some(span.start)
        })
        .collect();

    // Pass 2 — rebuild content, injecting `{#slug}` at each pulldown-cmark-verified heading.
    let mut out = String::with_capacity(raw.len() + heading_starts.len() * 24);
    let mut toc: Vec<TocEntry> = Vec::new();
    let mut slug_counts: HashMap<String, usize> = HashMap::new();
    // Every slug handed out so far, including explicit `{#id}`s, so that a generated slug
    // can never collide with one the author wrote.
    let mut used_slugs: HashSet<String> = HashSet::new();
    for &start in &heading_starts {
        let end = raw[start..].find('\n').map_or(raw.len(), |p| start + p);
        if let Some(id) = extract_heading_id(&raw[start..end]) {
            used_slugs.insert(id.to_string());
        }
    }
    let mut pos = 0usize; // read cursor into `raw`

    for line_start in heading_starts {
        // Emit everything between the last position and this heading.
        out.push_str(&raw[pos..line_start]);

        // Extract the heading line (up to but not including the trailing `\n`).
        let newline = raw[line_start..]
            .find('\n')
            .map_or(raw.len(), |p| line_start + p);
        let line = &raw[line_start..newline];

        // Use our ATX parser to get the level and plain text.
        if let Some((level, plain_text)) = parse_heading_line(line) {
            let (slug, injected) = if let Some(id) = extract_heading_id(line) {
                (id.to_string(), line.to_string())
            } else {
                let mut base = slugify(plain_text);
                if base.is_empty() {
                    // `{#}` is not a valid id, so give punctuation-only headings a name.
                    base = "section".to_owned();
                }
                let counter = slug_counts.entry(base.clone()).or_insert(0);
                let slug = loop {
                    let candidate = if *counter == 0 {
                        base.clone()
                    } else {
                        format!("{base}-{counter}")
                    };
                    *counter += 1;
                    if used_slugs.insert(candidate.clone()) {
                        break candidate;
                    }
                };
                let with_id = format!("{} {{#{slug}}}", line.trim_end());
                (slug, with_id)
            };
            toc.push(TocEntry {
                level,
                text: plain_text.to_string(),
                slug,
            });
            out.push_str(&injected);
        } else {
            // pulldown-cmark found a heading our ATX parser doesn't recognise
            // (e.g. setext style). Pass it through without injection.
            out.push_str(line);
        }

        // Advance past the newline (or to end-of-file).
        pos = if newline < raw.len() {
            out.push('\n');
            newline + 1
        } else {
            newline
        };
    }

    // Emit the tail of the file after the last heading.
    out.push_str(&raw[pos..]);

    (out, toc)
}

/// Byte offset and slug of each TOC heading within `content` (the prepared markdown).
///
/// `toc` only contains the headings that `extract_toc_and_inject_ids` could parse as ATX
/// headings; pulldown-cmark also reports setext headings and headings nested in block quotes
/// or lists, whose span doesn't start at the `#`. Those must be skipped here too, using the
/// same test, or every later heading would be paired with the wrong TOC entry.
pub fn content_heading_positions(content: &str, toc: &[TocEntry]) -> Vec<(usize, String)> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut entries = toc.iter();
    let mut positions = Vec::with_capacity(toc.len());
    for (event, span) in Parser::new_ext(content, opts).into_offset_iter() {
        if !matches!(event, Event::Start(Tag::Heading { .. })) {
            continue;
        }
        let line_end = content[span.start..]
            .find('\n')
            .map_or(content.len(), |p| span.start + p);
        if parse_heading_line(&content[span.start..line_end]).is_none() {
            continue;
        }
        match entries.next() {
            Some(entry) => positions.push((span.start, entry.slug.clone())),
            None => break,
        }
    }
    positions
}

// ─── Image path absolutization ─────────────────────────────────────────────────

/// Rewrites relative image paths in Markdown to absolute `file://` URIs so they
/// load correctly regardless of platform CWD behaviour.
///
/// Paths that already carry a URI scheme (`http://`, `file://`, `data:`, …) are
/// left untouched. If a relative path cannot be resolved (file does not exist)
/// it is also left untouched so existing error behaviour is preserved.
///
/// Note: processes the raw text, so a path inside a fenced code block is also
/// rewritten if it matches the image syntax — an acceptable trade-off for the
/// cross-platform fix.
pub fn absolutize_image_paths(content: &str, base_dir: &Path) -> String {
    let mut out = String::with_capacity(content.len() + 128);
    let mut rest = content;

    while let Some(bang) = rest.find("![") {
        out.push_str(&rest[..bang]);
        rest = &rest[bang..];

        // Find `](`  — alt text must not contain `]`
        let Some(close_bracket) = rest.find("](") else {
            out.push_str(&rest[..2]);
            rest = &rest[2..];
            continue;
        };

        let prefix = &rest[..close_bracket + 2]; // `![alt](`
        rest = &rest[close_bracket + 2..];

        let Some(close_paren) = rest.find(')') else {
            out.push_str(prefix);
            continue;
        };

        let inner = &rest[..close_paren]; // path, possibly with `"title"`
        rest = &rest[close_paren + 1..];

        // Split optional title: `path "title"` or `path 'title'`
        let (raw_path, title_suffix) = inner
            .find(" \"")
            .or_else(|| inner.find(" '"))
            .map_or_else(|| (inner.trim(), ""), |i| (&inner[..i], &inner[i..]));

        let is_schemed = raw_path.starts_with("http://")
            || raw_path.starts_with("https://")
            || raw_path.starts_with("file://")
            || raw_path.starts_with("data:");

        out.push_str(prefix);
        if is_schemed {
            out.push_str(inner);
        } else if let Ok(abs) = base_dir.join(raw_path).canonicalize() {
            out.push_str(&path_to_file_uri(&abs));
            out.push_str(title_suffix);
        } else {
            // File not found — leave unchanged so the viewer shows a
            // broken-image placeholder rather than silently doing nothing.
            out.push_str(inner);
        }
        out.push(')');
    }
    out.push_str(rest);
    out
}

/// Turns raw file text into what `egui_commonmark` displays: embedded HTML is
/// rewritten (see `html_prep`), then heading `{#slug}` ids are injected (and the
/// TOC built), then relative image paths are absolutized.
pub fn prepare_markdown(
    raw: &str,
    base_dir: &Path,
    search_collapsible: bool,
) -> (String, Vec<TocEntry>) {
    let prepared = preprocess_html_with(raw, search_collapsible);
    let (id_injected, toc) = extract_toc_and_inject_ids(&prepared);
    (absolutize_image_paths(&id_injected, base_dir), toc)
}

/// Builds a `file://` URI for `path`, percent-encoding anything that isn't safe in a URI
/// (spaces, `#`, `%`, parentheses, non-ASCII, ...). `egui`'s file loader decodes these.
pub fn path_to_file_uri(path: &Path) -> String {
    file_uri_from_str(&path.to_string_lossy(), cfg!(windows))
}

/// Platform-independent core of [`path_to_file_uri`]; `windows` selects Windows path syntax
/// so that it can be tested on any platform.
pub fn file_uri_from_str(raw: &str, windows: bool) -> String {
    let mut s = if windows {
        raw.replace('\\', "/")
    } else {
        raw.to_owned()
    };
    if windows {
        // `canonicalize()` yields verbatim paths: `\\?\C:\dir` and `\\?\UNC\server\share`.
        if let Some(rest) = s.strip_prefix("//?/UNC/") {
            s = format!("//{rest}");
        } else if let Some(rest) = s.strip_prefix("//?/") {
            s = rest.to_owned();
        }
    }

    let mut encoded = String::with_capacity(s.len() + 16);
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            encoded.push(char::from(b));
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }

    if encoded.starts_with("//") {
        format!("file:{encoded}") // UNC: file://server/share/...
    } else if encoded.starts_with('/') {
        format!("file://{encoded}") // file:// + /unix/path = file:///unix/path
    } else {
        format!("file:///{encoded}") // file:/// + C:/... = file:///C:/...
    }
}

// ─── Code fence pre-screening ────────────────────────────────────────────────

/// A code-fence problem detected before rendering.
pub enum FenceError {
    /// Total fence boundary count is odd — at least one fence is unclosed.
    OddCount { total: usize },
    /// An even-numbered boundary carries a language tag, meaning the preceding
    /// typed opener was never closed.
    UnclosedBeforeTyped {
        at_line: usize,
        at_header: String,
        prev_typed: Option<(usize, String)>,
    },
}

/// Scan `content` for fence problems without a full parser.
///
/// A fence boundary is a line with ≤ 3 leading spaces that starts with a triple backtick
/// or triple tilde (`~~~`) (matching the same rule used by `extract_toc_and_inject_ids`).
///
/// Two invariants are checked:
/// - **(a)** Total boundary count must be even — odd means at least one unclosed.
/// - **(b)** Every *typed* boundary (one with a language tag) must be
///   odd-numbered in sequence; an even-numbered typed boundary means the
///   previous typed opener was never closed.
pub fn validate_code_fences(content: &str) -> Result<(), FenceError> {
    let mut fence_count = 0usize;
    let mut last_typed: Option<(usize, String)> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            continue;
        }
        let is_backtick = trimmed.starts_with("```");
        let is_tilde = trimmed.starts_with("~~~");
        if !is_backtick && !is_tilde {
            continue;
        }

        fence_count += 1;

        // Strip all leading fence chars to get the language tag (if any).
        let lang = if is_backtick {
            trimmed.trim_start_matches('`')
        } else {
            trimmed.trim_start_matches('~')
        }
        .trim();

        if !lang.is_empty() {
            if fence_count.is_multiple_of(2) {
                // Even-numbered typed boundary → previous opener was never closed.
                return Err(FenceError::UnclosedBeforeTyped {
                    at_line: line_num,
                    at_header: trimmed.trim_end().to_string(),
                    prev_typed: last_typed,
                });
            }
            last_typed = Some((line_num, trimmed.trim_end().to_string()));
        }
    }

    if !fence_count.is_multiple_of(2) {
        return Err(FenceError::OddCount { total: fence_count });
    }
    Ok(())
}

/// Pre-screens `raw` for malformed code fences. Returns it unchanged if it's fine, otherwise
/// logs the problem and returns an error page (see [`fence_error_content`]) to show instead.
#[must_use]
pub fn screen_code_fences(raw: String, path: &Path, mod_key: &str) -> String {
    match validate_code_fences(&raw) {
        Ok(()) => raw,
        Err(err) => {
            eprintln!("workman: {}", fence_err_brief(&err, path));
            fence_error_content(path, &err, mod_key)
        }
    }
}

/// Return a one-line description of a fence error suitable for `eprintln!`.
pub fn fence_err_brief(err: &FenceError, path: &Path) -> String {
    let p = path.display();
    match err {
        FenceError::OddCount { total } => {
            format!("malformed code fence in '{p}': odd boundary count ({total} total)")
        }
        FenceError::UnclosedBeforeTyped {
            at_line,
            at_header,
            prev_typed,
        } => match prev_typed {
            Some((pl, ph)) => format!(
                "malformed code fence in '{p}': unclosed fence between \
                     '{ph}' (line {pl}) and '{at_header}' (line {at_line})"
            ),
            None => format!(
                "malformed code fence in '{p}': unclosed fence before \
                     '{at_header}' (line {at_line})"
            ),
        },
    }
}

/// Build a markdown error page to display instead of a broken file.
/// The error renders nicely in the viewer; the user can fix the file and
/// press `mod_key`-R (Cmd/Ctrl-R) to reload.
pub fn fence_error_content(path: &Path, err: &FenceError, mod_key: &str) -> String {
    let file = path.display();
    match err {
        FenceError::OddCount { total } => format!(
            "# \u{26a0} Malformed Code Fence\n\n\
             Cannot render `{file}`.\n\n\
             **Odd number of fence boundaries detected ({total} total)** \u{2014} \
             at least one code fence is unclosed.\n\n\
             Please fix the file, then press **{mod_key}-R** to reload."
        ),
        FenceError::UnclosedBeforeTyped {
            at_line,
            at_header,
            prev_typed,
        } => {
            let location = match prev_typed {
                Some((pl, ph)) => {
                    format!("between `{ph}` on line {pl} and `{at_header}` on line {at_line}")
                }
                None => format!("before `{at_header}` on line {at_line}"),
            };
            format!(
                "# \u{26a0} Malformed Code Fence\n\n\
                 Cannot render `{file}`.\n\n\
                 **Unclosed code fence detected** \u{2014} {location}.\n\n\
                 Please fix the file, then press **{mod_key}-R** to reload."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("workman-md-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Canonical, so expectations match what `canonicalize()` returns (e.g. /private/var).
        dir.canonicalize().unwrap()
    }

    fn slugs(toc: &[TocEntry]) -> Vec<&str> {
        toc.iter().map(|e| e.slug.as_str()).collect()
    }

    // ── slugify ────────────────────────────────────────────────────────────────────────

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  Leading and trailing  "), "leading-and-trailing");
        assert_eq!(slugify("a -- b __ c"), "a-b-c");
        assert_eq!(slugify("C++ & Rust: a love story!"), "c-rust-a-love-story");
        assert_eq!(slugify("Section 2.1"), "section-2-1");
    }

    #[test]
    fn slugify_empty_and_punctuation_only() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("?!..."), "");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn slugify_keeps_and_lowercases_unicode() {
        assert_eq!(slugify("\u{dc}ber uns"), "\u{fc}ber-uns");
        assert_eq!(
            slugify("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{41c}\u{438}\u{440}"),
            "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}-\u{43c}\u{438}\u{440}"
        );
        assert_eq!(
            slugify("\u{65e5}\u{672c}\u{8a9e}"),
            "\u{65e5}\u{672c}\u{8a9e}"
        );
        // Emoji are not alphanumeric, so they act as separators.
        assert_eq!(slugify("\u{1f680} Launch"), "launch");
    }

    // ── parse_heading_line / extract_heading_id ────────────────────────────────────────

    #[test]
    fn heading_line_levels() {
        for level in 1..=6u8 {
            let line = format!("{} Title", "#".repeat(usize::from(level)));
            assert_eq!(parse_heading_line(&line), Some((level, "Title")));
        }
    }

    #[test]
    fn heading_line_rejects_non_headings() {
        assert_eq!(parse_heading_line("####### seven"), None);
        assert_eq!(parse_heading_line("#NoSpace"), None);
        assert_eq!(parse_heading_line("# "), None);
        assert_eq!(parse_heading_line("#"), None);
        assert_eq!(parse_heading_line("plain text"), None);
        assert_eq!(parse_heading_line(" # indented"), None);
        assert_eq!(parse_heading_line(""), None);
    }

    #[test]
    fn heading_line_strips_trailing_attribute_block() {
        assert_eq!(parse_heading_line("## Title {#custom}"), Some((2, "Title")));
        assert_eq!(parse_heading_line("## Title {.class}"), Some((2, "Title")));
        // An unterminated brace is part of the text.
        assert_eq!(parse_heading_line("## Set {a, b"), Some((2, "Set {a, b")));
        assert_eq!(parse_heading_line("# {#only-id}"), Some((1, "")));
    }

    #[test]
    fn heading_id_extraction() {
        assert_eq!(extract_heading_id("## Title {#custom}"), Some("custom"));
        assert_eq!(extract_heading_id("## Title {#custom}  "), Some("custom"));
        assert_eq!(extract_heading_id("## Title {.class}"), None);
        assert_eq!(extract_heading_id("## Title"), None);
        assert_eq!(extract_heading_id("## Title {#unterminated"), None);
    }

    // ── extract_toc_and_inject_ids ─────────────────────────────────────────────────────

    #[test]
    fn toc_empty_and_headingless_input() {
        let (out, toc) = extract_toc_and_inject_ids("");
        assert_eq!(out, "");
        assert!(toc.is_empty());

        let text = "just text\n\nmore text\n";
        let (out, toc) = extract_toc_and_inject_ids(text);
        assert_eq!(out, text);
        assert!(toc.is_empty());
    }

    #[test]
    fn toc_injects_ids_and_records_levels() {
        let (out, toc) = extract_toc_and_inject_ids("# One\n\ntext\n\n## Two words\n");
        assert_eq!(out, "# One {#one}\n\ntext\n\n## Two words {#two-words}\n");
        assert_eq!(slugs(&toc), ["one", "two-words"]);
        assert_eq!(toc[0].level, 1);
        assert_eq!(toc[1].level, 2);
        assert_eq!(toc[1].text, "Two words");
    }

    #[test]
    fn toc_duplicate_headings_get_numeric_suffixes() {
        let (_, toc) = extract_toc_and_inject_ids("# A\n## A\n### A\n");
        assert_eq!(slugs(&toc), ["a", "a-1", "a-2"]);
    }

    #[test]
    fn toc_explicit_ids_are_preserved() {
        let (out, toc) = extract_toc_and_inject_ids("# Title {#mine}\n");
        assert_eq!(out, "# Title {#mine}\n");
        assert_eq!(slugs(&toc), ["mine"]);
        assert_eq!(toc[0].text, "Title");
    }

    #[test]
    fn toc_generated_slug_never_collides_with_explicit_id() {
        // The explicit `{#intro}` comes later but must still be reserved.
        let (_, toc) = extract_toc_and_inject_ids("# Intro\n\n# Other {#intro}\n");
        let s = slugs(&toc);
        assert_eq!(s.len(), 2);
        assert_ne!(s[0], s[1]);
        assert_eq!(s[1], "intro");
    }

    #[test]
    fn toc_punctuation_only_heading_gets_a_valid_slug() {
        let (out, toc) = extract_toc_and_inject_ids("# ???\n\n# !!!\n");
        assert_eq!(slugs(&toc), ["section", "section-1"]);
        assert!(!out.contains("{#}"));
    }

    #[test]
    fn toc_unicode_headings() {
        let (out, toc) = extract_toc_and_inject_ids("# \u{dc}ber uns\n");
        assert_eq!(slugs(&toc), ["\u{fc}ber-uns"]);
        assert_eq!(out, "# \u{dc}ber uns {#\u{fc}ber-uns}\n");
    }

    #[test]
    fn toc_ignores_headings_in_code_fences() {
        let raw = "```\n# not a heading\n```\n\n~~~\n## nor this\n~~~\n\n# Real\n";
        let (out, toc) = extract_toc_and_inject_ids(raw);
        assert_eq!(slugs(&toc), ["real"]);
        assert!(out.contains("# not a heading\n"), "fence content untouched");
        assert!(out.contains("## nor this\n"));
    }

    #[test]
    fn toc_ignores_headings_in_indented_code_and_html_comments() {
        let (_, toc) =
            extract_toc_and_inject_ids("para\n\n    # code\n\n<!--\n# hidden\n-->\n\n# Shown\n");
        assert_eq!(slugs(&toc), ["shown"]);
    }

    #[test]
    fn toc_heading_on_last_line_without_newline() {
        let (out, toc) = extract_toc_and_inject_ids("text\n\n# Last");
        assert_eq!(out, "text\n\n# Last {#last}");
        assert_eq!(slugs(&toc), ["last"]);
    }

    #[test]
    fn toc_handles_crlf() {
        let (out, toc) = extract_toc_and_inject_ids("# Title\r\n\r\ntext\r\n## Next\r\n");
        assert_eq!(slugs(&toc), ["title", "next"]);
        assert_eq!(toc[0].text, "Title", "no stray carriage return");
        assert!(out.contains("# Title {#title}"));
        assert!(out.contains("text\r\n"), "non-heading lines are untouched");
    }

    #[test]
    fn toc_skips_setext_headings_but_keeps_atx_ones() {
        let (out, toc) = extract_toc_and_inject_ids("Setext\n======\n\n# Atx\n");
        assert_eq!(slugs(&toc), ["atx"]);
        assert!(out.starts_with("Setext\n======\n"));
    }

    #[test]
    fn toc_headings_inside_block_quotes_and_lists_are_recognised() {
        let (_, toc) = extract_toc_and_inject_ids("> ## Quoted\n\n- ### Listed\n\n# Plain\n");
        assert_eq!(slugs(&toc), ["quoted", "listed", "plain"]);
    }

    // ── content_heading_positions ──────────────────────────────────────────────────────

    #[test]
    fn heading_positions_skip_headings_the_toc_skips() {
        // A setext heading is reported by pulldown-cmark but is not an ATX heading, so it is
        // not in the TOC. The headings after it must still be paired with the right slugs
        // (previously every later heading was shifted by one).
        let raw = "# One\n\nSetext\n======\n\n> ## Quoted\n\n## Two\n\n### Three\n";
        let (content, toc) = extract_toc_and_inject_ids(raw);
        assert_eq!(slugs(&toc), ["one", "quoted", "two", "three"]);

        let positions = content_heading_positions(&content, &toc);
        assert_eq!(positions.len(), toc.len());
        for ((start, slug), entry) in positions.iter().zip(&toc) {
            assert_eq!(slug, &entry.slug);
            let line = content[*start..].lines().next().unwrap();
            assert!(
                line.contains(&format!("{{#{slug}}}")),
                "line {line:?} should carry {{#{slug}}}"
            );
        }
    }

    #[test]
    fn heading_positions_ignore_headings_in_code_fences() {
        let raw = "```\n# not a heading\n```\n\n# Real\n";
        let (content, toc) = extract_toc_and_inject_ids(raw);
        let positions = content_heading_positions(&content, &toc);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].1, "real");
    }

    #[test]
    fn heading_positions_empty() {
        assert!(content_heading_positions("", &[]).is_empty());
        assert!(content_heading_positions("no headings", &[]).is_empty());
    }

    // ── absolutize_image_paths ─────────────────────────────────────────────────────────

    #[test]
    fn image_relative_existing_file_becomes_file_uri() {
        let dir = temp_dir("img-ok");
        fs::write(dir.join("pic.png"), b"x").unwrap();
        let uri = path_to_file_uri(&dir.join("pic.png"));
        let out = absolutize_image_paths("before ![alt](pic.png) after", &dir);
        assert_eq!(out, format!("before ![alt]({uri}) after"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_title_is_preserved() {
        let dir = temp_dir("img-title");
        fs::write(dir.join("pic.png"), b"x").unwrap();
        let uri = path_to_file_uri(&dir.join("pic.png"));
        assert_eq!(
            absolutize_image_paths("![a](pic.png \"The title\")", &dir),
            format!("![a]({uri} \"The title\")")
        );
        assert_eq!(
            absolutize_image_paths("![a](pic.png 'single')", &dir),
            format!("![a]({uri} 'single')")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_missing_file_is_left_alone() {
        let dir = temp_dir("img-missing");
        let text = "![a](nope.png) and ![b](nope.png \"t\")";
        assert_eq!(absolutize_image_paths(text, &dir), text);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_schemed_urls_are_left_alone() {
        let dir = temp_dir("img-scheme");
        let text = "![a](http://x/y.png) ![b](https://x/y.png \"t\") ![c](file:///tmp/y.png) ![d](data:image/png;base64,AAAA)";
        assert_eq!(absolutize_image_paths(text, &dir), text);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_malformed_syntax_is_passed_through() {
        let dir = temp_dir("img-bad");
        for text in [
            "",
            "no images here",
            "![ unterminated alt",
            "![alt](unterminated.png",
            "![alt] (space.png)",
            "!![a]",
        ] {
            assert_eq!(absolutize_image_paths(text, &dir), text, "{text:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_several_on_one_line_and_in_subdirectories() {
        let dir = temp_dir("img-multi");
        fs::create_dir_all(dir.join("sub dir")).unwrap();
        fs::write(dir.join("a.png"), b"x").unwrap();
        fs::write(dir.join("sub dir").join("b.png"), b"x").unwrap();
        let a = path_to_file_uri(&dir.join("a.png"));
        let b = path_to_file_uri(&dir.join("sub dir").join("b.png"));
        assert!(b.contains("sub%20dir"));
        // A space in a source path is rewritten into a valid, percent-encoded destination.
        let out = absolutize_image_paths("![1](a.png)![2](./sub dir/b.png)", &dir);
        assert_eq!(out, format!("![1]({a})![2]({b})"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_dot_dot_is_resolved() {
        let dir = temp_dir("img-dotdot");
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join("top.png"), b"x").unwrap();
        let uri = path_to_file_uri(&dir.join("top.png"));
        assert_eq!(
            absolutize_image_paths("![t](../top.png)", &dir.join("docs")),
            format!("![t]({uri})")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // ── file URIs ──────────────────────────────────────────────────────────────────────

    #[test]
    fn uri_unix_path() {
        assert_eq!(
            file_uri_from_str("/home/me/doc.md", false),
            "file:///home/me/doc.md"
        );
    }

    #[test]
    fn uri_encodes_unsafe_characters() {
        assert_eq!(
            file_uri_from_str("/a b/c#d/100%/(x).png", false),
            "file:///a%20b/c%23d/100%25/%28x%29.png"
        );
        // Non-ASCII is encoded as UTF-8 bytes.
        assert_eq!(
            file_uri_from_str("/na\u{ef}ve", false),
            "file:///na%C3%AFve"
        );
        assert_eq!(
            file_uri_from_str("/q?x=1&y", false),
            "file:///q%3Fx%3D1%26y"
        );
    }

    #[test]
    fn uri_backslash_is_literal_on_unix() {
        assert_eq!(file_uri_from_str("/a\\b", false), "file:///a%5Cb");
    }

    #[test]
    fn uri_windows_drive_path() {
        assert_eq!(
            file_uri_from_str(r"C:\Users\me\my doc.md", true),
            "file:///C:/Users/me/my%20doc.md"
        );
    }

    #[test]
    fn uri_windows_verbatim_prefixes() {
        assert_eq!(
            file_uri_from_str(r"\\?\C:\dir\a.md", true),
            "file:///C:/dir/a.md"
        );
        assert_eq!(
            file_uri_from_str(r"\\?\UNC\server\share\a.md", true),
            "file://server/share/a.md"
        );
        assert_eq!(
            file_uri_from_str(r"\\server\share\a.md", true),
            "file://server/share/a.md"
        );
    }

    // ── prepare_markdown ───────────────────────────────────────────────────────────────

    #[test]
    fn prepare_markdown_runs_the_whole_pipeline() {
        let dir = temp_dir("prepare");
        fs::write(dir.join("pic.png"), b"x").unwrap();
        let uri = path_to_file_uri(&dir.join("pic.png"));
        let (content, toc) =
            prepare_markdown("# Title\n\n<b>bold</b> ![p](pic.png)\n", &dir, false);
        assert_eq!(slugs(&toc), ["title"]);
        assert!(content.contains("# Title {#title}"));
        assert!(content.contains(&format!("![p]({uri})")));
        assert!(
            content.contains("**bold**"),
            "inline HTML is lowered: {content}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Every TOC slug must be reachable in the prepared content, whatever the document.
    #[test]
    fn prepare_markdown_toc_slugs_all_appear_in_content() {
        let dir = temp_dir("prepare-all");
        let doc = "# A\n\nSetext\n---\n\n## A\n\n> ## Q\n\n```\n# no\n```\n\n### \u{dc}ber ???\n";
        let (content, toc) = prepare_markdown(doc, &dir, false);
        for entry in &toc {
            assert!(
                content.contains(&format!("{{#{}}}", entry.slug)),
                "missing {} in {content}",
                entry.slug
            );
        }
        let positions = content_heading_positions(&content, &toc);
        assert_eq!(positions.len(), toc.len());
        let _ = fs::remove_dir_all(&dir);
    }

    // ── code fences ────────────────────────────────────────────────────────────────────

    #[test]
    fn fences_balanced_are_ok() {
        assert!(validate_code_fences("").is_ok());
        assert!(validate_code_fences("no fences").is_ok());
        assert!(validate_code_fences("```rust\ncode\n```\n").is_ok());
        assert!(validate_code_fences("```\na\n```\n\n~~~\nb\n~~~\n").is_ok());
        assert!(validate_code_fences("```rust\na\n```\n```toml\nb\n```\n").is_ok());
        assert!(
            validate_code_fences("   ```\n   a\n   ```\n").is_ok(),
            "up to 3 spaces"
        );
    }

    #[test]
    fn fences_odd_count_is_reported() {
        match validate_code_fences("```\ncode\n") {
            Err(FenceError::OddCount { total }) => assert_eq!(total, 1),
            other => panic!(
                "unexpected {:?}",
                other.err().map(|e| fence_err_brief(&e, Path::new("x")))
            ),
        }
    }

    #[test]
    fn fences_unclosed_typed_opener_is_reported_with_locations() {
        let doc = "```rust\nfn a() {}\n\n```toml\nx = 1\n```\n";
        match validate_code_fences(doc) {
            Err(FenceError::UnclosedBeforeTyped {
                at_line,
                at_header,
                prev_typed,
            }) => {
                assert_eq!(at_line, 4);
                assert_eq!(at_header, "```toml");
                assert_eq!(prev_typed, Some((1, "```rust".to_owned())));
            }
            _ => panic!("expected UnclosedBeforeTyped"),
        }
    }

    #[test]
    fn fences_indented_four_spaces_are_not_fences() {
        assert!(validate_code_fences("    ```\n    code\n").is_ok());
    }

    #[test]
    fn fence_error_messages_mention_the_file_and_key() {
        let path = Path::new("/tmp/doc.md");
        let odd = FenceError::OddCount { total: 3 };
        assert!(fence_err_brief(&odd, path).contains("/tmp/doc.md"));
        assert!(fence_err_brief(&odd, path).contains("3 total"));
        let page = fence_error_content(path, &odd, "Cmd");
        assert!(page.starts_with("# \u{26a0} Malformed Code Fence"));
        assert!(page.contains("`/tmp/doc.md`"));
        assert!(page.contains("**Cmd-R**"));

        let typed = FenceError::UnclosedBeforeTyped {
            at_line: 9,
            at_header: "```toml".into(),
            prev_typed: Some((2, "```rust".into())),
        };
        let brief = fence_err_brief(&typed, path);
        assert!(
            brief.contains("line 2") && brief.contains("line 9"),
            "{brief}"
        );
        assert!(fence_error_content(path, &typed, "Ctrl").contains("**Ctrl-R**"));

        let first = FenceError::UnclosedBeforeTyped {
            at_line: 5,
            at_header: "```c".into(),
            prev_typed: None,
        };
        assert!(fence_err_brief(&first, path).contains("before"));
        let page = fence_error_content(path, &first, "Ctrl");
        assert!(
            page.contains("before `") && page.contains("on line 5"),
            "{page}"
        );
    }

    #[test]
    fn screen_code_fences_passes_good_text_and_replaces_bad() {
        let path = Path::new("/tmp/doc.md");
        let good = "```\nok\n```\n".to_owned();
        assert_eq!(screen_code_fences(good.clone(), path, "Cmd"), good);
        let bad = screen_code_fences("```\noops\n".to_owned(), path, "Cmd");
        assert!(bad.starts_with("# \u{26a0} Malformed Code Fence"));
    }

    // Known weaknesses of the fence pre-screen (it counts markers rather than parsing).
    // These documents are valid CommonMark, yet would be replaced by the error page.

    #[test]
    #[ignore = "known issue: a ``` line inside a ~~~ fence is counted as a boundary"]
    fn fences_backticks_inside_tilde_fence_are_content() {
        assert!(validate_code_fences("~~~\n```\n~~~\n").is_ok());
    }

    #[test]
    #[ignore = "known issue: fence length is not tracked, so a longer outer fence is misread"]
    fn fences_longer_outer_fence_may_contain_shorter_ones() {
        assert!(validate_code_fences("````md\n```rust\nx\n```\n````\n").is_ok());
    }
}
