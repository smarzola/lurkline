use std::iter::Peekable;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    error::{Error, Result},
    model::RenderedMessage,
};

pub(crate) const MAX_MARKDOWN_BYTES: usize = 40_000;
const MAX_RENDERED_BYTES: usize = 100_000;
const MAX_RICH_ELEMENTS: usize = 1_000;
const MAX_LIST_DEPTH: usize = 8;
const MAX_PARSE_DEPTH: usize = 64;
const SLACK_LINK_SYNTAX_ERROR: &str =
    "Slack-native <URL|label> link syntax is unsupported; use standard Markdown: [label](URL)";

type Events<'a> = Peekable<Parser<'a>>;

#[derive(Debug)]
enum Block {
    Section(Vec<Inline>),
    Heading(Vec<Inline>),
    Quote(Vec<Block>),
    Code(String),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Rule,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

#[derive(Debug, Clone, Default)]
struct InlineContext {
    style: InlineStyle,
    link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Inline {
    text: String,
    style: InlineStyle,
    link: Option<String>,
}

pub(crate) fn render_markdown(source: &str) -> Result<RenderedMessage> {
    validate_source(source)?;
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    validate_slack_native_links(source, options)?;
    validate_parse_depth(source, options)?;
    let mut events = Parser::new_ext(source, options).peekable();
    let parsed = parse_blocks(&mut events, None);
    let mut rich_elements = Vec::new();
    emit_blocks(&parsed, 0, &mut rich_elements)?;
    if rich_elements.is_empty() {
        return Err(Error::invalid_input(
            "markdown",
            "must render at least one visible character",
        ));
    }

    let text = blocks_plain(&parsed).trim_end().to_owned();
    if text.trim().is_empty() {
        return Err(Error::invalid_input(
            "markdown",
            "must render at least one visible character",
        ));
    }
    if text.len() > MAX_MARKDOWN_BYTES {
        return Err(Error::invalid_input(
            "markdown",
            "plain-text fallback exceeds 40000 bytes",
        ));
    }

    let blocks = vec![json!({
        "type": "rich_text",
        "elements": rich_elements,
    })];
    let rendered_bytes = serde_json::to_vec(&blocks)
        .map_err(|_| Error::Output)?
        .len();
    if rendered_bytes > MAX_RENDERED_BYTES {
        return Err(Error::invalid_input(
            "markdown",
            "rendered rich text exceeds the supported size",
        ));
    }
    if count_typed_elements(&blocks) > MAX_RICH_ELEMENTS {
        return Err(Error::invalid_input(
            "markdown",
            "renders too many rich-text elements",
        ));
    }

    Ok(RenderedMessage { text, blocks })
}

fn validate_slack_native_links(source: &str, options: Options) -> Result<()> {
    let mut prose_owned = vec![false; source.len()];
    let mut code_block_depth = 0_usize;

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                code_block_depth = code_block_depth.saturating_add(1);
            }
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Start(Tag::Link {
                link_type: LinkType::Autolink,
                ..
            }) if code_block_depth == 0 => prose_owned[range].fill(true),
            Event::Text(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
                if code_block_depth == 0 =>
            {
                prose_owned[range].fill(true);
            }
            _ => {}
        }
    }

    if contains_slack_native_link(source, &prose_owned) {
        return Err(Error::invalid_input("markdown", SLACK_LINK_SYNTAX_ERROR));
    }
    Ok(())
}

fn contains_slack_native_link(source: &str, prose_owned: &[bool]) -> bool {
    let bytes = source.as_bytes();
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        let Some(relative_start) = bytes[cursor..].iter().position(|byte| *byte == b'<') else {
            return false;
        };
        let start = cursor + relative_start;
        cursor = start.saturating_add(1);

        if !prose_owned[start] || opening_delimiter_is_escaped(bytes, start) {
            continue;
        }
        let Some(scheme_length) = slack_http_scheme_length(bytes, start) else {
            continue;
        };
        let destination_start = start + scheme_length;
        let line_end = bytes[destination_start..]
            .iter()
            .position(|byte| matches!(*byte, b'\n' | b'\r'))
            .map_or(bytes.len(), |relative| destination_start + relative);
        let Some(end) = (destination_start..line_end)
            .find(|position| bytes[*position] == b'>' && prose_owned[*position])
        else {
            continue;
        };
        let Some(pipe) = (destination_start..end)
            .find(|position| bytes[*position] == b'|' && prose_owned[*position])
        else {
            continue;
        };

        let candidate_url = &source[start + 1..pipe];
        if is_supported_link(candidate_url) {
            return true;
        }
    }
    false
}

