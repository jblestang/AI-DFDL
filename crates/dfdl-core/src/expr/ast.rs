//! Abstract Syntax Tree (AST) definitions for DFDL Expression Language.
//!
//! Follows DFDL 1.0 Specification §18. Defines literals, operators, variables,
//! function calls, conditional if-then-else, and relative/absolute path expressions.

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::infoset::value::DfdlValue;
use crate::types::{InfosetPath, QName};

/// Unary operators supported in DFDL expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Arithmetic identity (`+`).
    Plus,
    /// Arithmetic negation (`-`).
    Negate,
    /// Logical negation (`not`).
    Not,
}

/// Binary operators supported in DFDL expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// Arithmetic addition (`+`).
    Add,
    /// Arithmetic subtraction (`-`).
    Sub,
    /// Arithmetic multiplication (`*`).
    Mul,
    /// Floating point / number division (`div`).
    Div,
    /// Integer division (`idiv`).
    IDiv,
    /// Modulo operator (`mod`).
    Mod,
    /// Equality comparison (`=`).
    Eq,
    /// Inequality comparison (`!=`).
    Ne,
    /// Less than (`<`).
    Lt,
    /// Less than or equal (`<=`).
    Le,
    /// Greater than (`>`).
    Gt,
    /// Greater than or equal (`>=`).
    Ge,
    /// Logical AND (`and`).
    And,
    /// Logical OR (`or`).
    Or,
}

/// Abstract Syntax Tree node for a compiled DFDL expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprAst {
    /// Constant literal value.
    Literal(DfdlValue),
    /// Variable reference (e.g. `$var` or `$ns:var`).
    Variable(QName),
    /// Infoset path reference (e.g. `../header/len` or `/root/body`).
    Path(InfosetPath),
    /// Conditional if-then-else expression node (`if (cond) then A else B`).
    IfThenElse {
        /// Condition expression evaluated to boolean.
        cond: Box<ExprAst>,
        /// Result expression evaluated if condition is true.
        then_expr: Box<ExprAst>,
        /// Result expression evaluated if condition is false.
        else_expr: Box<ExprAst>,
    },
    /// Unary operation node.
    Unary {
        /// Operator kind.
        op: UnaryOp,
        /// Operand expression.
        expr: Box<ExprAst>,
    },
    /// Binary operation node.
    Binary {
        /// Operator kind.
        op: BinaryOp,
        /// Left-hand operand.
        left: Box<ExprAst>,
        /// Right-hand operand.
        right: Box<ExprAst>,
    },
    /// Function invocation node (e.g., `fn:concat(a, b)`).
    FnCall {
        /// Qualified function name.
        name: QName,
        /// Function argument expressions.
        args: Vec<ExprAst>,
    },
}
