//! Pure text processing for markdown documents: table-of-contents extraction, heading id
//! injection, image path rewriting, `file://` URIs and code-fence pre-screening.
//!
//! Nothing here touches the GUI, which keeps it unit-testable.

use crate::html_prep::preprocess_html_with;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The markdown extensions the viewer renders, which every pre-pass must parse with so that it
/// sees the same structure as the renderer.
fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
}

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
    let pc_opts = parser_options();

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
    let opts = parser_options();
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
#[derive(Debug, PartialEq, Eq)]
pub enum FenceError {
    /// A fence is never closed, so it swallows the rest of the document (or of its block
    /// quote or list item).
    Unclosed {
        /// 1-based line of the opening fence.
        at_line: usize,
        /// The opening fence line, e.g. ```` ```rust ````.
        header: String,
    },
    /// A fence contains what looks like another fence opener (a fence of the same kind and at
    /// least the same length, with a language tag), which almost always means the first fence
    /// was never closed and the rest of the text was swallowed into it.
    NestedOpener {
        /// 1-based line and text of the outer opening fence.
        opener: (usize, String),
        /// 1-based line and text of the suspicious inner line.
        inner: (usize, String),
    },
}

/// Checks that every fenced code block in `content` is properly closed.
///
/// pulldown-cmark decides what is a fence (so fences inside other fences, indented code, HTML
/// blocks and so on are handled exactly as the renderer will handle them), and this only
/// inspects the blocks it found. Two problems are reported:
///
/// - a fence that never gets a closing line (`CommonMark` closers are the same character,
///   at least as long as the opener, with nothing after them), and
/// - a fence that contains a line like a typed opener (see [`FenceError::NestedOpener`]),
///   the typical symptom of a forgotten closer, where the renderer would otherwise silently
///   produce a mangled page.
///
/// # Errors
///
/// Returns the first problem found, in document order.
pub fn validate_code_fences(content: &str) -> Result<(), FenceError> {
    let line_of = |offset: usize| content[..offset].matches('\n').count() + 1;

    for (event, span) in Parser::new_ext(content, parser_options()).into_offset_iter() {
        let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) = event else {
            continue;
        };
        let block = &content[span.clone()];
        let mut lines = block.split('\n');
        let header_line = lines.next().unwrap_or("");
        let header = header_line.trim_end();
        let Some((fence_char, open_len)) = fence_of(header_line) else {
            continue;
        };
        let body: Vec<&str> = lines.collect();
        // Bytes of block-quote markers / list indentation before the fence on its line.
        let container_len = span.start - content[..span.start].rfind('\n').map_or(0, |i| i + 1);

        // A typed opener inside the block means the outer fence was probably never closed.
        for (i, line) in body.iter().enumerate() {
            if is_typed_opener(line, fence_char, open_len, container_len) {
                return Err(FenceError::NestedOpener {
                    opener: (line_of(span.start), header.to_owned()),
                    inner: (
                        line_of(span.start) + 1 + i,
                        strip_container(line).trim_end().to_owned(),
                    ),
                });
            }
        }

        // pulldown-cmark's span ends at the closing fence if there is one, but includes the
        // trailing newline(s) if the block ran to the end of its container unclosed.
        let closed = body
            .last()
            .is_some_and(|last| is_closer(last, fence_char, open_len));
        if !closed {
            return Err(FenceError::Unclosed {
                at_line: line_of(span.start),
                header: header.to_owned(),
            });
        }
    }
    Ok(())
}

/// Removes the leading indentation and block-quote markers that can precede a fence line.
fn strip_container(line: &str) -> &str {
    line.trim_start_matches(|c: char| c == '>' || c.is_whitespace())
}

/// The fence character and length of an opening fence line.
fn fence_of(line: &str) -> Option<(char, usize)> {
    let line = strip_container(line);
    let ch = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = line.chars().take_while(|&c| c == ch).count();
    (len >= 3).then_some((ch, len))
}

/// Whether `line` closes a fence of `fence_char` opened with `open_len` of them.
fn is_closer(line: &str, fence_char: char, open_len: usize) -> bool {
    let t = strip_container(line).trim_end();
    t.chars().count() >= open_len && t.chars().all(|c| c == fence_char)
}

