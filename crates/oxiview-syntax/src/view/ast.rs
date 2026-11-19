//! Abstract syntax tree (AST) for a view expression, with source spans for editor navigation.
//! Both inline `view!` and external `.oxi` views use these nodes; embedded Rust uses syn's AST.

use proc_macro2::Span;
use syn::{Expr, Ident, Path};

pub(crate) struct ViewInput {
    pub(crate) core: Path,
    pub(crate) root: Node,
}
pub(crate) struct Attribute {
    pub(crate) name: Ident,
    pub(crate) value: Expr,
}
pub(crate) enum Node {
    Element {
        tag: Path,
        closing: Option<Span>,
        attributes: Vec<Attribute>,
        children: Vec<Node>,
    },
    Expression(Box<Expr>),
}

pub(crate) fn builtin(tag: &Path) -> bool {
    tag.segments.len() == 1
        && tag
            .segments
            .first()
            .is_some_and(|s| s.ident == "Node" || s.ident == "node")
}