fn opening_delimiter_is_escaped(bytes: &[u8], start: usize) -> bool {
    bytes[..start]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn slack_http_scheme_length(bytes: &[u8], start: usize) -> Option<usize> {
    [b"<https://".as_slice(), b"<http://".as_slice()]
        .into_iter()
        .find(|scheme| {
            bytes
                .get(start..start.saturating_add(scheme.len()))
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(scheme))
        })
        .map(<[u8]>::len)
}

fn validate_parse_depth(source: &str, options: Options) -> Result<()> {
    let mut depth = 0_usize;
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(_) => {
                depth = depth.saturating_add(1);
                if depth > MAX_PARSE_DEPTH {
                    return Err(Error::invalid_input(
                        "markdown",
                        "nesting exceeds 64 levels",
                    ));
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn validate_source(source: &str) -> Result<()> {
    if source.len() > MAX_MARKDOWN_BYTES {
        return Err(Error::invalid_input(
            "markdown",
            "is larger than 40000 bytes",
        ));
    }
    if source.trim().is_empty() {
        return Err(Error::invalid_input(
            "markdown",
            "must contain visible content",
        ));
    }
    if source
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(Error::invalid_input(
            "markdown",
            "contains an unsupported control character",
        ));
    }
    Ok(())
}

fn parse_blocks(events: &mut Events<'_>, expected_end: Option<TagEnd>) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::End(end) => {
                debug_assert!(expected_end.is_some_and(|expected| expected == end));
                break;
            }
            Event::Start(Tag::Paragraph) => blocks.push(Block::Section(parse_inlines(
                events,
                TagEnd::Paragraph,
                InlineContext::default(),
            ))),
            Event::Start(Tag::Heading { level, .. }) => blocks.push(Block::Heading(parse_inlines(
                events,
                TagEnd::Heading(level),
                InlineContext::default(),
            ))),
            Event::Start(Tag::BlockQuote(kind)) => blocks.push(Block::Quote(parse_blocks(
                events,
                Some(TagEnd::BlockQuote(kind)),
            ))),
            Event::Start(Tag::CodeBlock(_)) => {
                blocks.push(Block::Code(collect_literal(events, TagEnd::CodeBlock)));
            }
            Event::Start(Tag::List(start)) => blocks.push(parse_list(events, start)),
            Event::Start(tag) => blocks.push(Block::Section(parse_inlines(
                events,
                tag.to_end(),
                InlineContext::default(),
            ))),
            Event::Rule => blocks.push(Block::Rule),
            event => {
                let mut inlines = Vec::new();
                append_inline_event(&mut inlines, event, &InlineContext::default());
                blocks.push(Block::Section(inlines));
            }
        }
    }
    blocks
}

fn parse_list(events: &mut Events<'_>, start: Option<u64>) -> Block {
    let mut items = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::Item) => {
                items.push(parse_blocks(events, Some(TagEnd::Item)));
            }
            Event::End(TagEnd::List(_)) => break,
            Event::End(end) => {
                debug_assert!(matches!(end, TagEnd::List(_)));
                break;
            }
            _ => {}
        }
    }
    Block::List { start, items }
}

