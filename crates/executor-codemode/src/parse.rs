//! Recursive-descent parser for the subset.

use serde_json::{Number, Value};

use super::CodeError;
use super::ast::{Expr, Stmt};
use super::lex::{Token, tokenize};

/// Parse `source` into statements.
///
/// # Errors
///
/// Lexer or grammar errors.
pub fn parse(source: &str) -> Result<Vec<Stmt>, CodeError> {
    let tokens = tokenize(source)?;
    let mut p = Parser { tokens, i: 0 };
    p.program()
}

struct Parser {
    tokens: Vec<Token>,
    i: usize,
}

impl Parser {
    fn program(&mut self) -> Result<Vec<Stmt>, CodeError> {
        let mut stmts = Vec::new();
        while !self.at(&Token::Eof) {
            stmts.push(self.stmt()?);
            if stmts.len() > 256 {
                return Err(CodeError::new("too many statements"));
            }
        }
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<Stmt, CodeError> {
        match self.peek() {
            Token::Let | Token::Const => self.let_stmt(),
            Token::Return => {
                self.bump();
                let expr = self.expr()?;
                self.eat_semi();
                Ok(Stmt::Return(expr))
            }
            _ => {
                let expr = self.expr()?;
                self.eat_semi();
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn let_stmt(&mut self) -> Result<Stmt, CodeError> {
        self.bump();
        let Token::Ident(name) = self.bump() else {
            return Err(CodeError::new("expected binding name"));
        };
        self.expect(&Token::Eq)?;
        let expr = self.expr()?;
        self.eat_semi();
        Ok(Stmt::Let { name, expr })
    }

    fn expr(&mut self) -> Result<Expr, CodeError> {
        if self.at(&Token::Await) {
            self.bump();
            return Ok(Expr::Await(Box::new(self.expr()?)));
        }
        self.call()
    }

    fn call(&mut self) -> Result<Expr, CodeError> {
        let mut expr = self.primary()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    self.bump();
                    let Token::Ident(prop) = self.bump() else {
                        return Err(CodeError::new("expected property name"));
                    };
                    expr = Expr::Member {
                        object: Box::new(expr),
                        prop,
                    };
                }
                Token::LBrack => {
                    self.bump();
                    let index = self.expr()?;
                    self.expect(&Token::RBrack)?;
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                Token::LParen => {
                    self.bump();
                    let args = self.args()?;
                    self.expect(&Token::RParen)?;
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                    };
                }
                _ => return Ok(expr),
            }
        }
    }

    fn args(&mut self) -> Result<Vec<Expr>, CodeError> {
        let mut args = Vec::new();
        if self.at(&Token::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if self.at(&Token::Comma) {
                self.bump();
                continue;
            }
            break;
        }
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, CodeError> {
        match self.bump() {
            Token::Ident(name) => Ok(Expr::Ident(name)),
            Token::String(s) => Ok(Expr::Literal(Value::String(s))),
            Token::Number(n) => number_literal(&n),
            Token::True => Ok(Expr::Literal(Value::Bool(true))),
            Token::False => Ok(Expr::Literal(Value::Bool(false))),
            Token::Null => Ok(Expr::Literal(Value::Null)),
            Token::LBrace => self.object(),
            Token::LBrack => self.array(),
            Token::LParen => {
                let inner = self.expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            other => Err(CodeError::new(format!("unexpected token {other:?}"))),
        }
    }

    fn object(&mut self) -> Result<Expr, CodeError> {
        let mut fields = Vec::new();
        while !self.at(&Token::RBrace) && !self.at(&Token::Eof) {
            let key = self.object_key()?;
            self.expect(&Token::Colon)?;
            let value = self.expr()?;
            fields.push((key, value));
            if self.at(&Token::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(Expr::Object(fields))
    }

    fn object_key(&mut self) -> Result<String, CodeError> {
        match self.bump() {
            Token::Ident(s) | Token::String(s) => Ok(s),
            other => Err(CodeError::new(format!(
                "expected object key, got {other:?}"
            ))),
        }
    }

    fn array(&mut self) -> Result<Expr, CodeError> {
        let mut items = Vec::new();
        while !self.at(&Token::RBrack) && !self.at(&Token::Eof) {
            items.push(self.expr()?);
            if self.at(&Token::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(&Token::RBrack)?;
        Ok(Expr::Array(items))
    }

    fn peek(&self) -> Token {
        self.tokens.get(self.i).cloned().unwrap_or(Token::Eof)
    }

    fn at(&self, token: &Token) -> bool {
        &self.peek() == token
    }

    fn bump(&mut self) -> Token {
        let t = self.peek();
        if !matches!(t, Token::Eof) {
            self.i += 1;
        }
        t
    }

    fn expect(&mut self, token: &Token) -> Result<(), CodeError> {
        if self.at(token) {
            self.bump();
            Ok(())
        } else {
            Err(CodeError::new(format!("expected {token:?}")))
        }
    }

    fn eat_semi(&mut self) {
        if self.at(&Token::Semi) {
            self.bump();
        }
    }
}

fn number_literal(raw: &str) -> Result<Expr, CodeError> {
    if let Ok(n) = raw.parse::<i64>() {
        return Ok(Expr::Literal(Value::Number(n.into())));
    }
    let parsed: f64 = raw
        .parse()
        .map_err(|_| CodeError::new(format!("invalid number {raw}")))?;
    Number::from_f64(parsed)
        .map(Value::Number)
        .map(Expr::Literal)
        .ok_or_else(|| CodeError::new(format!("invalid number {raw}")))
}