/// Whether `line` looks like the opening of a new fence with a language tag, of the same kind
/// as (and at least as long as) the enclosing one.
///
/// This is deliberately conservative, because a false alarm replaces a perfectly good document
/// with an error page. The line must be indented by at most three spaces beyond its container
/// (`container_len` is how many bytes of block-quote markers or list indentation precede the
/// outer fence; more indentation than that is content, not a fence), and the info string must
/// be a single language-like word (`rust`, `c++`, `objective-c`).
fn is_typed_opener(line: &str, fence_char: char, open_len: usize, container_len: usize) -> bool {
    // Drop the container prefix, which is spaces and `>` markers.
    let mut rest = line;
    for _ in 0..container_len {
        match rest.strip_prefix([' ', '>']) {
            Some(r) => rest = r,
            None => break,
        }
    }
    let t = rest.trim_start_matches(' ');
    if rest.len() - t.len() > 3 {
        return false;
    }
    let len = t.chars().take_while(|&c| c == fence_char).count();
    if len < open_len {
        return false;
    }
    let info = t[len * fence_char.len_utf8()..].trim();
    !info.is_empty()
        && info
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '+' | '#' | '.' | '-' | '_'))
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
#[must_use]
pub fn fence_err_brief(err: &FenceError, path: &Path) -> String {
    let p = path.display();
    match err {
        FenceError::Unclosed { at_line, header } => {
            format!("malformed code fence in '{p}': '{header}' (line {at_line}) is never closed")
        }
        FenceError::NestedOpener {
            opener: (ol, oh),
            inner: (il, ih),
        } => format!(
            "malformed code fence in '{p}': unclosed fence between \
             '{oh}' (line {ol}) and '{ih}' (line {il})"
        ),
    }
}