fn parse_inlines(
    events: &mut Events<'_>,
    expected_end: TagEnd,
    context: InlineContext,
) -> Vec<Inline> {
    let mut inlines = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::End(end) => {
                debug_assert_eq!(end, expected_end);
                break;
            }
            Event::Start(Tag::Emphasis) => {
                let mut nested = context.clone();
                nested.style.italic = true;
                extend_inlines(
                    &mut inlines,
                    parse_inlines(events, TagEnd::Emphasis, nested),
                );
            }
            Event::Start(Tag::Strong) => {
                let mut nested = context.clone();
                nested.style.bold = true;
                extend_inlines(&mut inlines, parse_inlines(events, TagEnd::Strong, nested));
            }
            Event::Start(Tag::Strikethrough) => {
                let mut nested = context.clone();
                nested.style.strike = true;
                extend_inlines(
                    &mut inlines,
                    parse_inlines(events, TagEnd::Strikethrough, nested),
                );
            }
            Event::Start(Tag::Link { dest_url, .. }) => parse_link_like(
                events,
                TagEnd::Link,
                dest_url.into_string(),
                &context,
                &mut inlines,
            ),
            Event::Start(Tag::Image { dest_url, .. }) => parse_link_like(
                events,
                TagEnd::Image,
                dest_url.into_string(),
                &context,
                &mut inlines,
            ),
            Event::Start(tag) => {
                let end = tag.to_end();
                extend_inlines(&mut inlines, parse_inlines(events, end, context.clone()));
            }
            event => append_inline_event(&mut inlines, event, &context),
        }
    }
    inlines
}

fn collect_literal(events: &mut Events<'_>, expected_end: TagEnd) -> String {
    let mut literal = String::new();
    for event in events.by_ref() {
        match event {
            Event::End(end) => {
                debug_assert_eq!(end, expected_end);
                break;
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => literal.push_str(&text),
            Event::SoftBreak | Event::HardBreak => literal.push('\n'),
            Event::TaskListMarker(checked) => {
                literal.push_str(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(reference) => {
                literal.push_str("[^");
                literal.push_str(&reference);
                literal.push(']');
            }
            Event::Rule => literal.push('—'),
            Event::Start(_) => {}
        }
    }
    literal
}

fn parse_link_like(
    events: &mut Events<'_>,
    end: TagEnd,
    destination: String,
    context: &InlineContext,
    target: &mut Vec<Inline>,
) {
    if is_supported_link(&destination) {
        let mut nested = context.clone();
        nested.link = Some(destination);
        extend_inlines(target, parse_inlines(events, end, nested));
    } else {
        extend_inlines(target, parse_inlines(events, end, context.clone()));
        push_inline(
            target,
            format!(" ({destination})"),
            context.style.clone(),
            context.link.clone(),
        );
    }
}

fn append_inline_event(target: &mut Vec<Inline>, event: Event<'_>, context: &InlineContext) {
    match event {
        Event::Text(text)
        | Event::Html(text)
        | Event::InlineHtml(text)
        | Event::InlineMath(text)
        | Event::DisplayMath(text) => push_inline(
            target,
            text.into_string(),
            context.style.clone(),
            context.link.clone(),
        ),
        Event::Code(text) => {
            let mut style = context.style.clone();
            style.code = true;
            push_inline(target, text.into_string(), style, context.link.clone());
        }
        Event::SoftBreak | Event::HardBreak => push_inline(
            target,
            "\n".into(),
            context.style.clone(),
            context.link.clone(),
        ),
        Event::FootnoteReference(reference) => push_inline(
            target,
            format!("[^{reference}]"),
            context.style.clone(),
            context.link.clone(),
        ),
        Event::TaskListMarker(checked) => push_inline(
            target,
            if checked { "☑ " } else { "☐ " }.into(),
            context.style.clone(),
            context.link.clone(),
        ),
        Event::Rule => push_inline(
            target,
            "—".into(),
            context.style.clone(),
            context.link.clone(),
        ),
        Event::Start(_) | Event::End(_) => {}
    }
}

fn extend_inlines(target: &mut Vec<Inline>, source: Vec<Inline>) {
    for inline in source {
        push_inline(target, inline.text, inline.style, inline.link);
    }
}

fn push_inline(target: &mut Vec<Inline>, text: String, style: InlineStyle, link: Option<String>) {
    if text.is_empty() {
        return;
    }
    if let Some(previous) = target
        .last_mut()
        .filter(|previous| previous.style == style && previous.link == link)
    {
        previous.text.push_str(&text);
    } else {
        target.push(Inline { text, style, link });
    }
}

fn emit_blocks(blocks: &[Block], indent: usize, target: &mut Vec<Value>) -> Result<()> {
    if indent > MAX_LIST_DEPTH {
        return Err(Error::invalid_input(
            "markdown",
            "list nesting exceeds 8 levels",
        ));
    }
    for block in blocks {
        match block {
            Block::Section(inlines) => push_section(inlines.clone(), target),
            Block::Heading(inlines) => {
                let mut inlines = inlines.clone();
                for inline in &mut inlines {
                    inline.style.bold = true;
                }
                push_section(inlines, target);
            }
            Block::Quote(quoted) => {
                let inlines = quote_inlines(quoted, 0);
                if !inlines.is_empty() {
                    target.push(json!({
                        "type": "rich_text_quote",
                        "elements": inlines.into_iter().map(text_value).collect::<Vec<_>>(),
                    }));
                }
            }
            Block::Code(text) => {
                if !text.is_empty() {
                    target.push(json!({
                        "type": "rich_text_preformatted",
                        "elements": [text_value(Inline {
                            text: text.clone(),
                            style: InlineStyle::default(),
                            link: None,
                        })],
                    }));
                }
            }
            Block::List { start, items } => emit_list(*start, items, indent, target)?,
            Block::Rule => push_section(
                vec![Inline {
                    text: "—".into(),
                    style: InlineStyle::default(),
                    link: None,
                }],
                target,
            ),
        }
    }
    Ok(())
}

fn quote_inlines(blocks: &[Block], depth: usize) -> Vec<Inline> {
    let mut inlines = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            push_inline(&mut inlines, "\n".into(), InlineStyle::default(), None);
        }
        append_quote_block(block, depth, &mut inlines);
    }
    trim_inline_end(&mut inlines);
    inlines
}

fn append_quote_block(block: &Block, depth: usize, target: &mut Vec<Inline>) {
    match block {
        Block::Section(inlines) => extend_inlines(target, inlines.clone()),
        Block::Heading(inlines) => {
            let mut inlines = inlines.clone();
            for inline in &mut inlines {
                inline.style.bold = true;
            }
            extend_inlines(target, inlines);
        }
        Block::Quote(blocks) => {
            push_inline(
                target,
                format!("{}> ", "  ".repeat(depth)),
                InlineStyle::default(),
                None,
            );
            extend_inlines(target, quote_inlines(blocks, depth + 1));
        }
        Block::Code(text) => push_inline(
            target,
            text.clone(),
            InlineStyle {
                code: true,
                ..InlineStyle::default()
            },
            None,
        ),
        Block::List { start, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    push_inline(target, "\n".into(), InlineStyle::default(), None);
                }
                let marker = start
                    .map(|first| format!("{}. ", first + index as u64))
                    .unwrap_or_else(|| "- ".into());
                push_inline(
                    target,
                    format!("{}{marker}", "  ".repeat(depth)),
                    InlineStyle::default(),
                    None,
                );
                for (block_index, item_block) in item.iter().enumerate() {
                    if block_index > 0 {
                        push_inline(target, "\n".into(), InlineStyle::default(), None);
                    }
                    append_quote_block(item_block, depth + 1, target);
                }
            }
        }
        Block::Rule => push_inline(target, "—".into(), InlineStyle::default(), None),
    }
}

