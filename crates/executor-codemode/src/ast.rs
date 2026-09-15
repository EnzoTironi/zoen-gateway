//! Statements and expressions for the bounded subset.

use serde_json::Value;

/// One statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `let` / `const` binding.
    Let {
        /// Binding name.
        name: String,
        /// Right-hand side.
        expr: Expr,
    },
    /// `return expr;`
    Return(Expr),
    /// Expression statement.
    Expr(Expr),
}

/// Expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// `await expr` (scheduling is already async; this is sugar).
    Await(Box<Self>),
    /// `callee(args...)`
    Call {
        /// Function expression.
        callee: Box<Self>,
        /// Arguments.
        args: Vec<Self>,
    },
    /// `object.prop`
    Member {
        /// Receiver.
        object: Box<Self>,
        /// Property name.
        prop: String,
    },
    /// `object[index]`
    Index {
        /// Receiver.
        object: Box<Self>,
        /// Index expression.
        index: Box<Self>,
    },
    /// Identifier.
    Ident(String),
    /// JSON literal.
    Literal(Value),
    /// `{ k: expr }`
    Object(Vec<(String, Self)>),
    /// `[expr, ...]`
    Array(Vec<Self>),
}
