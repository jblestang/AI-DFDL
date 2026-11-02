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

impl ExprAst {
    /// Returns `true` if evaluating this expression AST may access the active Infoset document.
    ///
    /// Expressions consisting only of literals, variable references, scalar arithmetic,
    /// bitstream lookahead (`dfdlx:lookAhead`), occurs indexing (`dfdl:occursIndex`), or standard
    /// scalar string functions do not require the active Infoset document. Gating document creation
    /// behind this check avoids expensive deep-cloning of in-progress Infoset trees.
    #[must_use]
    pub fn needs_infoset_doc(&self) -> bool {
        match self {
            Self::Literal(_) | Self::Variable(_) => false,
            Self::Path(_) => true,
            Self::IfThenElse {
                cond,
                then_expr,
                else_expr,
            } => {
                cond.needs_infoset_doc()
                    || then_expr.needs_infoset_doc()
                    || else_expr.needs_infoset_doc()
            }
            Self::Unary { expr, .. } => expr.needs_infoset_doc(),
            Self::Binary { left, right, .. } => {
                left.needs_infoset_doc() || right.needs_infoset_doc()
            }
            Self::FnCall { name, args } => {
                // Functions that inspect the element tree even without explicit path arguments:
                if name.local_name == "checkConstraints"
                    || name.local_name == "count"
                    || name.local_name == "exists"
                    || name.local_name == "empty"
                    || name.local_name == "valueLength"
                    || name.local_name == "contentLength"
                {
                    return true;
                }
                args.iter().any(|arg| arg.needs_infoset_doc())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_infoset_doc_detection() {
        // Literals and variables do not need the document
        let lit = ExprAst::Literal(DfdlValue::Int(42));
        assert!(!lit.needs_infoset_doc());

        let var = ExprAst::Variable(QName::local("my_var"));
        assert!(!var.needs_infoset_doc());

        // Arithmetic between variables and literals does not need doc
        let bin = ExprAst::Binary {
            op: BinaryOp::Add,
            left: Box::new(lit.clone()),
            right: Box::new(var),
        };
        assert!(!bin.needs_infoset_doc());

        // Function call without doc inspection (e.g. lookAhead)
        let fn_la = ExprAst::FnCall {
            name: QName::with_namespace("urn:dfdlx", "lookAhead", Some("dfdlx")),
            args: alloc::vec![lit.clone(), ExprAst::Literal(DfdlValue::Int(8))],
        };
        assert!(!fn_la.needs_infoset_doc());

        // Path expressions ALWAYS need doc
        let path = ExprAst::Path(InfosetPath::parse("../header/len"));
        assert!(path.needs_infoset_doc());

        // Binary with path needs doc
        let bin_with_path = ExprAst::Binary {
            op: BinaryOp::Add,
            left: Box::new(lit),
            right: Box::new(path),
        };
        assert!(bin_with_path.needs_infoset_doc());

        // checkConstraints needs doc
        let cc = ExprAst::FnCall {
            name: QName::with_namespace("urn:dfdl", "checkConstraints", Some("dfdl")),
            args: alloc::vec![],
        };
        assert!(cc.needs_infoset_doc());
    }
}