fn emit_list(
    start: Option<u64>,
    items: &[Vec<Block>],
    indent: usize,
    target: &mut Vec<Value>,
) -> Result<()> {
    for (index, item) in items.iter().enumerate() {
        let mut direct = Vec::new();
        let mut emitted_item = false;
        for block in item {
            if matches!(block, Block::List { .. }) {
                emit_list_direct_segment(
                    start,
                    index,
                    indent,
                    &mut direct,
                    &mut emitted_item,
                    target,
                );
                if !emitted_item {
                    push_list_item(
                        start,
                        index,
                        indent,
                        vec![Inline {
                            text: " ".into(),
                            style: InlineStyle::default(),
                            link: None,
                        }],
                        target,
                    );
                    emitted_item = true;
                }
                emit_blocks(std::slice::from_ref(block), indent + 1, target)?;
            } else {
                append_block_inlines(block, &mut direct);
                push_inline(&mut direct, "\n".into(), InlineStyle::default(), None);
            }
        }
        emit_list_direct_segment(start, index, indent, &mut direct, &mut emitted_item, target);
    }
    Ok(())
}

fn emit_list_direct_segment(
    start: Option<u64>,
    item_index: usize,
    indent: usize,
    inlines: &mut Vec<Inline>,
    emitted_item: &mut bool,
    target: &mut Vec<Value>,
) {
    trim_inline_end(inlines);
    if inlines.is_empty() {
        return;
    }
    let segment = std::mem::take(inlines);
    if *emitted_item {
        let mut continuation = Vec::new();
        push_inline(
            &mut continuation,
            "  ".repeat(indent + 1),
            InlineStyle::default(),
            None,
        );
        extend_inlines(&mut continuation, segment);
        push_section(continuation, target);
        return;
    }

    push_list_item(start, item_index, indent, segment, target);
    *emitted_item = true;
}

