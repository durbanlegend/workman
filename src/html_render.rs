#![allow(clippy::doc_link_with_quotes)]
//! Minimal HTML -> egui renderer for the "README-style" subset of HTML that
//! shows up inside Markdown, designed to plug into `egui_commonmark`'s
//! `render_html_fn`.
//!
//! Handles: <img>, <a>, <br>, <hr>, <p>/<div>/<center> (+ align="center" and
//! style="text-align:center"), <h1>-<h6>, <b>/<strong>, <i>/<em>, <u>, <s>/<del>,
//! <code>/<kbd>, <pre>, <ul>/<ol>/<li>, <details>/<summary>.
//! Anything unknown is treated as a transparent container (its text still shows).
//!
//! Cargo.toml (versions: match to whatever you're on; UNTESTED, written without a compiler):
//!   egui       = "0.36"
//!   scraper    = "0.2x"
//!   ego-tree   = "0.x"   # must be the same version scraper depends on
//!   egui_extras = { version = "*", features = ["all_loaders"] } # + call install_image_loaders(ctx)
//!
//! Usage:
//!   let html = HtmlRenderer::new("file:///path/to/markdown/dir/");   // keep in your App struct
//!   let html = self.html.clone();   // RenderHtmlFn is 'static: the closure must own its renderer
//!   CommonMarkViewer::new()
//!       .render_html_fn(Some(&move |ui, chunk| html.render(ui, chunk)))
//!       .show(ui, &mut self.cache, &self.markdown);

use eframe::egui;
use egui::{CursorIcon, Id, Label, OpenUrl, RichText, Sense, TextStyle, Ui};
use scraper::node::Element;
use scraper::{ElementRef, Html, Node};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

type N<'a> = ego_tree::NodeRef<'a, Node>;

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Cheap to clone: clones share the same buffer. This matters because
/// `egui_commonmark`'s `RenderHtmlFn` is `dyn Fn(..) + 'static`, so the closure
/// passed to `render_html_fn` must own (a clone of) the renderer rather than
/// borrow it from the app.
#[allow(clippy::too_long_first_doc_paragraph)]
#[derive(Clone)]
pub struct HtmlRenderer {
    /// Prefix for relative image paths, e.g. "<file:///home/me/docs>".
    base_uri: Rc<str>,
    /// (pass number the buffer belongs to, buffered html)
    buffer: Rc<RefCell<(u64, String)>>,
}

impl HtmlRenderer {
    pub fn new(base_uri: impl Into<String>) -> Self {
        Self {
            base_uri: Rc::from(base_uri.into()),
            buffer: Rc::new(RefCell::new((0, String::new()))),
        }
    }

    /// Sets the base URI for relative image paths.
    pub fn set_base_uri(&mut self, base_uri: impl Into<String>) {
        self.base_uri = Rc::from(base_uri.into());
    }

    /// Entry point for `render_html_fn`. pulldown-cmark can hand HTML over in
    /// pieces (line by line / tag by tag), so chunks are buffered until the
    /// tags balance, then the whole fragment is rendered at once.
    pub fn render(&self, ui: &mut Ui, chunk: &str) {
        // eprintln!("html chunk: {chunk:?}");
        let pass = ui.ctx().cumulative_pass_nr();
        let html = {
            let mut buf = self.buffer.borrow_mut();
            if buf.0 != pass {
                // Leftover from an earlier pass (unbalanced html): discard it.
                *buf = (pass, String::new());
            }
            buf.1.push_str(chunk);
            if tag_depth(&buf.1) > 0 {
                return; // wait for the closing tag(s)
            }
            std::mem::take(&mut buf.1)
        };
        // eprintln!("html fragment: {html:?}");
        let doc = Html::parse_fragment(&html);
        // The parent is egui_commonmark's wrapped left-to-right row; give our
        // output its own top-down layout so block-level widgets (tables,
        // collapsing headers, spacing) behave.
        ui.vertical(|ui| {
            render_children(ui, doc.tree.root(), &Style::default(), self, false);
        });
        // egui_commonmark lays the document out in a wrapped left-to-right row
        // and separates blocks with `ui.label("\n")` (its `newline()`): one at
        // the end of a block, one at the start of the next. It emits the start
        // one for html blocks but nothing at the end, so without this the next
        // heading/paragraph butts up against the html. `add_space` would be
        // horizontal space in that layout, hence the label.
        if ui.layout().is_horizontal() {
            ui.label("\n");
        } else {
            ui.add_space(ui.spacing().item_spacing.y);
        }
    }

