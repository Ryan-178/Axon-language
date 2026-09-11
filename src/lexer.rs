//! Aoxn lexer: source text -> tokens with positions.
//!
//! Python-style layout: NEWLINE / INDENT / DEDENT tokens, `#` comments,
//! blank and comment-only lines produce no tokens, and inside parentheses
//! newlines are ignored (implicit line joining). Tabs count as 4 columns.

use crate::ast::Pos;
use crate::Diag;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    Def,
    Struct,
    Extern,
    Import,
    If,
    Elif,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Return,
    Pass,
    True,
    False,
    TyInt,
    TyFloat,
    TyBool,
    TyString,
    TyVoid,
    Newline,
    Indent,
    Dedent,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Dot,
    Colon,
    Arrow,
    Assign,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Comma,
    FStr(Vec<FStrPart>),
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

/// part of an f-string: literal text or an embedded expression (pre-lexed)
#[derive(Debug, Clone, PartialEq)]
pub enum FStrPart {
    Lit(String),
    ExprTokens(Vec<Token>),
}

pub fn lex(src: &str, file_id: u32) -> Result<Vec<Token>, Diag> {
    let mut out: Vec<Token> = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut indent_stack: Vec<usize> = vec![0];
    let mut paren_depth: i32 = 0;
    let mut at_line_start = true;

    macro_rules! adv {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    loop {
        // ---- line start: measure indentation (unless inside brackets) ----
        if at_line_start && paren_depth == 0 {
            let mut indent = 0usize;
            loop {
                match chars.get(i) {
                    Some(' ') => {
                        indent += 1;
                        adv!();
                    }
                    Some('\t') => {
                        indent = (indent / 4 + 1) * 4;
                        adv!();
                    }
                    _ => break,
                }
            }
            match chars.get(i) {
                None => {
                    at_line_start = false;
                }
                Some('\r') | Some('\n') => {
                    // blank line: no tokens
                    if *chars.get(i).unwrap() == '\r' {
                        adv!();
                    }
                    adv!(); // '\n'
                    continue;
                }
                Some('#') => {
                    while i < chars.len() && chars[i] != '\n' {
                        adv!();
                    }
                    continue;
                }
                Some(_) => {
                    let pos = Pos { line, col: 1, file: file_id };
                    let top = *indent_stack.last().unwrap();
                    if indent > top {
                        indent_stack.push(indent);
                        out.push(Token { tok: Tok::Indent, pos });
                    } else if indent < top {
                        while *indent_stack.last().unwrap() > indent {
                            indent_stack.pop();
                            out.push(Token { tok: Tok::Dedent, pos });
                        }
                        if *indent_stack.last().unwrap() != indent {
                            return Err(Diag {
                                stage: "lex", file: file_id,
                                line: pos.line,
                                col: pos.col,
                                message: format!(
                                    "unindent does not match any outer indentation level (expected one of {:?}, found {})",
                                    indent_stack, indent
                                ),
                            });
                        }
                    }
                    at_line_start = false;
                    // fall through to token scanning
                }
            }
        }

        let Some(&c) = chars.get(i) else { break };

        let pos = Pos { line, col, file: file_id };

        // ---- whitespace / line breaks ----
        if c == ' ' || c == '\t' || c == '\r' {
            adv!();
            continue;
        }
        if c == '\n' {
            adv!();
            if paren_depth == 0 {
                out.push(Token { tok: Tok::Newline, pos });
                at_line_start = true;
            }
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                adv!();
            }
            continue;
        }

        // ---- identifiers / keywords ----
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                adv!();
            }
            let word: String = chars[start..i].iter().collect();

            // f-string: `f"` (or `F"`) switches to interpolation scanning
            if (word == "f" || word == "F") && i < chars.len() && chars[i] == '"' {
                let parts = lex_fstring(&chars, &mut i, &mut line, &mut col, pos, file_id)?;
                out.push(Token { tok: Tok::FStr(parts), pos });
                continue;
            }

            let tok = match word.as_str() {
                "def" => Tok::Def,
                "struct" => Tok::Struct,
                "extern" => Tok::Extern,
                "import" => Tok::Import,
                "if" => Tok::If,
                "elif" => Tok::Elif,
                "else" => Tok::Else,
                "while" => Tok::While,
                "for" => Tok::For,
                "in" => Tok::In,
                "break" => Tok::Break,
                "continue" => Tok::Continue,
                "return" => Tok::Return,
                "pass" => Tok::Pass,
                "and" => Tok::AndAnd,
                "or" => Tok::OrOr,
                "not" => Tok::Bang,
                "true" | "True" => Tok::True,
                "false" | "False" => Tok::False,
                "int" => Tok::TyInt,
                "float" => Tok::TyFloat,
                "bool" => Tok::TyBool,
                "string" => Tok::TyString,
                "void" => Tok::TyVoid,
                _ => Tok::Ident(word),
            };
            out.push(Token { tok, pos });
            continue;
        }

        // ---- numbers ----
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                adv!();
            }
            let mut is_float = false;
            if i < chars.len() && chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                is_float = true;
                adv!(); // '.'
                while i < chars.len() && chars[i].is_ascii_digit() {
                    adv!();
                }
            }
            let text: String = chars[start..i].iter().collect();
            if is_float {
                let v: f64 = text.parse().map_err(|_| Diag {
                    stage: "lex", file: file_id,
                    line: pos.line,
                    col: pos.col,
                    message: format!("invalid float literal '{text}'"),
                })?;
                out.push(Token { tok: Tok::Float(v), pos });
            } else {
                let v: i64 = text.parse().map_err(|_| Diag {
                    stage: "lex", file: file_id,
                    line: pos.line,
                    col: pos.col,
                    message: format!("integer literal '{text}' out of range (max 9223372036854775807)"),
                })?;
                out.push(Token { tok: Tok::Int(v), pos });
            }
            continue;
        }

        // ---- strings ----
        if c == '"' {
            adv!(); // opening quote
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    return Err(Diag {
                        stage: "lex", file: file_id,
                        line: pos.line,
                        col: pos.col,
                        message: "unterminated string literal".into(),
                    });
                }
                let ch = chars[i];
                if ch == '"' {
                    adv!();
                    break;
                }
                if ch == '\\' {
                    adv!();
                    if i >= chars.len() {
                        return Err(Diag {
                            stage: "lex", file: file_id,
                            line: pos.line,
                            col: pos.col,
                            message: "unterminated string literal".into(),
                        });
                    }
                    let esc = chars[i];
                    match esc {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        _ => {
                            return Err(Diag {
                                stage: "lex", file: file_id,
                                line: pos.line,
                                col: pos.col,
                                message: format!("unknown escape sequence '\\{esc}'"),
                            })
                        }
                    }
                    adv!();
                    continue;
                }
                s.push(ch);
                adv!();
            }
            out.push(Token { tok: Tok::Str(s), pos });
            continue;
        }

        // ---- punctuation / operators ----
        macro_rules! single {
            ($t:expr) => {{
                out.push(Token { tok: $t, pos });
                adv!();
            }};
        }
        match c {
            '(' => {
                paren_depth += 1;
                single!(Tok::LParen);
            }
            '[' => {
                paren_depth += 1;
                single!(Tok::LBracket);
            }
            ')' | ']' => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return Err(Diag {
                        stage: "lex", file: file_id,
                        line: pos.line,
                        col: pos.col,
                        message: format!("unmatched closing '{c}'"),
                    });
                }
                single!(if c == ')' { Tok::RParen } else { Tok::RBracket });
            }
            ';' => single!(Tok::Semi),
            '.' => single!(Tok::Dot),
            ',' => single!(Tok::Comma),
            ':' => single!(Tok::Colon),
            '+' => single!(Tok::Plus),
            '-' => {
                if i + 1 < chars.len() && chars[i + 1] == '>' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::Arrow, pos });
                } else {
                    single!(Tok::Minus)
                }
            }
            '*' => single!(Tok::Star),
            '/' => single!(Tok::Slash),
            '%' => single!(Tok::Percent),
            '=' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::Eq, pos });
                } else {
                    single!(Tok::Assign)
                }
            }
            '!' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::Ne, pos });
                } else {
                    single!(Tok::Bang)
                }
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::Le, pos });
                } else {
                    single!(Tok::Lt)
                }
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::Ge, pos });
                } else {
                    single!(Tok::Gt)
                }
            }
            '&' => {
                if i + 1 < chars.len() && chars[i + 1] == '&' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::AndAnd, pos });
                } else {
                    return Err(Diag {
                        stage: "lex", file: file_id,
                        line: pos.line,
                        col: pos.col,
                        message: "unexpected character '&' (did you mean '&&' or 'and'?)".into(),
                    });
                }
            }
            '|' => {
                if i + 1 < chars.len() && chars[i + 1] == '|' {
                    adv!();
                    adv!();
                    out.push(Token { tok: Tok::OrOr, pos });
                } else {
                    return Err(Diag {
                        stage: "lex", file: file_id,
                        line: pos.line,
                        col: pos.col,
                        message: "unexpected character '|' (did you mean '||' or 'or'?)".into(),
                    });
                }
            }
            _ => {
                return Err(Diag {
                    stage: "lex", file: file_id,
                    line: pos.line,
                    col: pos.col,
                    message: format!("unexpected character '{c}'"),
                });
            }
        }
    }

    if paren_depth > 0 {
        return Err(Diag {
            stage: "lex", file: file_id,
            line,
            col,
            message: "unclosed '(' 鈥?bracket was never closed".into(),
        });
    }

    // final NEWLINE, then flush remaining DEDENTs, then EOF
    let eof_pos = Pos { line, col, file: file_id };
    if !matches!(out.last().map(|t| &t.tok), Some(Tok::Newline)) {
        out.push(Token { tok: Tok::Newline, pos: eof_pos });
    }
    while indent_stack.len() > 1 {
        indent_stack.pop();
        out.push(Token { tok: Tok::Dedent, pos: eof_pos });
    }
    out.push(Token { tok: Tok::Eof, pos: eof_pos });
    Ok(out)
}