fn push_list_item(
    start: Option<u64>,
    item_index: usize,
    indent: usize,
    inlines: Vec<Inline>,
    target: &mut Vec<Value>,
) {
    let mut list = Map::new();
    list.insert("type".into(), Value::String("rich_text_list".into()));
    list.insert(
        "style".into(),
        Value::String(if start.is_some() { "ordered" } else { "bullet" }.into()),
    );
    list.insert("indent".into(), json!(indent));
    if let Some(first) = start {
        list.insert(
            "offset".into(),
            json!(first.saturating_sub(1) + item_index as u64),
        );
    }
    list.insert(
        "elements".into(),
        Value::Array(vec![section_value(inlines)]),
    );
    target.push(Value::Object(list));
}

fn append_block_inlines(block: &Block, target: &mut Vec<Inline>) {
    match block {
        Block::Section(inlines) => extend_inlines(target, inlines.clone()),
        Block::Heading(inlines) => {
            let mut inlines = inlines.clone();
            for inline in &mut inlines {
                inline.style.bold = true;
            }
            extend_inlines(target, inlines);
        }
        Block::Quote(blocks) => push_inline(
            target,
            blocks_plain(blocks).trim_end().to_owned(),
            InlineStyle::default(),
            None,
        ),
        Block::Code(text) => push_inline(
            target,
            text.clone(),
            InlineStyle {
                code: true,
                ..InlineStyle::default()
            },
            None,
        ),
        Block::Rule => push_inline(target, "—".into(), InlineStyle::default(), None),
        Block::List { .. } => {}
    }
}

fn trim_inline_end(inlines: &mut Vec<Inline>) {
    while let Some(last) = inlines.last_mut() {
        let trimmed = last.text.trim_end_matches('\n').len();
        last.text.truncate(trimmed);
        if last.text.is_empty() {
            inlines.pop();
        } else {
            break;
        }
    }
}

fn push_section(inlines: Vec<Inline>, target: &mut Vec<Value>) {
    if !inlines.is_empty() {
        target.push(section_value(inlines));
    }
}

fn section_value(inlines: Vec<Inline>) -> Value {
    json!({
        "type": "rich_text_section",
        "elements": inlines.into_iter().map(text_value).collect::<Vec<_>>(),
    })
}

fn text_value(inline: Inline) -> Value {
    let mut value = Map::new();
    if let Some(link) = inline.link {
        value.insert("type".into(), Value::String("link".into()));
        value.insert("url".into(), Value::String(link));
        value.insert("text".into(), Value::String(inline.text));
    } else {
        value.insert("type".into(), Value::String("text".into()));
        value.insert("text".into(), Value::String(inline.text));
    }
    let style = style_value(&inline.style);
    if !style.is_empty() {
        value.insert("style".into(), Value::Object(style));
    }
    Value::Object(value)
}

fn style_value(style: &InlineStyle) -> Map<String, Value> {
    let mut value = Map::new();
    for (name, enabled) in [
        ("bold", style.bold),
        ("italic", style.italic),
        ("strike", style.strike),
        ("code", style.code),
    ] {
        if enabled {
            value.insert(name.into(), Value::Bool(true));
        }
    }
    value
}

fn is_supported_link(destination: &str) -> bool {
    if destination.len() > 8_192 || destination.chars().any(char::is_control) {
        return false;
    }
    Url::parse(destination).is_ok_and(|url| match url.scheme() {
        "http" | "https" => url.has_host(),
        "mailto" => !url.path().is_empty(),
        _ => false,
    })
}

