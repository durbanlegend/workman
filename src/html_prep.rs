//! Markdown pre-processing that makes embedded HTML work with `egui_commonmark`.
//!
//! `egui_commonmark` only hands *block-level* HTML to `render_html_fn`. Two
//! things it can't do, which this module works around by rewriting the
//! Markdown text before it is displayed:
//!
//! 1. **Inline HTML** (`<b>`, `<u>`, `<kbd>`, `<br>`, `<sub>`, `<a href>` ...)
//!    arrives as `Event::InlineHtml` and is printed verbatim. We rewrite the
//!    tags into the closest Markdown (`**`, `` ` ``, hard breaks, Unicode
//!    sub/superscripts, links, ...). Tags with no Markdown equivalent are
//!    approximated: `<u>`/`<ins>` become italics and `<mark>` becomes bold.
//!
//! 2. **HTML blocks split by Markdown** such as
//!    ```text
//!    <details>
//!    <summary>Title</summary>
//!
//!    ```code```
//!    </details>
//!    ```
//!    CommonMark ends an HTML block at a blank line, so the opening tags, the
//!    Markdown in between and the closing tags are three unrelated top-level
//!    blocks. We convert the Markdown in between to HTML and merge everything
//!    into a single HTML block, which `html_render` can then nest properly.
//!
//! 3. **Block HTML that has a native Markdown form** (a standalone `<table>`
//!    or `<iframe>`) is lowered to Markdown. Anything `html_fn` draws is
//!    invisible to the viewer's search, whereas native Markdown gets search,
//!    match highlighting, selection and scroll tracking for free.

use crate::html_render::{collapse, tag_depth};
use ego_tree::NodeRef;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html::push_html};
use scraper::{ElementRef, Html, Node};
use std::collections::HashSet;
use std::ops::Range;

fn opts() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// How `<details>` is handled; the trade-off is collapsibility vs. search.
///
/// * `true`: lowered to native Markdown: a bold summary line followed by the
///   body, always expanded. Fully searchable/highlightable, and code blocks in
///   the body keep syntax highlighting, but it can't be collapsed.
/// * `false`: rendered by `html_render` as a real collapsible section, but
///   text drawn there is invisible to the viewer's search.
pub const EXPAND_DETAILS: bool = false;

/// Entry point. Cheap no-op for documents without any `<`.
#[must_use]
pub fn preprocess_html(md: &str) -> String {
    preprocess_html_with(md, EXPAND_DETAILS)
}

/// [`preprocess_html`] with an explicit choice for [`EXPAND_DETAILS`].
#[must_use]
pub fn preprocess_html_with(md: &str, expand_details: bool) -> String {
    if !md.contains('<') {
        return md.to_owned();
    }
    rewrite_inline_html(&lower_blocks(&merge_split_blocks(md, expand_details)))
}

// ---------------------------------------------------------------------------
// Split block merging
// ---------------------------------------------------------------------------

struct Block {
    html: bool,
    range: Range<usize>,
}