/// scan an f-string literal (after `f"`): literal parts with escapes,
/// `{expr}` interpolations (pre-lexed recursively), `{{`/`}}` escapes.
fn lex_fstring(
    chars: &[char],
    i: &mut usize,
    line: &mut usize,
    col: &mut usize,
    open_pos: Pos,
    file_id: u32,
) -> Result<Vec<FStrPart>, Diag> {
    let mut parts: Vec<FStrPart> = Vec::new();
    let mut lit = String::new();

    macro_rules! adv {
        () => {{
            if chars[*i] == '\n' {
                *line += 1;
                *col = 1;
            } else {
                *col += 1;
            }
            *i += 1;
        }};
    }

    adv!(); // consume opening quote

    loop {
        if *i >= chars.len() {
            return Err(Diag {
                stage: "lex", file: file_id,
                line: open_pos.line,
                col: open_pos.col,
                message: "unterminated f-string literal".into(),
            });
        }
        let ch = chars[*i];

        if ch == '"' {
            adv!();
            break;
        }

        if ch == '{' {
            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                lit.push('{');
                adv!();
                adv!();
                continue;
            }
            if !lit.is_empty() {
                parts.push(FStrPart::Lit(std::mem::take(&mut lit)));
            }
            let brace_col = *col;
            adv!(); // consume '{'
            let start = *i;
            let mut depth: i32 = 0;
            loop {
                if *i >= chars.len() {
                    return Err(Diag {
                        stage: "lex", file: file_id,
                        line: open_pos.line,
                        col: open_pos.col,
                        message: "unterminated '{' in f-string".into(),
                    });
                }
                let c2 = chars[*i];
                match c2 {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '"' => {
                        adv!();
                        while *i < chars.len() && chars[*i] != '"' {
                            if chars[*i] == '\\' && *i + 1 < chars.len() {
                                adv!();
                            }
                            adv!();
                        }
                    }
                    '}' if depth == 0 => break,
                    _ => {}
                }
                adv!();
            }
            let expr_text: String = chars[start..*i].iter().collect();
            if *i >= chars.len() || chars[*i] != '}' {
                return Err(Diag {
                    stage: "lex", file: file_id,
                    line: open_pos.line,
                    col: open_pos.col,
                    message: "unterminated '{' in f-string".into(),
                });
            }
            adv!(); // consume '}'
            if expr_text.trim().is_empty() {
                return Err(Diag {
                    stage: "lex", file: file_id,
                    line: open_pos.line,
                    col: open_pos.col,
                    message: "empty '{}' in f-string".into(),
                });
            }
            // wrap in parens so the sub-lexer's line-start indent tracking is
            // disabled; the '(' also preserves exact original column numbers
            let padded: String = format!("({})", " ".repeat(brace_col - 1) + &expr_text);
            let toks = lex(&padded, file_id)?;
            parts.push(FStrPart::ExprTokens(toks));
            continue;
        }

        if ch == '}' {
            if *i + 1 < chars.len() && chars[*i + 1] == '}' {
                lit.push('}');
                adv!();
                adv!();
                continue;
            }
            return Err(Diag {
                stage: "lex", file: file_id,
                line: open_pos.line,
                col: open_pos.col,
                message: "single '}' in f-string (use '}}' for a literal brace)".into(),
            });
        }

        if ch == '\\' {
            adv!();
            if *i >= chars.len() {
                return Err(Diag {
                    stage: "lex", file: file_id,
                    line: open_pos.line,
                    col: open_pos.col,
                    message: "unterminated f-string literal".into(),
                });
            }
            let esc = chars[*i];
            match esc {
                'n' => lit.push('\n'),
                't' => lit.push('\t'),
                '\\' => lit.push('\\'),
                '"' => lit.push('"'),
                _ => {
                    return Err(Diag {
                        stage: "lex", file: file_id,
                        line: open_pos.line,
                        col: open_pos.col,
                        message: format!("unknown escape sequence '\\{esc}' in f-string"),
                    })
                }
            }
            adv!();
            continue;
        }

        lit.push(ch);
        adv!();
    }

    if !lit.is_empty() {
        parts.push(FStrPart::Lit(lit));
    }
    Ok(parts)
}





