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

use crate::html_render::tag_depth;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html::push_html};
use scraper::{Html, Node};
use std::ops::Range;

fn opts() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Entry point. Cheap no-op for documents without any `<`.
#[must_use]
pub fn preprocess_html(md: &str) -> String {
    if !md.contains('<') {
        return md.to_owned();
    }
    rewrite_inline_html(&merge_split_blocks(md))
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

fn merge_split_blocks(md: &str) -> String {
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
    fn details_is_merged_into_one_block() {
        let md = "before\n\n<details>\n    <summary>Click <b>here</b></summary>\n\n```bash\nError\n\nmore\n```\n</details>\n\nafter\n";
        let out = preprocess_html(md);
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
