use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use super::super::palette;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Language {
    Rust,
    JavaScript,
    Python,
    Shell,
    Sql,
    Data,
    Generic,
}

pub(super) fn syntax_spans_for_language(code: &str, language: Language) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    let mut previous_word: Option<&str> = None;

    while cursor < code.len() {
        let rest = &code[cursor..];

        if is_line_comment(rest, language) {
            spans.push(styled(
                rest,
                Style::default()
                    .fg(palette().faint)
                    .add_modifier(Modifier::ITALIC),
            ));
            break;
        }
        if rest.starts_with("/*") {
            let end = rest.find("*/").map_or(rest.len(), |index| index + 2);
            spans.push(styled(
                &rest[..end],
                Style::default()
                    .fg(palette().faint)
                    .add_modifier(Modifier::ITALIC),
            ));
            cursor += end;
            continue;
        }
        if language == Language::Rust
            && let Some(end) = rust_raw_string_end(rest)
        {
            spans.push(styled(&rest[..end], Style::default().fg(palette().green)));
            cursor += end;
            previous_word = None;
            continue;
        }

        let character = rest.chars().next().expect("nonempty remainder");
        if character.is_whitespace() {
            let end = take_while(rest, char::is_whitespace);
            spans.push(Span::raw(&rest[..end]));
            cursor += end;
            continue;
        }
        if language == Language::Rust
            && character == '\''
            && let Some(end) = rust_lifetime_end(rest)
        {
            spans.push(styled(&rest[..end], Style::default().fg(palette().yellow)));
            cursor += end;
            previous_word = None;
            continue;
        }
        if matches!(character, '"' | '\'' | '`') {
            let end = quoted_end(rest, character);
            spans.push(styled(&rest[..end], Style::default().fg(palette().green)));
            cursor += end;
            previous_word = None;
            continue;
        }
        if character.is_ascii_digit() {
            let end = number_end(rest);
            spans.push(styled(&rest[..end], Style::default().fg(palette().orange)));
            cursor += end;
            previous_word = None;
            continue;
        }
        if is_identifier_start(character) {
            let end = take_while(rest, is_identifier_continue);
            let token = &rest[..end];
            let following = rest[end..].trim_start();
            let preceding = code[..cursor]
                .chars()
                .rev()
                .find(|next| !next.is_whitespace());
            spans.push(styled(
                token,
                identifier_style(token, previous_word, preceding, following, language),
            ));
            cursor += end;
            previous_word = Some(token);
            continue;
        }
        if character == '@' || (character == '#' && !is_line_comment(rest, language)) {
            let marker_len = character.len_utf8();
            let name_len = take_while(&rest[marker_len..], is_identifier_continue);
            let end = marker_len + name_len;
            spans.push(styled(&rest[..end], Style::default().fg(palette().yellow)));
            cursor += end;
            previous_word = None;
            continue;
        }

        let end = operator_end(rest);
        let token = &rest[..end];
        let style = if is_operator(token) {
            Style::default().fg(palette().red)
        } else if token == "::" {
            Style::default().fg(palette().cyan)
        } else {
            Style::default().fg(palette().muted)
        };
        spans.push(styled(token, style));
        cursor += end;
        if !matches!(token, "." | "::") {
            previous_word = None;
        }
    }

    spans
}

impl Language {
    pub(super) fn from_path(path: &str) -> Self {
        Self::from_name(path.rsplit('.').next().unwrap_or_default())
    }

    pub(super) fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "rs" | "rust" => Self::Rust,
            "js" | "jsx" | "javascript" | "ts" | "tsx" | "typescript" | "mjs" | "cjs" | "vue"
            | "svelte" => Self::JavaScript,
            "py" | "pyi" | "python" | "rb" | "ruby" => Self::Python,
            "sh" | "bash" | "zsh" | "fish" | "shell" | "console" => Self::Shell,
            "sql" | "db" | "sqlite" | "sqlite3" => Self::Sql,
            "json" | "jsonc" | "toml" | "yaml" | "yml" => Self::Data,
            _ => Self::Generic,
        }
    }
}