fn blocks_plain(blocks: &[Block]) -> String {
    let mut text = String::new();
    for block in blocks {
        match block {
            Block::Section(inlines) | Block::Heading(inlines) => {
                text.push_str(&inlines_plain(inlines));
                text.push_str("\n\n");
            }
            Block::Quote(quoted) => {
                let quoted = blocks_plain(quoted);
                for line in quoted.trim_end().lines() {
                    text.push_str("> ");
                    text.push_str(line);
                    text.push('\n');
                }
                text.push('\n');
            }
            Block::Code(code) => {
                text.push_str(code);
                if !code.ends_with('\n') {
                    text.push('\n');
                }
                text.push('\n');
            }
            Block::List { start, items } => {
                append_list_plain(*start, items, 0, &mut text);
                text.push('\n');
            }
            Block::Rule => text.push_str("—\n\n"),
        }
    }
    text
}

fn append_list_plain(start: Option<u64>, items: &[Vec<Block>], indent: usize, target: &mut String) {
    for (index, item) in items.iter().enumerate() {
        target.push_str(&"  ".repeat(indent));
        if let Some(first) = start {
            target.push_str(&(first + index as u64).to_string());
            target.push_str(". ");
        } else {
            target.push_str("- ");
        }
        let mut wrote_direct = false;
        let mut encountered_nested = false;
        for block in item {
            if let Block::List {
                start: nested_start,
                items: nested_items,
            } = block
            {
                if !target.ends_with('\n') {
                    target.push('\n');
                }
                append_list_plain(*nested_start, nested_items, indent + 1, target);
                encountered_nested = true;
            } else {
                if wrote_direct || encountered_nested {
                    if !target.ends_with('\n') {
                        target.push('\n');
                    }
                    target.push_str(&"  ".repeat(indent + 1));
                }
                target.push_str(block_plain(block).trim());
                target.push('\n');
                wrote_direct = true;
            }
        }
        if !target.ends_with('\n') {
            target.push('\n');
        }
    }
}

fn block_plain(block: &Block) -> String {
    match block {
        Block::Section(inlines) | Block::Heading(inlines) => inlines_plain(inlines),
        Block::Quote(blocks) => blocks_plain(blocks).trim().to_owned(),
        Block::Code(code) => code.clone(),
        Block::List { start, items } => {
            let mut text = String::new();
            append_list_plain(*start, items, 0, &mut text);
            text
        }
        Block::Rule => "—".into(),
    }
}

fn inlines_plain(inlines: &[Inline]) -> String {
    let mut text = String::new();
    let mut index = 0;
    while index < inlines.len() {
        let link = inlines[index].link.as_deref();
        let mut label = String::new();
        while index < inlines.len() && inlines[index].link.as_deref() == link {
            label.push_str(&inlines[index].text);
            index += 1;
        }
        text.push_str(&label);
        if let Some(destination) = link
            && label != destination
        {
            text.push_str(" (");
            text.push_str(destination);
            text.push(')');
        }
    }
    text
}

