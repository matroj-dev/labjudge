use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag};

use crate::url::segment;

pub struct StatementCtx<'a> {
    pub contest_id: &'a str,
    pub problem_key: &'a str,
}

/// 题面通常以 `# 题目名` 开头，而页面顶部已经渲染了题目标题，
/// 这里去掉开头的一级标题行，避免同一屏出现两遍标题。
pub fn strip_leading_title(markdown: &str) -> &str {
    let text = markdown.trim_start();
    let Some(rest) = text.strip_prefix("# ") else {
        return markdown;
    };
    match rest.find('\n') {
        Some(index) => rest[index + 1..].trim_start_matches(['\n', '\r']),
        None => "",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Formula {
    tex: String,
    display: bool,
}

pub fn render_statement(markdown: &str, ctx: &StatementCtx<'_>) -> String {
    let (extracted, formulas) = extract_formulas(markdown);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(&extracted, options);

    let mut events = Vec::new();
    for event in parser {
        match event {
            Event::Html(text) | Event::InlineHtml(text) => events.push(Event::Text(text)),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_destination(dest_url.as_ref(), ctx, true);
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: CowStr::Boxed(dest.into_boxed_str()),
                    title,
                    id,
                }));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_destination(dest_url.as_ref(), ctx, false);
                events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url: CowStr::Boxed(dest.into_boxed_str()),
                    title,
                    id,
                }));
            }
            other => events.push(other),
        }
    }

    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());

    for (index, formula) in formulas.iter().enumerate() {
        let placeholder = format!("\u{e000}{index}\u{e001}");
        let tex = escape_tex(&formula.tex);
        let replacement = if formula.display {
            format!("\\[{tex}\\]")
        } else {
            format!("\\({tex}\\)")
        };
        output = output.replace(&placeholder, &replacement);
    }
    output
}

fn extract_formulas(markdown: &str) -> (String, Vec<Formula>) {
    let mut output = String::new();
    let mut formulas = Vec::new();
    let bytes = markdown.as_bytes();
    let mut index = 0;
    let mut line_start = true;

    while index < bytes.len() {
        if line_start {
            if let Some((fence_char, fence_len)) = fence_open_at(markdown, index) {
                let end = find_fence_end(markdown, index, fence_char, fence_len);
                let segment = &markdown[index..end];
                output.push_str(segment);
                line_start = segment.ends_with('\n');
                index = end;
                continue;
            }
            line_start = false;
        }

        let byte = bytes[index];
        if byte == b'\n' {
            output.push('\n');
            index += 1;
            line_start = true;
            continue;
        }

        if byte == b'`' {
            let run = byte_run(bytes, index, b'`');
            let end = find_code_span_end(bytes, index + run, run).unwrap_or(bytes.len());
            let segment = &markdown[index..end];
            output.push_str(segment);
            line_start = segment.ends_with('\n');
            index = end;
            continue;
        }

        if byte == b'\\' && index + 1 < bytes.len() && bytes[index + 1] == b'$' {
            output.push_str("\\$");
            index += 2;
            continue;
        }

        if byte == b'$' && index + 1 < bytes.len() && bytes[index + 1] == b'$' {
            let content_start = index + 2;
            if let Some(offset) = find_unescaped(&markdown[content_start..], "$$") {
                let content = &markdown[content_start..content_start + offset];
                if content.is_empty() {
                    output.push_str("$$");
                    index += 2;
                } else {
                    let placeholder = push_formula(&mut formulas, content, true);
                    output.push_str(&placeholder);
                    index = content_start + offset + 2;
                }
            } else {
                output.push_str("$$");
                index += 2;
            }
            continue;
        }

        if byte == b'$' {
            let content_start = index + 1;
            if let Some(offset) = find_inline_closing(&markdown[content_start..]) {
                let content = &markdown[content_start..content_start + offset];
                if content.is_empty() {
                    output.push('$');
                    index += 1;
                } else {
                    let placeholder = push_formula(&mut formulas, content, false);
                    output.push_str(&placeholder);
                    index = content_start + offset + 1;
                }
            } else {
                output.push('$');
                index += 1;
            }
            continue;
        }

        let ch = markdown[index..]
            .chars()
            .next()
            .expect("valid char boundary");
        output.push(ch);
        index += ch.len_utf8();
    }

    (output, formulas)
}

fn fence_open_at(text: &str, start: usize) -> Option<(char, usize)> {
    let bytes = text.as_bytes();
    let mut index = start;
    let mut spaces = 0;
    while index < bytes.len() && bytes[index] == b' ' && spaces < 4 {
        index += 1;
        spaces += 1;
    }
    if spaces > 3 || index >= bytes.len() {
        return None;
    }
    let fence_char = bytes[index] as char;
    if fence_char != '`' && fence_char != '~' {
        return None;
    }
    let len = byte_run(bytes, index, fence_char as u8);
    (len >= 3).then_some((fence_char, len))
}

fn find_fence_end(text: &str, open_start: usize, fence_char: char, fence_len: usize) -> usize {
    let mut position = text[open_start..]
        .find('\n')
        .map(|offset| open_start + offset + 1)
        .unwrap_or(text.len());
    while position < text.len() {
        let line_end = text[position..]
            .find('\n')
            .map(|offset| position + offset + 1)
            .unwrap_or(text.len());
        if is_fence_close(&text[position..line_end], fence_char, fence_len) {
            return line_end;
        }
        position = line_end;
    }
    text.len()
}

fn is_fence_close(line: &str, fence_char: char, fence_len: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    let len = trimmed.chars().take_while(|ch| *ch == fence_char).count();
    len >= fence_len
}