fn identifier_style(
    token: &str,
    previous: Option<&str>,
    preceding: Option<char>,
    following: &str,
    language: Language,
) -> Style {
    if is_keyword(token, language) {
        return Style::default()
            .fg(palette().purple)
            .add_modifier(Modifier::ITALIC);
    }
    if is_primitive(token) || starts_uppercase(token) {
        return Style::default().fg(palette().cyan);
    }
    if token
        .chars()
        .all(|next| next == '_' || next.is_ascii_uppercase() || next.is_ascii_digit())
        && token.chars().any(|next| next.is_ascii_uppercase())
    {
        return Style::default().fg(palette().orange);
    }
    if matches!(previous, Some("fn" | "def" | "function")) {
        return Style::default()
            .fg(palette().orange)
            .add_modifier(Modifier::BOLD);
    }
    if matches!(
        previous,
        Some("struct" | "enum" | "trait" | "type" | "class" | "interface")
    ) {
        return Style::default()
            .fg(palette().cyan)
            .add_modifier(Modifier::BOLD);
    }
    if following.starts_with('!') {
        return Style::default().fg(palette().yellow);
    }
    if following.starts_with('(') {
        return Style::default().fg(palette().orange);
    }
    if preceding == Some('.') {
        return Style::default().fg(palette().cyan);
    }
    if following.starts_with(':')
        || matches!(previous, Some("let" | "const" | "static" | "var"))
        || previous.is_some_and(starts_uppercase)
    {
        return Style::default().fg(palette().accent);
    }
    Style::default().fg(palette().ink)
}

fn is_line_comment(rest: &str, language: Language) -> bool {
    rest.starts_with("//")
        || (language == Language::Sql && rest.starts_with("--"))
        || (matches!(
            language,
            Language::Python | Language::Shell | Language::Data
        ) && rest.starts_with('#'))
}

fn rust_raw_string_end(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    if bytes.first() != Some(&b'r') {
        return None;
    }
    let hashes = bytes[1..].iter().take_while(|byte| **byte == b'#').count();
    if bytes.get(1 + hashes) != Some(&b'"') {
        return None;
    }
    let terminator = format!("\"{}", "#".repeat(hashes));
    let content_start = hashes + 2;
    Some(
        rest[content_start..]
            .find(&terminator)
            .map_or(rest.len(), |index| content_start + index + terminator.len()),
    )
}

fn rust_lifetime_end(rest: &str) -> Option<usize> {
    let after_quote = &rest[1..];
    let first = after_quote.chars().next()?;
    if !is_identifier_start(first) {
        return None;
    }
    let identifier_end = take_while(after_quote, is_identifier_continue);
    (!after_quote[identifier_end..].starts_with('\'')).then_some(identifier_end + 1)
}

fn quoted_end(rest: &str, quote: char) -> usize {
    let mut escaped = false;
    let mut end = quote.len_utf8();
    for next in rest[quote.len_utf8()..].chars() {
        end += next.len_utf8();
        if next == quote && !escaped {
            break;
        }
        escaped = next == '\\' && !escaped;
        if next != '\\' {
            escaped = false;
        }
    }
    end
}

fn number_end(rest: &str) -> usize {
    rest.char_indices()
        .find_map(|(index, next)| {
            (!matches!(next, '0'..='9' | 'a'..='f' | 'A'..='F' | 'x' | 'o' | '_' | '.'))
                .then_some(index)
        })
        .unwrap_or(rest.len())
}

fn operator_end(rest: &str) -> usize {
    const OPERATORS: [&str; 30] = [
        "<<=", ">>=", "..=", "...", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=",
        "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", "..", "??", "?.", "++", "--", "**",
        ":=",
    ];
    OPERATORS
        .iter()
        .find_map(|operator| rest.starts_with(operator).then_some(operator.len()))
        .unwrap_or_else(|| rest.chars().next().expect("nonempty remainder").len_utf8())
}