fn top_level_blocks(md: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    for (ev, range) in Parser::new_ext(md, opts()).into_offset_iter() {
        match ev {
            Event::Start(tag) => {
                if depth == 0 {
                    blocks.push(Block {
                        html: matches!(tag, Tag::HtmlBlock),
                        range,
                    });
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ if depth == 0 => blocks.push(Block { html: false, range }),
            _ => {}
        }
    }
    blocks
}

/// A blank line would end the merged HTML block, so replace blank lines with
/// an (invisible) HTML comment.
fn unblank(html: &str) -> String {
    html.lines()
        .map(|l| if l.trim().is_empty() { "<!---->" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Some(title)` if `open`/`close` are exactly `<details>[<summary>..</summary>]`
/// and `</details>`, i.e. the pair can be lowered to Markdown without loss
/// (other than collapsibility).
fn details_title(open: &str, close: &str) -> Option<String> {
    if !close.trim().eq_ignore_ascii_case("</details>") {
        return None;
    }
    let frag = Html::parse_fragment(open);
    let mut kids = frag.root_element().children().filter(|c| match c.value() {
        Node::Text(t) => !t.trim().is_empty(),
        Node::Element(_) => true,
        _ => false,
    });
    let details = ElementRef::wrap(kids.next()?)?;
    if kids.next().is_some() || details.value().name() != "details" {
        return None;
    }
    let mut parts = details.children().filter(|c| match c.value() {
        Node::Text(t) => !t.trim().is_empty(),
        Node::Element(_) => true,
        _ => false,
    });
    let title = match parts.next() {
        None => String::new(),
        Some(n) => {
            let s = ElementRef::wrap(n).filter(|e| e.value().name() == "summary")?;
            collapse(&s.text().collect::<String>()).trim().to_owned()
        }
    };
    if parts.next().is_some() {
        return None;
    }
    Some(if title.is_empty() {
        "Details".to_owned()
    } else {
        title
    })
}

fn merge_split_blocks(md: &str, expand_details: bool) -> String {
    let blocks = top_level_blocks(md);
    let mut out = String::with_capacity(md.len());
    let mut pos = 0usize;
    let mut i = 0usize;

    while i < blocks.len() {
        let b = &blocks[i];
        if b.html {
            let mut depth = tag_depth(&md[b.range.clone()]);
            if depth > 0 {
                let close = (i + 1..blocks.len()).find(|&j| {
                    if blocks[j].html {
                        depth += tag_depth(&md[blocks[j].range.clone()]);
                        depth <= 0
                    } else {
                        false
                    }
                });
                if let Some(j) = close {
                    if expand_details
                        && let Some(title) =
                            details_title(&md[b.range.clone()], &md[blocks[j].range.clone()])
                    {
                        let body = if j > i + 1 {
                            let body = &md[blocks[i + 1].range.start..blocks[j].range.start];
                            merge_split_blocks(body, expand_details) // nested <details>
                        } else {
                            String::new()
                        };
                        out.push_str(&md[pos..b.range.start]);
                        out.push_str(&format!("**\u{25be} {}**\n\n", escape_md(&title)));
                        out.push_str(&body);
                        out.push_str("\n\n");
                        pos = blocks[j].range.end;
                        i = j + 1;
                        continue;
                    }
                    let mut html = String::from(&md[b.range.clone()]);
                    if !html.ends_with('\n') {
                        html.push('\n');
                    }
                    let mut k = i + 1;
                    while k <= j {
                        if blocks[k].html {
                            html.push_str(&md[blocks[k].range.clone()]);
                            if !html.ends_with('\n') {
                                html.push('\n');
                            }
                            k += 1;
                        } else {
                            // A run of consecutive Markdown blocks -> HTML.
                            let start = blocks[k].range.start;
                            let mut m = k;
                            while !blocks[m].html {
                                m += 1;
                            }
                            let end = blocks[m - 1].range.end;
                            push_html(&mut html, Parser::new_ext(&md[start..end], opts()));
                            k = m;
                        }
                    }
                    out.push_str(&md[pos..b.range.start]);
                    out.push_str(&unblank(&html));
                    out.push_str("\n\n");
                    pos = blocks[j].range.end;
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&md[pos..]);
    out
}

// ---------------------------------------------------------------------------
// Block lowering (HTML -> native Markdown)
// ---------------------------------------------------------------------------

fn has_tag(md: &str, name: &str) -> bool {
    md.match_indices('<').any(|(i, _)| {
        md.as_bytes()
            .get(i + 1..i + 1 + name.len())
            .is_some_and(|b| b.eq_ignore_ascii_case(name.as_bytes()))
    })
}

/// Replaces top-level HTML blocks that consist solely of a `<table>` or an
/// `<iframe>` with equivalent Markdown. Other blocks are left alone.
fn lower_blocks(md: &str) -> String {
    if !(has_tag(md, "table") || has_tag(md, "iframe")) {
        return md.to_owned();
    }
    let mut out = String::with_capacity(md.len());
    let mut pos = 0usize;
    for b in top_level_blocks(md).iter().filter(|b| b.html) {
        let Some(lowered) = lower_html(&md[b.range.clone()]) else {
            continue;
        };
        out.push_str(&md[pos..b.range.start]);
        // Markdown tables need a blank line before them.
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
        }
        out.push_str(&lowered);
        out.push_str("\n\n");
        pos = b.range.end;
    }
    out.push_str(&md[pos..]);
    out
}

fn lower_html(html: &str) -> Option<String> {
    let frag = Html::parse_fragment(html);
    let mut significant = frag.root_element().children().filter(|c| match c.value() {
        Node::Text(t) => !t.trim().is_empty(),
        Node::Element(_) => true,
        _ => false,
    });
    let only = significant.next()?;
    if significant.next().is_some() {
        return None;
    }
    let el = ElementRef::wrap(only)?;
    match el.value().name() {
        "table" => table_to_md(el),
        "iframe" => {
            let src = el.value().attr("src")?;
            let label = el
                .value()
                .attr("title")
                .filter(|t| !t.is_empty())
                .unwrap_or(src);
            Some(format!(
                "[{}]({})",
                escape_md(label).replace(']', "\\]"),
                dest(src)
            ))
        }
        _ => None,
    }
}

fn collect_rows<'a>(n: ElementRef<'a>, out: &mut Vec<ElementRef<'a>>) {
    for c in n.children().filter_map(ElementRef::wrap) {
        match c.value().name() {
            "tr" => out.push(c),
            "thead" | "tbody" | "tfoot" => collect_rows(c, out),
            _ => {}
        }
    }
}

fn cells<'a>(row: ElementRef<'a>) -> impl Iterator<Item = ElementRef<'a>> {
    row.children()
        .filter_map(ElementRef::wrap)
        .filter(|e| matches!(e.value().name(), "td" | "th"))
}

/// GFM tables have no spans: a spanning cell's content goes in its first slot
/// and the slots it covers are left empty. Tables without a `<th>` header row
/// get an empty header row, as GFM requires one.
fn table_to_md(table: ElementRef) -> Option<String> {
    if table
        .descendants()
        .skip(1)
        .any(|n| matches!(n.value(), Node::Element(e) if e.name() == "table"))
    {
        return None; // nested tables can't be expressed
    }
    let mut rows = Vec::new();
    collect_rows(table, &mut rows);
    if rows.is_empty() {
        return None;
    }

    let span = |c: ElementRef, k: &str| {
        c.value()
            .attr(k)
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 1000)
    };
    let mut taken: HashSet<(usize, usize)> = HashSet::new();
    let mut placed: Vec<(usize, usize, String)> = Vec::new();
    let mut ncols = 0;
    for (r, row) in rows.iter().enumerate() {
        let mut c = 0;
        for cell in cells(*row) {
            while taken.contains(&(r, c)) {
                c += 1;
            }
            let (rs, cs) = (span(cell, "rowspan"), span(cell, "colspan"));
            for dr in 0..rs {
                for dc in 0..cs {
                    taken.insert((r + dr, c + dc));
                }
            }
            placed.push((r, c, node_md(*cell).trim().replace('|', "\\|")));
            c += cs;
            ncols = ncols.max(c);
        }
    }
    if ncols == 0 {
        return None;
    }
    let mut grid = vec![vec![String::new(); ncols]; rows.len()];
    for (r, c, s) in placed {
        grid[r][c] = s;
    }

    let first: Vec<_> = cells(rows[0]).collect();
    let header_first = !first.is_empty() && first.iter().all(|e| e.value().name() == "th");
    let line = |cells: &[String]| format!("| {} |", cells.join(" | "));
    let mut out = String::new();
    if let Some(cap) = table
        .children()
        .filter_map(ElementRef::wrap)
        .find(|e| e.value().name() == "caption")
    {
        let cap = node_md(*cap);
        if !cap.trim().is_empty() {
            out.push_str(&format!("**{}**\n\n", cap.trim()));
        }
    }
    let body = if header_first {
        out.push_str(&line(&grid[0]));
        &grid[1..]
    } else {
        out.push_str(&line(&vec![String::new(); ncols]));
        &grid[..]
    };
    out.push('\n');
    out.push_str(&line(&vec!["---".to_owned(); ncols]));
    for row in body {
        out.push('\n');
        out.push_str(&line(row));
    }
    Some(out)
}

/// Backslash-escapes characters that would otherwise be read as Markdown.
fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn wrap_md(marker: &str, s: String) -> String {
    let t = s.trim();
    if t.is_empty() {
        return s;
    }
    let lead = &s[..s.len() - s.trim_start().len()];
    let trail = &s[s.trim_end().len()..];
    format!("{lead}{marker}{t}{marker}{trail}")
}

/// Converts a DOM node to inline Markdown (used for table cells, so block
/// elements are flattened to space-separated text).
fn node_md(n: NodeRef<Node>) -> String {
    match n.value() {
        Node::Text(t) => escape_md(&collapse(t)),
        Node::Element(e) => {
            let inner = || n.children().map(node_md).collect::<String>();
            let text = || {
                ElementRef::wrap(n)
                    .map(|e| e.text().collect::<String>())
                    .unwrap_or_default()
            };
            match e.name() {
                "script" | "style" | "head" => String::new(),
                "br" => " ".to_owned(),
                "img" => e.attr("src").map_or_else(String::new, |src| {
                    let alt = e.attr("alt").unwrap_or("").replace(']', "\\]");
                    format!("![{alt}]({})", dest(src))
                }),
                "b" | "strong" | "mark" => wrap_md("**", inner()),
                "i" | "em" | "cite" | "var" | "dfn" | "u" | "ins" => wrap_md("*", inner()),
                "s" | "del" | "strike" => wrap_md("~~", inner()),
                "code" | "kbd" | "samp" | "tt" => format!("`{}`", text()),
                "a" => e
                    .attr("href")
                    .map_or_else(inner, |h| format!("[{}]({})", inner(), dest(h))),
                "sub" | "sup" => script(&text(), e.name() == "sup"),
                "p" | "div" | "ul" | "ol" | "li" | "tr" => format!(" {} ", inner()),
                _ => inner(),
            }
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Inline rewriting
// ---------------------------------------------------------------------------

/// Inline elements that are simply dropped (their content is kept).
const TRANSPARENT: &[&str] = &[
    "span", "font", "small", "big", "abbr", "acronym", "label", "q", "bdi", "bdo", "time", "data",
    "ruby", "rt", "rp", "nobr",
];

#[derive(PartialEq)]
enum Kind {
    Plain,
    /// Emphasis-style opener: whitespace after it is moved in front of it,
    /// because `** text**` would not be recognised as bold.
    Open,
    /// Emphasis-style closer: whitespace before it is moved after it.
    Close,
}

struct Rewrite {
    text: String,
    /// End of the replaced source range.
    end: usize,
    /// Number of events consumed.
    consumed: usize,
    kind: Kind,
}

fn parse_tag(s: &str) -> Option<(String, bool)> {
    let inner = s.trim().strip_prefix('<')?.strip_suffix('>')?;
    let (closing, inner) = inner
        .strip_prefix('/')
        .map_or((false, inner), |rest| (true, rest));
    let name: String = inner
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    (!name.is_empty()).then_some((name, closing))
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let frag = Html::parse_fragment(tag);
    frag.tree.nodes().find_map(|n| match n.value() {
        Node::Element(e) if e.name() != "html" => e.attr(name).map(str::to_owned),
        _ => None,
    })
}

fn dest(url: &str) -> String {
    if url.contains([' ', '(', ')']) {
        format!("<{}>", url.replace(['<', '>'], ""))
    } else {
        url.to_owned()
    }
}

fn sup_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '\u{2212}' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        ' ' => ' ',
        _ => return None,
    })
}

fn sub_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '\u{2212}' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'o' => 'ₒ',
        'x' => 'ₓ',
        'h' => 'ₕ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'p' => 'ₚ',
        's' => 'ₛ',
        't' => 'ₜ',
        ' ' => ' ',
        _ => return None,
    })
}

/// Converts to Unicode super/subscripts, falling back to `^(..)` / `_(..)`.
fn script(text: &str, sup: bool) -> String {
    let map = if sup { sup_char } else { sub_char };
    text.chars()
        .map(map)
        .collect::<Option<String>>()
        .unwrap_or_else(|| {
            if sup {
                format!("^({text})")
            } else {
                format!("_({text})")
            }
        })
}

fn rewrite_tag(
    events: &[(Event, Range<usize>)],
    i: usize,
    md: &str,
    links: &mut Vec<Option<String>>,
    no_break: bool,
) -> Option<Rewrite> {
    let (Event::InlineHtml(raw), range) = &events[i] else {
        return None;
    };
    let raw: &str = raw;
    let plain = |text: String| Rewrite {
        text,
        end: range.end,
        consumed: 1,
        kind: Kind::Plain,
    };
    if raw.starts_with("<!--") {
        return Some(plain(String::new()));
    }
    let (name, closing) = parse_tag(raw)?;
    let marker = |m: &str| Rewrite {
        text: m.to_owned(),
        end: range.end,
        consumed: 1,
        kind: if closing { Kind::Close } else { Kind::Open },
    };

    Some(match (name.as_str(), closing) {
        ("b" | "strong" | "mark", _) => marker("**"),
        ("i" | "em" | "cite" | "var" | "dfn" | "u" | "ins", _) => marker("*"),
        ("s" | "del" | "strike", _) => marker("~~"),
        ("code" | "kbd" | "samp" | "tt", _) => plain("`".to_owned()),
        ("wbr", _) => plain(String::new()),
        ("br", _) => {
            // Skip trailing spaces so `<br>  \n` doesn't leave a literal `\`.
            let after = md[range.end..].trim_start_matches([' ', '\t']);
            let end = md.len() - after.len();
            let text = if no_break {
                " " // hard breaks would split a table row / heading
            } else if let Some(next) = after
                .strip_prefix("\r\n")
                .or_else(|| after.strip_prefix('\n'))
            {
                // The newline itself completes the `\` hard break, unless the
                // paragraph ends here (then a trailing <br> is meaningless).
                if next.lines().next().is_none_or(|l| l.trim().is_empty()) {
                    ""
                } else {
                    "\\"
                }
            } else if after.is_empty() {
                ""
            } else {
                "\\\n"
            };
            Rewrite {
                text: text.to_owned(),
                end,
                consumed: 1,
                kind: Kind::Plain,
            }
        }
        ("a", false) => match attr(raw, "href") {
            Some(href) => {
                links.push(Some(href));
                plain("[".to_owned())
            }
            None => {
                links.push(None);
                plain(String::new())
            }
        },
        ("a", true) => match links.pop() {
            Some(Some(href)) => plain(format!("]({})", dest(&href))),
            _ => plain(String::new()),
        },
        ("img", false) => {
            let src = attr(raw, "src")?;
            let alt = attr(raw, "alt").unwrap_or_default().replace(']', "\\]");
            plain(format!("![{alt}]({})", dest(&src)))
        }
        ("sub" | "sup", false) => {
            let mut text = String::new();
            let mut j = i + 1;
            while let Some((Event::Text(t), _)) = events.get(j) {
                text.push_str(t);
                j += 1;
            }
            match events.get(j) {
                Some((Event::InlineHtml(close), r))
                    if parse_tag(close).is_some_and(|(n, c)| c && n == name) =>
                {
                    Rewrite {
                        text: script(&text, name == "sup"),
                        end: r.end,
                        consumed: j + 1 - i,
                        kind: Kind::Plain,
                    }
                }
                // Nested formatting: just drop the tags.
                _ => plain(String::new()),
            }
        }
        ("sub" | "sup", true) => plain(String::new()),
        (n, _) if TRANSPARENT.contains(&n) => plain(String::new()),
        _ => return None,
    })
}

fn rewrite_inline_html(md: &str) -> String {
    let events: Vec<(Event, Range<usize>)> =
        Parser::new_ext(md, opts()).into_offset_iter().collect();
    let mut out = String::with_capacity(md.len());
    let mut pos = 0usize;
    let mut links: Vec<Option<String>> = Vec::new();
    let mut no_break = 0usize;
    let mut i = 0usize;

    while i < events.len() {
        match &events[i].0 {
            Event::Start(Tag::TableCell | Tag::Heading { .. }) => no_break += 1,
            Event::End(TagEnd::TableCell | TagEnd::Heading(_)) => {
                no_break = no_break.saturating_sub(1);
            }
            Event::InlineHtml(_) => {
                if let Some(rw) = rewrite_tag(&events, i, md, &mut links, no_break > 0) {
                    out.push_str(&md[pos..events[i].1.start]);
                    match rw.kind {
                        Kind::Plain => {
                            out.push_str(&rw.text);
                            pos = rw.end;
                        }
                        Kind::Close => {
                            let keep = out.trim_end_matches([' ', '\t']).len();
                            let ws = out.split_off(keep);
                            out.push_str(&rw.text);
                            out.push_str(&ws);
                            pos = rw.end;
                        }
                        Kind::Open => {
                            let rest = &md[rw.end..];
                            let n = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                            out.push_str(&rest[..n]);
                            out.push_str(&rw.text);
                            pos = rw.end + n;
                        }
                    }
                    i += rw.consumed;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(&md[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_tags() {
        let md = "* **Underline:** <u>This text.</u>\n\
                  * H<sub>2</sub>O and a<sup>2</sup> + b<sup>2</sup>\n\
                  * <mark>hi</mark> <kbd>Ctrl</kbd> <b>bold </b>x <span>s</span>\n";
        let out = preprocess_html(md);
        assert!(out.contains("*This text.*"), "{out}");
        assert!(out.contains("H₂O and a² + b²"), "{out}");
        assert!(out.contains("**hi** `Ctrl` **bold** x s"), "{out}");
        assert!(!out.contains('<'), "{out}");
    }

    #[test]
    fn br_becomes_hard_break() {
        let md = "Registry  \nWay  \n<br>\nCape Town  \n7405\n";
        let out = preprocess_html(md);
        assert_eq!(out, "Registry  \nWay  \n\\\nCape Town  \n7405\n");
        assert_eq!(preprocess_html("a<br>b\n"), "a\\\nb\n");
    }

    #[test]
    fn code_and_generics_untouched() {
        let md = "Use Vec<String> here.\n\n```html\n<b>x</b>\n```\n";
        assert_eq!(preprocess_html(md), md);
    }

    #[test]
    fn links_and_images() {
        let out =
            preprocess_html("<a href=\"https://x.y\"><img src=\"a b.png\" alt=\"hi\"></a> t\n");
        assert_eq!(out, "[![hi](<a b.png>)](https://x.y) t\n");
    }

    #[test]
    fn table_is_lowered_to_markdown() {
        let md = "Intro\n\n<table>\n  <tr>\n    <th>Item</th>\n    <th>Price</th>\n  </tr>\n  \
                  <tr>\n    <td rowspan=\"2\">Bundle <b>A</b></td>\n    <td>$49.00</td>\n  </tr>\n  \
                  <tr>\n    <td>a|b</td>\n  </tr>\n</table>\n\nAfter\n";
        let out = preprocess_html(md);
        assert!(!out.contains("<table"), "{out}");
        assert!(out.contains("| Item | Price |\n| --- | --- |"), "{out}");
        assert!(
            out.contains("| **A** |") || out.contains("| Bundle **A** | $49.00 |"),
            "{out}"
        );
        assert!(out.contains("|  | a\\|b |"), "{out}");
        // pulldown now sees a real table.
        let tables = Parser::new_ext(&out, opts())
            .filter(|e| matches!(e, Event::Start(Tag::Table(_))))
            .count();
        assert_eq!(tables, 1, "{out}");
        assert!(
            out.contains("Intro") && out.trim_end().ends_with("After"),
            "{out}"
        );
    }

    #[test]
    fn headerless_table_and_iframe() {
        let out = preprocess_html("<table><tr><td>a</td><td>b</td></tr></table>\n");
        assert!(
            out.starts_with("|  |  |\n| --- | --- |\n| a | b |"),
            "{out}"
        );
        let out = preprocess_html("<iframe width=\"5\" src=\"https://youtube.com\"></iframe>\n");
        assert_eq!(out.trim(), "[https://youtube.com](https://youtube.com)");
    }

    #[test]
    fn details_expanded_to_markdown() {
        let md = "<details>\n    <summary>Click <b>here</b> now</summary>\n\n```bash\nError\n```\n</details>\n\n* item\n";
        let out = preprocess_html_with(md, true);
        assert!(out.contains("**\u{25be} Click here now**"), "{out}");
        assert!(out.contains("```bash\nError\n```"), "{out}");
        assert!(
            !out.contains("details") && !out.contains("summary"),
            "{out}"
        );
        assert!(out.trim_end().ends_with("* item"), "{out}");
    }

    #[test]
    fn details_is_merged_into_one_block() {
        let md = "before\n\n<details>\n    <summary>Click <b>here</b></summary>\n\n```bash\nError\n\nmore\n```\n</details>\n\nafter\n";
        let out = preprocess_html_with(md, false);
        let start = out.find("<details>").unwrap();
        let end = out.find("</details>").unwrap();
        let block = &out[start..end];
        assert!(block.contains("<summary>"), "{out}");
        assert!(block.contains("<pre><code"), "{out}");
        assert!(
            !block.contains("\n\n"),
            "blank line would split the block: {out:?}"
        );
        assert!(out.ends_with("after\n"), "{out}");
        // The merged block is seen by pulldown as exactly one HTML block.
        let html_blocks = top_level_blocks(&out).iter().filter(|b| b.html).count();
        assert_eq!(html_blocks, 1, "{out}");
    }
}
