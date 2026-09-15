//! Hand-rolled lexer. No `unsafe`, no eval.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Token {
    Ident(String),
    String(String),
    Number(String),
    True,
    False,
    Null,
    Let,
    Const,
    Return,
    Await,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    LParen,
    RParen,
    Dot,
    Comma,
    Colon,
    Semi,
    Eq,
    Eof,
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, super::CodeError> {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        out.push(one(&chars, &mut i)?);
    }
    out.push(Token::Eof);
    Ok(out)
}

fn one(chars: &[char], i: &mut usize) -> Result<Token, super::CodeError> {
    let ch = chars[*i];
    *i += 1;
    match ch {
        '{' => Ok(Token::LBrace),
        '}' => Ok(Token::RBrace),
        '[' => Ok(Token::LBrack),
        ']' => Ok(Token::RBrack),
        '(' => Ok(Token::LParen),
        ')' => Ok(Token::RParen),
        '.' => Ok(Token::Dot),
        ',' => Ok(Token::Comma),
        ':' => Ok(Token::Colon),
        ';' => Ok(Token::Semi),
        '=' => Ok(Token::Eq),
        '"' => string(chars, i),
        c if c.is_ascii_digit() || c == '-' => Ok(number(chars, i, c)),
        c if is_ident_start(c) => Ok(ident(chars, i, c)),
        other => Err(super::CodeError::new(format!(
            "unexpected character {other:?}"
        ))),
    }
}

const fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

const fn is_ident_cont(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

fn ident(chars: &[char], i: &mut usize, first: char) -> Token {
    let mut s = String::new();
    s.push(first);
    while *i < chars.len() && is_ident_cont(chars[*i]) {
        s.push(chars[*i]);
        *i += 1;
    }
    match s.as_str() {
        "let" => Token::Let,
        "const" => Token::Const,
        "return" => Token::Return,
        "await" => Token::Await,
        "true" => Token::True,
        "false" => Token::False,
        "null" => Token::Null,
        _ => Token::Ident(s),
    }
}

fn string(chars: &[char], i: &mut usize) -> Result<Token, super::CodeError> {
    let mut s = String::new();
    while *i < chars.len() {
        let ch = chars[*i];
        *i += 1;
        match ch {
            '"' => return Ok(Token::String(s)),
            '\\' => {
                let esc = chars
                    .get(*i)
                    .copied()
                    .ok_or_else(|| super::CodeError::new("unterminated string escape"))?;
                *i += 1;
                s.push(match esc {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' | '\\' => esc,
                    other => other,
                });
            }
            other => s.push(other),
        }
    }
    Err(super::CodeError::new("unterminated string"))
}

fn number(chars: &[char], i: &mut usize, first: char) -> Token {
    let mut s = String::new();
    s.push(first);
    while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '.') {
        s.push(chars[*i]);
        *i += 1;
    }
    Token::Number(s)
}