    fn resolve(&self, src: &str) -> String {
        if src.contains("://") || src.starts_with("data:") {
            src.to_owned()
        } else {
            // `base_uri` may be the URI of the markdown *file*; relative paths
            // resolve against its directory (everything up to the last '/').
            let dir = &self.base_uri[..self.base_uri.rfind('/').map_or(0, |i| i + 1)];
            format!("{dir}{}", src.trim_start_matches("./"))
        }
    }
}

// ---------------------------------------------------------------------------
// Block layer
// ---------------------------------------------------------------------------

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    code: bool,
    size: Option<f32>,
    link: Option<String>,
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "html"
            | "body"
            | "p"
            | "div"
            | "center"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "pre"
            | "hr"
            | "details"
            | "summary"
            | "table"
            | "blockquote"
            | "iframe"
            | "figure"
            | "figcaption"
            | "nav"
            | "main"
            | "aside"
    )
}

fn is_el(n: N, name: &str) -> bool {
    matches!(n.value(), Node::Element(e) if e.name() == name)
}

fn is_blank(n: N) -> bool {
    match n.value() {
        Node::Text(t) => t.trim().is_empty(),
        Node::Comment(_) => true,
        _ => false,
    }
}

fn is_centered(el: &Element) -> bool {
    el.name() == "center"
        || el
            .attr("align")
            .is_some_and(|a| a.eq_ignore_ascii_case("center"))
        || el.attr("style").is_some_and(|s| {
            s.replace(' ', "")
                .to_ascii_lowercase()
                .contains("text-align:center")
        })
}

fn node_key(n: N) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match n.value() {
        Node::Element(_) => ElementRef::wrap(n)
            .map(|e| e.html())
            .unwrap_or_default()
            .hash(&mut h),
        Node::Text(t) => (**t).hash(&mut h),
        _ => {}
    }
    h.finish()
}

fn render_children(ui: &mut Ui, parent: N, st: &Style, cx: &HtmlRenderer, centered: bool) {
    let mut run: Vec<N> = Vec::new();
    for child in parent.children() {
        let block = matches!(child.value(), Node::Element(e) if is_block(e.name()));
        if block {
            flush_run(ui, &mut run, st, cx, centered);
            render_block(ui, child, st, cx, centered);
        } else {
            run.push(child);
        }
    }
    flush_run(ui, &mut run, st, cx, centered);
}

fn render_block(ui: &mut Ui, node: N, st: &Style, cx: &HtmlRenderer, centered: bool) {
    let Node::Element(el) = node.value() else {
        return;
    };
    let centered = centered || is_centered(el);
    let name = el.name();

    match name {
        "hr" => {
            ui.separator();
        }
        // Only meaningful inside <details>, which handles it itself.
        "summary" => {}
        "table" => render_table(ui, node, st, cx),
        // Can't embed a web page; show a link to it instead.
        "iframe" => {
            if let Some(src) = el.attr("src") {
                let label = el.attr("title").filter(|t| !t.is_empty()).unwrap_or(src);
                ui.hyperlink_to(label, src);
            }
        }
        "blockquote" => {
            ui.horizontal_top(|ui| {
                ui.add_space(12.0);
                ui.vertical(|ui| render_children(ui, node, st, cx, centered));
            });
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = (name.as_bytes()[1] - b'0') as usize;
            let scale = [2.0, 1.6, 1.35, 1.2, 1.1, 1.0][level - 1];
            let mut st = st.clone();
            st.bold = true;
            st.size = Some(TextStyle::Body.resolve(ui.style()).size * scale);
            render_children(ui, node, &st, cx, centered);
            ui.add_space(4.0);
        }
        "ul" | "ol" => {
            for (n, li) in (el
                .attr("start")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)..)
                .zip(node.children().filter(|c| is_el(*c, "li")))
            {
                let marker = if name == "ol" {
                    format!("{n}.")
                } else {
                    "\u{2022}".to_owned()
                };
                // n += 1;
                ui.horizontal_top(|ui| {
                    ui.add_space(12.0);
                    ui.label(marker);
                    ui.vertical(|ui| render_children(ui, li, st, cx, false));
                });
            }
        }
        "pre" => {
            let text: String = ElementRef::wrap(node)
                .map(|e| e.text().collect())
                .unwrap_or_default();
            egui::Frame::new()
                .fill(ui.visuals().code_bg_color)
                .corner_radius(3.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.add(
                            Label::new(RichText::new(text.trim_end_matches('\n')).monospace())
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                });
        }
        "details" => {
            let title = node
                .children()
                .find(|c| is_el(*c, "summary"))
                .and_then(ElementRef::wrap)
                .map(|s| collapse(&s.text().collect::<String>()).trim().to_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "Details".to_owned());
            // Stable id so open/closed state survives across frames.
            let id = Id::new(("html_details", node_key(node)));
            egui::CollapsingHeader::new(title)
                .id_salt(id)
                .default_open(el.attr("open").is_some())
                .show(ui, |ui| render_children(ui, node, st, cx, centered));
        }
        // p, div, center, section, html, body, li-outside-list, ...
        _ => {
            render_children(ui, node, st, cx, centered);
            if name == "p" {
                ui.add_space(ui.spacing().item_spacing.y);
            }
        }
    }
}