fn is_operator(token: &str) -> bool {
    token.chars().all(|next| {
        matches!(
            next,
            '=' | '+' | '-' | '*' | '/' | '%' | '!' | '<' | '>' | '&' | '|' | '^' | '?' | '~'
        )
    })
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn starts_uppercase(token: &str) -> bool {
    token.chars().next().is_some_and(char::is_uppercase)
}

fn take_while(input: &str, predicate: impl Fn(char) -> bool) -> usize {
    input
        .char_indices()
        .find_map(|(index, character)| (!predicate(character)).then_some(index))
        .unwrap_or(input.len())
}

fn styled(content: &str, style: Style) -> Span<'_> {
    Span::styled(content, style)
}

fn is_primitive(token: &str) -> bool {
    matches!(
        token,
        "bool"
            | "char"
            | "str"
            | "usize"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "f32"
            | "f64"
            | "string"
            | "number"
            | "object"
            | "void"
    )
}

fn is_keyword(token: &str, language: Language) -> bool {
    let common = matches!(
        token,
        "as" | "async"
            | "await"
            | "break"
            | "class"
            | "const"
            | "continue"
            | "def"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "from"
            | "function"
            | "if"
            | "import"
            | "in"
            | "interface"
            | "let"
            | "loop"
            | "match"
            | "new"
            | "none"
            | "null"
            | "return"
            | "static"
            | "struct"
            | "throw"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "var"
            | "where"
            | "while"
            | "yield"
    );
    common
        || (language == Language::Sql
            && matches!(
                token.to_ascii_lowercase().as_str(),
                "alter"
                    | "and"
                    | "as"
                    | "by"
                    | "case"
                    | "create"
                    | "default"
                    | "delete"
                    | "distinct"
                    | "drop"
                    | "else"
                    | "end"
                    | "exists"
                    | "from"
                    | "group"
                    | "having"
                    | "in"
                    | "index"
                    | "insert"
                    | "into"
                    | "join"
                    | "limit"
                    | "not"
                    | "null"
                    | "on"
                    | "or"
                    | "order"
                    | "primary"
                    | "references"
                    | "select"
                    | "set"
                    | "table"
                    | "then"
                    | "union"
                    | "unique"
                    | "update"
                    | "values"
                    | "view"
                    | "when"
                    | "where"
            ))
        || (language == Language::Rust
            && matches!(
                token,
                "crate"
                    | "dyn"
                    | "impl"
                    | "mod"
                    | "move"
                    | "mut"
                    | "pub"
                    | "ref"
                    | "self"
                    | "Self"
                    | "super"
                    | "unsafe"
                    | "use"
            ))
        || (language == Language::Python
            && matches!(
                token,
                "and"
                    | "elif"
                    | "except"
                    | "finally"
                    | "global"
                    | "is"
                    | "lambda"
                    | "nonlocal"
                    | "not"
                    | "or"
                    | "pass"
                    | "raise"
                    | "with"
            ))
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    #[test]
    fn highlights_rust_roles_without_losing_lifetimes() {
        let spans = syntax_spans_for_language(
            "fn render<'a>(value: Option<u32>) { let Some(item) = value.map(call); }",
            Language::Rust,
        );
        let style = |token: &str| {
            spans
                .iter()
                .find(|span| span.content == token)
                .map(|span| span.style)
                .unwrap()
        };

        assert!(style("fn").add_modifier.contains(Modifier::ITALIC));
        assert_eq!(style("render").fg, Some(palette().orange));
        assert_eq!(style("'a").fg, Some(palette().yellow));
        assert_eq!(style("Option").fg, Some(palette().cyan));
        assert_eq!(style("u32").fg, Some(palette().cyan));
        assert_eq!(style("map").fg, Some(palette().orange));
    }
}