fn count_typed_elements(values: &[Value]) -> usize {
    values
        .iter()
        .map(|value| match value {
            Value::Array(values) => count_typed_elements(values),
            Value::Object(object) => {
                usize::from(object.contains_key("type"))
                    + object
                        .values()
                        .map(|value| count_typed_elements(std::slice::from_ref(value)))
                        .sum::<usize>()
            }
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_renders_documented_rich_text_losslessly_and_deterministically() {
        let rendered = render_markdown(concat!(
            "# Release\n\n",
            "Hello **bold _and italic_**, ~~old~~, and `code` with ",
            "[docs](https://example.com/docs).\n\n",
            "> quoted\n> twice\n\n",
            "3. first\n",
            "   - nested\n",
            "4. second\n\n",
            "```rust\nlet answer = 42;\n```\n",
        ))
        .unwrap();

        assert!(rendered.text.contains("Release"));
        assert!(rendered.text.contains("docs (https://example.com/docs)"));
        assert!(rendered.text.contains("3. first\n  - nested\n4. second"));
        assert_eq!(rendered.blocks[0]["type"], "rich_text");
        let encoded = serde_json::to_string(&rendered.blocks).unwrap();
        for expected in [
            "\"rich_text_section\"",
            "\"bold\":true",
            "\"italic\":true",
            "\"strike\":true",
            "\"code\":true",
            "\"rich_text_quote\"",
            "\"rich_text_list\"",
            "\"style\":\"ordered\"",
            "\"indent\":1",
            "\"offset\":2",
            "\"rich_text_preformatted\"",
            "\"url\":\"https://example.com/docs\"",
        ] {
            assert!(encoded.contains(expected), "missing {expected}: {encoded}");
        }
        assert_eq!(
            rendered,
            render_markdown(concat!(
                "# Release\n\n",
                "Hello **bold _and italic_**, ~~old~~, and `code` with ",
                "[docs](https://example.com/docs).\n\n",
                "> quoted\n> twice\n\n",
                "3. first\n",
                "   - nested\n",
                "4. second\n\n",
                "```rust\nlet answer = 42;\n```\n",
            ))
            .unwrap()
        );
    }

    #[test]
    fn markdown_keeps_unicode_escapes_and_raw_html_as_literal_text() {
        let rendered =
            render_markdown(r"café 🚀 \*literal\* <b>not interpreted</b> [relative](../x)")
                .unwrap();
        assert_eq!(
            rendered.text,
            "café 🚀 *literal* <b>not interpreted</b> relative (../x)"
        );
        let encoded = serde_json::to_string(&rendered.blocks).unwrap();
        assert!(encoded.contains("<b>not interpreted</b>"));
        assert!(!encoded.contains("\"type\":\"link\""));
    }

    #[test]
    fn markdown_rejects_slack_native_labeled_links_in_prose() {
        for source in [
            "<https://example.com/one|One Label>",
            "<https://example.com/two|Two>",
            "Before (<http://example.com/path|a label>), after.",
            "First <https://example.com/one|one>; second <https://example.com/two|two>.",
            "[<https://example.com/nested|nested>](https://example.com/outer)",
            "<https://example.com|**Bold** Label>",
            "<https://example.com|Label `code`>",
            "<https://example.com|read [docs](<https://other.example>)>",
        ] {
            assert_eq!(
                render_markdown(source).unwrap_err().to_string(),
                format!("invalid markdown: {SLACK_LINK_SYNTAX_ERROR}"),
                "accepted unsupported source: {source}"
            );
        }
    }

    #[test]
    fn markdown_preserves_escaped_and_code_slack_link_examples() {
        for source in [
            r"\<https://example.com/escaped|Label>",
            r"\\\<https://example.com/escaped|Label>",
            r"`<https://example.com/inline|Label>`",
            "```text\n<https://example.com/fenced|Label>\n```",
            "    <https://example.com/indented|Label>\n",
        ] {
            let rendered = render_markdown(source).unwrap();
            assert!(
                rendered.text.contains("<https://example.com"),
                "example was not preserved: {source}"
            );
        }

        for source in [
            r"\\<https://example.com/unescaped|Label>",
            r"\\\\<https://example.com/unescaped|Label>",
        ] {
            assert!(
                matches!(
                    render_markdown(source),
                    Err(Error::InvalidInput {
                        field: "markdown",
                        reason: SLACK_LINK_SYNTAX_ERROR,
                    })
                ),
                "even escape parity hid unsupported source: {source}"
            );
        }
    }

    #[test]
    fn markdown_keeps_standard_links_and_plain_autolinks_unchanged() {
        let standard = render_markdown(concat!(
            "[standard](https://example.com/path|value) and ",
            "[angled](<https://example.com/angled|value>) and ",
            "<https://example.com/plain>",
        ))
        .unwrap();
        let encoded = serde_json::to_string(&standard.blocks).unwrap();
        assert!(encoded.contains("\"url\":\"https://example.com/path|value\""));
        assert!(encoded.contains("\"url\":\"https://example.com/angled|value\""));
        assert!(encoded.contains("\"url\":\"https://example.com/plain\""));

        let encoded_pipe = render_markdown("<https://example.com/path%7Cvalue>").unwrap();
        assert_eq!(encoded_pipe.text, "https://example.com/path%7Cvalue");

        let incomplete =
            render_markdown("<https://example.com|unfinished [docs](<https://other.example>)")
                .unwrap();
        assert!(
            incomplete
                .text
                .starts_with("<https://example.com|unfinished")
        );
    }

    #[test]
    fn markdown_quotes_preserve_styles_links_nested_paragraphs_and_code() {
        let rendered = render_markdown(concat!(
            "> **bold** and [docs](https://example.com)\n",
            ">\n",
            "> second paragraph with `code`\n",
            ">\n",
            "> > nested *quote*\n",
        ))
        .unwrap();
        let quote = &rendered.blocks[0]["elements"][0];
        assert_eq!(quote["type"], "rich_text_quote");
        let elements = quote["elements"].as_array().unwrap();
        assert!(elements.iter().any(|element| {
            element["type"] == "text"
                && element["text"] == "bold"
                && element["style"]["bold"] == true
        }));
        assert!(elements.iter().any(|element| {
            element["type"] == "link"
                && element["text"] == "docs"
                && element["url"] == "https://example.com"
        }));
        assert!(elements.iter().any(|element| {
            element["type"] == "text"
                && element["text"] == "code"
                && element["style"]["code"] == true
        }));
        assert!(elements.iter().any(|element| {
            element["type"] == "text"
                && element["text"] == "quote"
                && element["style"]["italic"] == true
        }));
        assert!(elements.iter().any(|element| {
            element["text"]
                .as_str()
                .is_some_and(|text| text.contains("> "))
        }));
    }

    #[test]
    fn markdown_preserves_content_after_a_nested_list_in_source_order() {
        let rendered = render_markdown(concat!(
            "- parent before\n",
            "\n",
            "  - nested\n",
            "\n",
            "  parent after\n",
        ))
        .unwrap();
        assert_eq!(rendered.text, "- parent before\n  - nested\n  parent after");
        let encoded = serde_json::to_string(&rendered.blocks).unwrap();
        let before = encoded.find("parent before").unwrap();
        let nested = encoded.find("nested").unwrap();
        let after = encoded.find("parent after").unwrap();
        assert!(before < nested && nested < after, "{encoded}");
    }

    #[test]
    fn markdown_preserves_nested_first_and_nested_only_parent_items() {
        let nested_only = render_markdown("-\n  - nested\n").unwrap();
        assert_eq!(nested_only.text, "- \n  - nested");
        assert_eq!(
            nested_only.blocks[0]["elements"][0]["type"],
            "rich_text_list"
        );
        assert_eq!(
            nested_only.blocks[0]["elements"][0]["elements"][0]["elements"][0]["text"],
            " "
        );
        assert_eq!(nested_only.blocks[0]["elements"][1]["indent"], 1);

        let nested_first = render_markdown("-\n  - nested\n\n  parent after\n").unwrap();
        assert_eq!(nested_first.text, "- \n  - nested\n  parent after");
        let encoded = serde_json::to_string(&nested_first.blocks).unwrap();
        let nested = encoded.find("nested").unwrap();
        let after = encoded.find("parent after").unwrap();
        assert!(nested < after, "{encoded}");
        assert_eq!(
            nested_first.blocks[0]["elements"][2]["type"],
            "rich_text_section"
        );
    }

    #[test]
    fn markdown_rejects_empty_control_over_limit_and_excessive_structure() {
        for source in ["", " \n\t", "hello\u{0}"] {
            assert!(matches!(
                render_markdown(source),
                Err(Error::InvalidInput {
                    field: "markdown",
                    ..
                })
            ));
        }
        assert!(render_markdown(&"x".repeat(MAX_MARKDOWN_BYTES + 1)).is_err());
        let deeply_nested_quote = format!("{}visible\n", "> ".repeat(MAX_PARSE_DEPTH + 1));
        assert!(matches!(
            render_markdown(&deeply_nested_quote),
            Err(Error::InvalidInput {
                field: "markdown",
                ..
            })
        ));

        let fragmented = (0..=MAX_RICH_ELEMENTS)
            .map(|index| format!("**{index}**"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(matches!(
            render_markdown(&fragmented),
            Err(Error::InvalidInput {
                field: "markdown",
                ..
            })
        ));
    }
}