fn collect_rows<'a>(n: N<'a>, out: &mut Vec<N<'a>>) {
    for c in n.children() {
        match c.value() {
            Node::Element(e) if e.name() == "tr" => out.push(c),
            Node::Element(e) if matches!(e.name(), "thead" | "tbody" | "tfoot") => {
                collect_rows(c, out);
            }
            _ => {}
        }
    }
}

/// Renders a `<table>` as an `egui::Grid`. `rowspan`/`colspan` are honoured for
/// placement (the spanned-over slots are left empty) but cells don't visually
/// stretch across the slots they span.
fn render_table(ui: &mut Ui, node: N, st: &Style, cx: &HtmlRenderer) {
    struct Cell<'a> {
        node: N<'a>,
        header: bool,
    }

    let mut rows: Vec<N> = Vec::new();
    collect_rows(node, &mut rows);
    if rows.is_empty() {
        return;
    }

    let span = |n: N, k: &str| -> usize {
        match n.value() {
            Node::Element(e) => e
                .attr(k)
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 1000),
            _ => 1,
        }
    };

    let mut taken: HashSet<(usize, usize)> = HashSet::new();
    let mut cells: HashMap<(usize, usize), Cell> = HashMap::new();
    let mut ncols = 0;
    for (r, row) in rows.iter().enumerate() {
        let mut c = 0;
        for cell in row
            .children()
            .filter(|n| is_el(*n, "td") || is_el(*n, "th"))
        {
            while taken.contains(&(r, c)) {
                c += 1;
            }
            let (rs, cs) = (span(cell, "rowspan"), span(cell, "colspan"));
            for dr in 0..rs {
                for dc in 0..cs {
                    taken.insert((r + dr, c + dc));
                }
            }
            cells.insert(
                (r, c),
                Cell {
                    node: cell,
                    header: is_el(cell, "th"),
                },
            );
            c += cs;
            ncols = ncols.max(c);
        }
    }

    let id = Id::new(("html_table", node_key(node)));
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(3.0)
        .inner_margin(4.0)
        .show(ui, |ui| {
            egui::Grid::new(id)
                .striped(true)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for r in 0..rows.len() {
                        for c in 0..ncols {
                            if let Some(cell) = cells.get(&(r, c)) {
                                let mut st = st.clone();
                                st.bold |= cell.header;
                                ui.vertical(|ui| render_children(ui, cell.node, &st, cx, false));
                            } else {
                                ui.label(""); // slot covered by a row/col span
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}

// ---------------------------------------------------------------------------
// Inline layer
// ---------------------------------------------------------------------------

/// Renders a run of consecutive inline nodes. <br> at the top level of a run
/// splits it into separate rows.
fn flush_run(ui: &mut Ui, run: &mut Vec<N>, st: &Style, cx: &HtmlRenderer, centered: bool) {
    let nodes = std::mem::take(run);
    if nodes.iter().all(|n| is_blank(*n)) {
        return;
    }
    let key = nodes
        .iter()
        .fold(0u64, |a, n| a.rotate_left(5) ^ node_key(*n));

    let mut lines: Vec<Vec<N>> = vec![Vec::new()];
    for n in nodes {
        if is_el(n, "br") {
            lines.push(Vec::new());
        } else {
            lines.last_mut().unwrap().push(n);
        }
    }

    let last = lines.len() - 1;
    let blank_h = TextStyle::Body.resolve(ui.style()).size;
    for (i, line) in lines.iter().enumerate() {
        if line.iter().all(|n| is_blank(*n)) {
            if i > 0 && i < last {
                ui.add_space(blank_h); // consecutive <br><br>
            }
            continue;
        }
        let draw = |ui: &mut Ui| {
            ui.spacing_mut().item_spacing.x = 0.0; // spacing comes from the text itself
            let mut at_start = true;
            for n in line {
                render_inline(ui, *n, st, cx, &mut at_start);
            }
        };
        if centered {
            let id = ui.id().with((key, i));
            centered_row(ui, id, draw);
        } else {
            ui.horizontal_wrapped(draw);
        }
    }
}

#[allow(clippy::assigning_clones)]
fn render_inline(ui: &mut Ui, node: N, style: &Style, cx: &HtmlRenderer, at_start: &mut bool) {
    match node.value() {
        Node::Text(t) => {
            let mut s = collapse(t);
            if *at_start {
                s = s.trim_start().to_owned();
            }
            if s.is_empty() {
                return;
            }
            *at_start = false;
            add_text(ui, s, style);
        }
        Node::Element(el) => {
            let mut st = style.clone();
            match el.name() {
                "img" => {
                    add_image(ui, el, style, cx);
                    *at_start = false;
                    return;
                }
                "script" | "style" | "head" => return,
                "a" => st.link = el.attr("href").map(str::to_owned),
                "b" | "strong" => st.bold = true,
                "i" | "em" | "cite" => st.italic = true,
                "u" | "ins" => st.underline = true,
                "s" | "del" | "strike" => st.strike = true,
                "code" | "kbd" | "samp" | "tt" => st.code = true,
                _ => {}
            }
            for c in node.children() {
                render_inline(ui, c, &st, cx, at_start);
            }
        }
        _ => {}
    }
}

fn add_text(ui: &mut Ui, text: String, st: &Style) {
    let mut rt = RichText::new(text);
    if st.bold {
        rt = rt.strong();
    }
    if st.italic {
        rt = rt.italics();
    }
    if st.underline {
        rt = rt.underline();
    }
    if st.strike {
        rt = rt.strikethrough();
    }
    if st.code {
        rt = rt.monospace().background_color(ui.visuals().code_bg_color);
    }
    if let Some(size) = st.size {
        rt = rt.size(size);
    }
    match &st.link {
        Some(url) => ui.add(egui::Hyperlink::from_label_and_url(rt, url)),
        None => ui.add(Label::new(rt)),
    };
}

fn add_image(ui: &mut Ui, el: &Element, st: &Style, cx: &HtmlRenderer) {
    let Some(src) = el.attr("src") else { return };
    let cap = ui.available_width();

    let mut img = egui::Image::new(cx.resolve(src));
    // egui's default image fit is a fraction of the *available* size, and inside
    // `horizontal_wrapped` the available height is a single text row, so always
    // pick an explicit fit. `fit_to_exact_size` keeps the aspect ratio, so an
    // infinite extent on one axis means "constrain only the other axis".
    img = match (html_px(el, "width"), html_px(el, "height")) {
        (Some(w), Some(h)) => img.fit_to_exact_size(egui::vec2(w.min(cap), h)),
        (Some(w), None) => img.fit_to_exact_size(egui::vec2(w.min(cap), f32::INFINITY)),
        (None, Some(h)) => img.fit_to_exact_size(egui::vec2(cap, h)),
        (None, None) => img.fit_to_original_size(1.0).max_width(cap),
    };
    if let Some(alt) = el.attr("alt") {
        img = img.alt_text(alt);
    }

    match &st.link {
        // Linked images (badges!) are clickable.
        Some(url) => {
            let r = ui
                .add(img.sense(Sense::click()))
                .on_hover_cursor(CursorIcon::PointingHand);
            if r.clicked() {
                ui.ctx().open_url(OpenUrl::new_tab(url));
            }
        }
        None => {
            ui.add(img);
        }
    }
}

/// Parses a pixel-valued HTML attribute such as `width="280"` or `width="280px"`.
/// Tolerates the unquoted `width=280/>` form, where the HTML tokenizer includes
/// the `/` in the value (giving `"280/"`).
fn html_px(el: &Element, name: &str) -> Option<f32> {
    el.attr(name).and_then(|v| {
        v.trim()
            .trim_end_matches('/')
            .trim_end()
            .trim_end_matches("px")
            .trim()
            .parse::<f32>()
            .ok()
    })
}

/// egui can't center a horizontal row of arbitrary widgets directly, so we
/// remember the row's width from the previous frame and indent by half the
/// remaining space. One-frame lag on first show (a repaint is requested).
/// Note: centered rows don't wrap, so this is meant for logos, badges and
/// titles rather than long centered paragraphs.
fn centered_row(ui: &mut Ui, id: Id, add_contents: impl FnOnce(&mut Ui)) {
    let prev: Option<f32> = ui.ctx().data(|d| d.get_temp(id));
    let avail = ui.available_width();
    let indent = prev.map_or(0.0, |w| ((avail - w) / 2.0).max(0.0));

    let r = ui.horizontal(|ui| {
        ui.add_space(indent);
        add_contents(ui);
    });

    let width = r.response.rect.width() - indent;
    if prev.is_none_or(|p| (p - width).abs() > 0.5) {
        ui.ctx().data_mut(|d| d.insert_temp(id, width));
        ui.ctx().request_repaint();
    }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// HTML whitespace collapsing (keeps a single leading/trailing space if present).
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// Net number of currently-open (non-void) tags in `s`. > 0 means "keep buffering".
pub fn tag_depth(s: &str) -> i32 {
    let mut depth = 0;
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        if rest.starts_with("!--") {
            rest = rest.find("-->").map_or("", |j| &rest[j + 3..]);
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        rest = &rest[end + 1..];

        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() || tag.ends_with('/') || VOID.contains(&name.as_str()) {
            continue;
        }
        depth += if closing { -1 } else { 1 };
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img_attr(html: &str, attr: &str) -> Option<f32> {
        let doc = Html::parse_fragment(html);
        let img = doc
            .tree
            .nodes()
            .find_map(|n| match n.value() {
                Node::Element(e) if e.name() == "img" => Some(e),
                _ => None,
            })
            .unwrap();
        html_px(img, attr)
    }

    #[test]
    fn unquoted_width_before_self_closing_slash() {
        let h = r#"<img src="x.png" alt="showcase" width=280/>"#;
        assert_eq!(img_attr(h, "width"), Some(280.0));
    }

    #[test]
    fn quoted_and_px_widths() {
        assert_eq!(
            img_attr(r#"<img src="x" width="280"/>"#, "width"),
            Some(280.0)
        );
        assert_eq!(
            img_attr(r#"<img src="x" width="280px">"#, "width"),
            Some(280.0)
        );
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    /// Runs `html` through the renderer in a headless egui context and returns
    /// the text of every shape painted.
    fn painted_text(html: &str) -> String {
        let ctx = egui::Context::default();
        let r = HtmlRenderer::new("file:///tmp/doc/README.md");
        let mut text = String::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Mimic egui_commonmark's wrapped left-to-right layout.
            let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);
            ui.allocate_ui_with_layout(egui::vec2(600.0, 0.0), layout, |ui| r.render(ui, html));
        });
        for clipped in &out.shapes {
            collect(&clipped.shape, &mut text);
        }
        out.textures_delta.clear();
        text
    }

    fn collect(shape: &egui::epaint::Shape, out: &mut String) {
        match shape {
            egui::epaint::Shape::Text(t) => {
                out.push_str(t.galley.text());
                out.push('|');
            }
            egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }

    #[test]
    fn table_cells_are_rendered() {
        let t = painted_text(
            "<table><tr><th>Item</th><th>Price</th></tr>\
             <tr><td rowspan=\"2\">Bundle A</td><td>$49.00</td></tr>\
             <tr><td>Free</td></tr></table>",
        );
        for want in ["Item", "Price", "Bundle A", "$49.00", "Free"] {
            assert!(t.contains(want), "missing {want:?} in {t:?}");
        }
    }

    #[test]
    fn details_summary_and_iframe() {
        let t = painted_text(
            "<details open><summary>Click <b>here</b></summary><p>Inside</p>\
             <pre>code</pre></details>",
        );
        assert!(t.contains("Click"), "{t:?}");
        assert!(t.contains("Inside") && t.contains("code"), "{t:?}");
        let t = painted_text("<iframe src=\"https://youtube.com\" width=\"560\"></iframe>");
        assert!(t.contains("https://youtube.com"), "{t:?}");
    }

    #[test]
    fn resolve_uses_directory_of_file_uri() {
        let r = HtmlRenderer::new("file:///tmp/doc/README.md");
        assert_eq!(r.resolve("./a/b.png"), "file:///tmp/doc/a/b.png");
        assert_eq!(r.resolve("https://x/y.png"), "https://x/y.png");
    }
}