/// Build a markdown error page to display instead of a broken file.
/// The error renders nicely in the viewer; the user can fix the file and
/// press `mod_key`-R (Cmd/Ctrl-R) to reload.
#[must_use]
pub fn fence_error_content(path: &Path, err: &FenceError, mod_key: &str) -> String {
    let file = path.display();
    let problem = match err {
        FenceError::Unclosed { at_line, header } => {
            format!("`{header}` on line {at_line} is never closed.")
        }
        FenceError::NestedOpener {
            opener: (ol, oh),
            inner: (il, ih),
        } => format!(
            "between `{oh}` on line {ol} and `{ih}` on line {il}, a code fence was probably \
             left unclosed."
        ),
    };
    format!(
        "# \u{26a0} Malformed Code Fence\n\n\
         Cannot render `{file}`.\n\n\
         **Unclosed code fence detected** \u{2014} {problem}\n\n\
         Please fix the file, then press **{mod_key}-R** to reload."
    )
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

    fn unclosed(at_line: usize, header: &str) -> FenceError {
        FenceError::Unclosed {
            at_line,
            header: header.to_owned(),
        }
    }

    #[test]
    fn fences_balanced_are_ok() {
        for doc in [
            "",
            "no fences",
            "```rust\ncode\n```\n",
            "```rust\ncode\n```",
            "```\na\n```\n\n~~~\nb\n~~~\n",
            "```rust\na\n```\n```toml\nb\n```\n",
            "   ```\n   a\n   ```\n",
            "```\n\n\n```\n",
            "```\n```\n",
        ] {
            assert_eq!(validate_code_fences(doc), Ok(()), "{doc:?}");
        }
    }

    #[test]
    fn fences_unclosed_at_end_of_document() {
        assert_eq!(validate_code_fences("```\ncode\n"), Err(unclosed(1, "```")));
        assert_eq!(validate_code_fences("```"), Err(unclosed(1, "```")));
        assert_eq!(validate_code_fences("```\n"), Err(unclosed(1, "```")));
        assert_eq!(
            validate_code_fences("text\n\n~~~rust\nfn a() {}"),
            Err(unclosed(3, "~~~rust"))
        );
        assert_eq!(
            validate_code_fences("```rust\nx\n\n\n"),
            Err(unclosed(1, "```rust"))
        );
    }

    #[test]
    fn fences_forgotten_closer_before_a_typed_opener_is_reported_with_locations() {
        let doc = "```rust\nfn a() {}\n\n```toml\nx = 1\n```\n";
        assert_eq!(
            validate_code_fences(doc),
            Err(FenceError::NestedOpener {
                opener: (1, "```rust".to_owned()),
                inner: (4, "```toml".to_owned()),
            })
        );
    }

    #[test]
    fn fences_closer_with_trailing_text_does_not_close() {
        // "``` trailing" can't close the fence, and looks like an opener, so it is reported
        // as such, with both locations.
        assert_eq!(
            validate_code_fences("```\ncode\n``` trailing\n"),
            Err(FenceError::NestedOpener {
                opener: (1, "```".to_owned()),
                inner: (3, "``` trailing".to_owned()),
            })
        );
    }

    #[test]
    fn fences_closer_must_be_at_least_as_long_as_the_opener() {
        assert_eq!(
            validate_code_fences("````\ncode\n```\n"),
            Err(unclosed(1, "````"))
        );
        assert_eq!(validate_code_fences("```\ncode\n`````\n"), Ok(()));
    }

    #[test]
    fn fences_closer_must_use_the_same_character() {
        assert_eq!(
            validate_code_fences("~~~\ncode\n```\n"),
            Err(unclosed(1, "~~~"))
        );
    }

    #[test]
    fn fences_indented_four_spaces_are_not_fences() {
        assert_eq!(validate_code_fences("    ```\n    code\n"), Ok(()));
    }

    #[test]
    fn fences_inside_block_quotes_and_lists() {
        assert_eq!(
            validate_code_fences("> ```rust\n> x\n> ```\n\nafter\n"),
            Ok(())
        );
        assert_eq!(
            validate_code_fences("- item\n  ```rust\n  x\n  ```\n- next\n"),
            Ok(())
        );
        assert_eq!(
            validate_code_fences("- item\n  ```rust\n  x\n\n- next\n"),
            Err(unclosed(2, "```rust"))
        );
    }

    #[test]
    fn fences_text_that_is_not_a_fence_is_ignored() {
        for doc in [
            "``` not `a fence` because of the backtick\n",
            "inline ```code``` here\n",
            "``\nnot enough\n``\n",
            "~~ two tildes\n",
        ] {
            assert_eq!(validate_code_fences(doc), Ok(()), "{doc:?}");
        }
    }

    #[test]
    fn fences_backticks_inside_tilde_fence_are_content() {
        assert_eq!(validate_code_fences("~~~\n```\n~~~\n"), Ok(()));
        assert_eq!(validate_code_fences("~~~\n```rust\nx\n```\n~~~\n"), Ok(()));
    }

    #[test]
    fn fences_longer_outer_fence_may_contain_shorter_ones() {
        assert_eq!(
            validate_code_fences("````md\n```rust\nx\n```\n````\n"),
            Ok(())
        );
    }

    #[test]
    fn fences_first_problem_in_document_order_wins() {
        let doc = "```a\nx\n```\n\n```b\nunclosed\n";
        assert_eq!(validate_code_fences(doc), Err(unclosed(5, "```b")));
    }

    #[test]
    fn fence_error_messages_mention_the_file_and_key() {
        let path = Path::new("/tmp/doc.md");
        let open = unclosed(7, "```rust");
        let brief = fence_err_brief(&open, path);
        assert!(
            brief.contains("/tmp/doc.md") && brief.contains("line 7"),
            "{brief}"
        );
        let page = fence_error_content(path, &open, "Cmd");
        assert!(page.starts_with("# \u{26a0} Malformed Code Fence"));
        assert!(page.contains("`/tmp/doc.md`"));
        assert!(page.contains("````rust` on line 7"), "{page}");
        assert!(page.contains("**Cmd-R**"));

        let nested = FenceError::NestedOpener {
            opener: (2, "```rust".into()),
            inner: (9, "```toml".into()),
        };
        let brief = fence_err_brief(&nested, path);
        assert!(
            brief.contains("line 2") && brief.contains("line 9"),
            "{brief}"
        );
        let page = fence_error_content(path, &nested, "Ctrl");
        assert!(page.contains("line 2") && page.contains("line 9"), "{page}");
        assert!(page.contains("**Ctrl-R**"));
    }

    #[test]
    fn screen_code_fences_passes_good_text_and_replaces_bad() {
        let path = Path::new("/tmp/doc.md");
        let good = "```\nok\n```\n".to_owned();
        assert_eq!(screen_code_fences(good.clone(), path, "Cmd"), good);
        let bad = screen_code_fences("```\noops\n".to_owned(), path, "Cmd");
        assert!(bad.starts_with("# \u{26a0} Malformed Code Fence"));
    }

    #[test]
    fn fences_indented_inner_fence_is_content_not_an_opener() {
        // From TEST_CASES.md: the inner fence is indented to show it literally.
        let doc = "```html\n<details>\n    <summary>x</summary>\n\n    ```bash\n    Error\n    ```\n</details>\n```\n";
        assert_eq!(validate_code_fences(doc), Ok(()));
    }

    #[test]
    fn fences_info_must_look_like_a_language_to_count_as_an_opener() {
        // A Rust sample that happens to contain "```" followed by escapes or prose.
        for inner in [
            "```\\n\\",
            "``` ([#2315](https://x/y))",
            "```rs path/to/file.rs",
            "``` and more words",
        ] {
            let doc = format!("```rust\nlet s = 1;\n{inner}\n```\n");
            assert!(
                !matches!(
                    validate_code_fences(&doc),
                    Err(FenceError::NestedOpener { .. })
                ),
                "{inner:?}"
            );
        }
    }

    #[test]
    fn fences_language_like_tokens_are_openers() {
        for lang in [
            "rust",
            "c++",
            "c#",
            "objective-c",
            "sh",
            "R",
            "x86_64",
            "tar.gz",
        ] {
            let doc = format!("```text\nabc\n```{lang}\nmore\n```\n");
            assert!(
                matches!(
                    validate_code_fences(&doc),
                    Err(FenceError::NestedOpener { .. })
                ),
                "{lang}"
            );
        }
    }

    #[test]
    fn fences_nested_opener_inside_a_quote_or_list_is_still_found() {
        assert!(matches!(
            validate_code_fences("> ```rust\n> a\n> ```toml\n> b\n> ```\n"),
            Err(FenceError::NestedOpener { .. })
        ));
        assert!(matches!(
            validate_code_fences("- x\n  ```rust\n  a\n  ```toml\n  b\n  ```\n"),
            Err(FenceError::NestedOpener { .. })
        ));
    }
}
