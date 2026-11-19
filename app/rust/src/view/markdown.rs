//! Turns Markdown into a flat list of styled blocks the UI can lay out
//! without parsing anything itself. Raw HTML is shown as plain text and
//! images as their description: nothing is ever fetched.
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Heading,
    Paragraph,
    Bullet,
    Numbered,
    Task,
    Quote,
    Code,
    Rule,
    TableRow,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    /// Link target, shown to the person but never opened automatically.
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Heading level (1-6), or how deep a list item or quote is nested.
    pub level: u32,
    /// The number of a numbered item.
    pub number: u64,
    /// Whether a task item is ticked, or a table row is the header.
    pub flag: bool,
    pub spans: Vec<Span>,
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
    link: Vec<String>,
}

struct List {
    next: Option<u64>,
}

pub fn parse(text: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut out: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;
    let mut style = Style::default();
    let mut lists: Vec<List> = Vec::new();
    let mut quote = 0u32;
    let mut in_code = false;

    fn push_text(cur: &mut Option<Block>, style: &Style, text: &str, code: bool, quote: u32) {
        let block = cur.get_or_insert_with(|| Block {
            kind: if quote > 0 {
                BlockKind::Quote
            } else {
                BlockKind::Paragraph
            },
            level: quote,
            number: 0,
            flag: false,
            spans: Vec::new(),
        });
        let span = Span {
            text: text.to_string(),
            bold: style.bold > 0,
            italic: style.italic > 0,
            code,
            strike: style.strike > 0,
            link: style.link.last().cloned(),
        };
        if let Some(last) = block.spans.last_mut() {
            let same = (last.bold, last.italic, last.code, last.strike, &last.link)
                == (span.bold, span.italic, span.code, span.strike, &span.link);
            if same {
                last.text.push_str(text);
                return;
            }
        }
        block.spans.push(span);
    }

    fn finish(out: &mut Vec<Block>, cur: &mut Option<Block>) {
        if let Some(mut b) = cur.take() {
            if b.kind != BlockKind::Code {
                if let Some(last) = b.spans.last_mut() {
                    let trimmed = last.text.trim_end().len();
                    last.text.truncate(trimmed);
                }
                b.spans.retain(|s| !s.text.is_empty());
            }
            if !b.spans.is_empty() || b.kind == BlockKind::Rule {
                out.push(b);
            }
        }
    }

    fn start(cur: &mut Option<Block>, kind: BlockKind, level: u32, number: u64) {
        *cur = Some(Block {
            kind,
            level,
            number,
            flag: false,
            spans: Vec::new(),
        });
    }

    for event in Parser::new_ext(text, opts) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    // A paragraph inside a list item continues that item.
                    if cur.as_ref().is_none_or(|b| {
                        !matches!(
                            b.kind,
                            BlockKind::Bullet | BlockKind::Numbered | BlockKind::Task
                        ) || !b.spans.is_empty()
                    }) {
                        finish(&mut out, &mut cur);
                        let kind = if quote > 0 {
                            BlockKind::Quote
                        } else {
                            BlockKind::Paragraph
                        };
                        start(&mut cur, kind, quote, 0);
                    }
                }
                Tag::Heading { level, .. } => {
                    finish(&mut out, &mut cur);
                    let n = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    start(&mut cur, BlockKind::Heading, n, 0);
                }
                Tag::BlockQuote(_) => {
                    finish(&mut out, &mut cur);
                    quote += 1;
                }
                Tag::CodeBlock(_) => {
                    finish(&mut out, &mut cur);
                    start(&mut cur, BlockKind::Code, quote, 0);
                    in_code = true;
                }
                Tag::List(first) => {
                    finish(&mut out, &mut cur);
                    lists.push(List { next: first });
                }
                Tag::Item => {
                    finish(&mut out, &mut cur);
                    let depth = lists.len() as u32;
                    match lists.last_mut().and_then(|l| l.next.as_mut()) {
                        Some(n) => {
                            start(&mut cur, BlockKind::Numbered, depth, *n);
                            *n += 1;
                        }
                        None => start(&mut cur, BlockKind::Bullet, depth, 0),
                    }
                }
                Tag::TableHead => {
                    finish(&mut out, &mut cur);
                    start(&mut cur, BlockKind::TableRow, 0, 0);
                    if let Some(b) = cur.as_mut() {
                        b.flag = true;
                    }
                }
                Tag::TableRow => {
                    finish(&mut out, &mut cur);
                    start(&mut cur, BlockKind::TableRow, 0, 0);
                }
                Tag::TableCell => {
                    if let Some(b) = cur.as_mut() {
                        if !b.spans.is_empty() {
                            b.spans.push(Span {
                                text: "  │  ".into(),
                                ..Span::default()
                            });
                        }
                    }
                }
                Tag::Emphasis => style.italic += 1,
                Tag::Strong => style.bold += 1,
                Tag::Strikethrough => style.strike += 1,
                Tag::Link { dest_url, .. } => style.link.push(dest_url.to_string()),
                Tag::Image { .. } => {
                    style.italic += 1;
                    push_text(&mut cur, &style, "[picture: ", false, quote);
                }
                Tag::FootnoteDefinition(label) => {
                    finish(&mut out, &mut cur);
                    start(&mut cur, BlockKind::Paragraph, 0, 0);
                    push_text(&mut cur, &style, &format!("[{label}] "), false, quote);
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) => {
                    if !matches!(
                        cur.as_ref().map(|b| b.kind),
                        Some(BlockKind::Bullet | BlockKind::Numbered | BlockKind::Task)
                    ) {
                        finish(&mut out, &mut cur);
                    } else {
                        push_text(&mut cur, &style, "\n", false, quote);
                    }
                }
                TagEnd::BlockQuote(_) => {
                    finish(&mut out, &mut cur);
                    quote = quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some(b) = cur.as_mut() {
                        if let Some(last) = b.spans.last_mut() {
                            if last.text.ends_with('\n') {
                                last.text.pop();
                            }
                        }
                    }
                    finish(&mut out, &mut cur);
                    in_code = false;
                }
                TagEnd::List(_) => {
                    finish(&mut out, &mut cur);
                    lists.pop();
                }
                TagEnd::Item
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::FootnoteDefinition => finish(&mut out, &mut cur),
                TagEnd::Emphasis => style.italic = style.italic.saturating_sub(1),
                TagEnd::Strong => style.bold = style.bold.saturating_sub(1),
                TagEnd::Strikethrough => style.strike = style.strike.saturating_sub(1),
                TagEnd::Link => {
                    style.link.pop();
                }
                TagEnd::Image => {
                    push_text(&mut cur, &style, "]", false, quote);
                    style.italic = style.italic.saturating_sub(1);
                }
                _ => {}
            },
            Event::Text(t) => push_text(&mut cur, &style, &t, in_code, quote),
            Event::Code(t) => push_text(&mut cur, &style, &t, true, quote),
            Event::Html(t) | Event::InlineHtml(t) => {
                if in_code || cur.is_some() {
                    push_text(&mut cur, &style, &t, in_code, quote)
                } else {
                    start(&mut cur, BlockKind::Code, quote, 0);
                    push_text(&mut cur, &style, t.trim_end(), true, quote);
                    finish(&mut out, &mut cur);
                }
            }
            Event::SoftBreak => push_text(&mut cur, &style, " ", in_code, quote),
            Event::HardBreak => push_text(&mut cur, &style, "\n", in_code, quote),
            Event::Rule => {
                finish(&mut out, &mut cur);
                out.push(Block {
                    kind: BlockKind::Rule,
                    level: 0,
                    number: 0,
                    flag: false,
                    spans: Vec::new(),
                });
            }
            Event::TaskListMarker(done) => {
                if let Some(b) = cur.as_mut() {
                    b.kind = BlockKind::Task;
                    b.flag = done;
                }
            }
            Event::FootnoteReference(label) => {
                push_text(&mut cur, &style, &format!("[{label}]"), false, quote)
            }
            _ => {}
        }
    }
    finish(&mut out, &mut cur);
    out
}