fn push_formula(formulas: &mut Vec<Formula>, tex: &str, display: bool) -> String {
    let index = formulas.len();
    formulas.push(Formula {
        tex: tex.to_string(),
        display,
    });
    format!("\u{e000}{index}\u{e001}")
}

fn byte_run(bytes: &[u8], start: usize, needle: u8) -> usize {
    let mut index = start;
    while index < bytes.len() && bytes[index] == needle {
        index += 1;
    }
    index - start
}

fn find_code_span_end(bytes: &[u8], mut index: usize, run: usize) -> Option<usize> {
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let found = byte_run(bytes, index, b'`');
            if found == run {
                return Some(index + found);
            }
            index += found;
        } else {
            index += 1;
        }
    }
    None
}

fn find_inline_closing(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => return None,
            b'\\' => index += 2,
            b'$' => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn find_unescaped(value: &str, needle: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut index = 0;
    while index + needle_bytes.len() <= bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if &bytes[index..index + needle_bytes.len()] == needle_bytes {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn rewrite_destination(dest: &str, ctx: &StatementCtx<'_>, image: bool) -> String {
    if dest.is_empty() || dest.starts_with('#') || dest.starts_with('/') {
        return dest.to_string();
    }
    if dest.starts_with("//") {
        return if image {
            "about:blank".to_string()
        } else {
            "#".to_string()
        };
    }

    if let Some(colon) = dest.find(':') {
        let scheme = &dest[..colon];
        if scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            let allowed = matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "mailto"
            );
            if image {
                return "about:blank".to_string();
            }
            return if allowed {
                dest.to_string()
            } else {
                "#".to_string()
            };
        }
    }

    let (path, suffix) = split_suffix(dest);
    let encoded = path.split('/').map(segment).collect::<Vec<_>>().join("/");
    format!(
        "/c/{}/p/{}/files/{}{}",
        segment(ctx.contest_id),
        segment(ctx.problem_key),
        encoded,
        suffix
    )
}

fn split_suffix(dest: &str) -> (&str, &str) {
    let position = dest
        .char_indices()
        .find(|(_, ch)| *ch == '?' || *ch == '#')
        .map(|(index, _)| index);
    match position {
        Some(index) => dest.split_at(index),
        None => (dest, ""),
    }
}

fn escape_tex(tex: &str) -> String {
    tex.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> StatementCtx<'static> {
        StatementCtx {
            contest_id: "demo",
            problem_key: "A",
        }
    }

    #[test]
    fn inline_and_display_math_are_restored() {
        let html = render_statement("设 $a_1=2$。\n\n$$\\sum_{i=1}^n i$$\n", &ctx());
        assert!(html.contains("\\(a_1=2\\)"));
        assert!(html.contains("\\[\\sum_{i=1}^n i\\]"));
    }

    #[test]
    fn multiline_display_math_is_restored() {
        let html = render_statement("$$\na + b\n$$\n", &ctx());
        assert!(html.contains("\\[\na + b\n\\]"));
    }

    #[test]
    fn math_inside_code_is_not_extracted() {
        let html = render_statement("行内 `$x$`\n\n```cpp\nint x = $y$;\n```\n", &ctx());
        assert!(html.contains("<code>$x$</code>"));
        assert!(!html.contains("\\(x\\)"));
        assert!(html.contains("$y$"));
    }

    #[test]
    fn escaped_dollar_is_literal() {
        let html = render_statement("价格 \\$5 与 $x$", &ctx());
        assert!(html.contains("$5"));
        assert!(html.contains("\\(x\\)"));
    }

    #[test]
    fn raw_html_is_escaped_and_relative_images_rewritten() {
        let html = render_statement(
            "<script>alert(1)</script>\n\n![图](<images/a b.png>)\n",
            &ctx(),
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("/c/demo/p/A/files/images/a%20b.png"));
    }

    #[test]
    fn relative_links_are_rewritten_and_dangerous_schemes_removed() {
        let html = render_statement(
            "[图](images/a.png) [文档](docs/a.md?x=1#top) [坏](javascript:alert(1))",
            &ctx(),
        );
        assert!(html.contains("href=\"/c/demo/p/A/files/images/a.png\""));
        assert!(html.contains("href=\"/c/demo/p/A/files/docs/a.md?x=1#top\""));
        assert!(html.contains("href=\"#\""));
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn latex_delimiters_are_not_input_math() {
        let html = render_statement("\\(y\\) 与 \\[z\\]", &ctx());
        assert!(!html.contains("\\(y\\)"));
        assert!(html.contains("(y)"));
        assert!(html.contains("[z]"));
    }

    #[test]
    fn closing_dollar_is_required() {
        let html = render_statement("价格是 $5 元", &ctx());
        assert!(html.contains("$5"));
        assert!(!html.contains("\\("));
    }

    #[test]
    fn math_special_html_characters_are_escaped() {
        let html = render_statement("$a < b & c > d$", &ctx());
        assert!(html.contains("\\(a &lt; b &amp; c &gt; d\\)"));
    }

    #[test]
    fn tables_render() {
        let html = render_statement("|a|b|\n|-|-|\n|1|2|\n", &ctx());
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn leading_h1_title_is_stripped() {
        assert_eq!(
            strip_leading_title("# A. 两数之和\n\n输入两个整数。\n"),
            "输入两个整数。\n"
        );
        assert_eq!(
            strip_leading_title("\r\n# A. 两数之和\r\n\r\n正文\r\n"),
            "正文\r\n"
        );
        assert_eq!(strip_leading_title("# 只有标题"), "");
        assert_eq!(strip_leading_title("## 输入格式\n"), "## 输入格式\n");
        assert_eq!(strip_leading_title("正文\n# 后置标题"), "正文\n# 后置标题");
    }
}
