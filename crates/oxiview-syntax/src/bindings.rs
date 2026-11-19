//! Source identities preserved across lowering for editor navigation.

use crate::{
    Kind,
    view::{
        ViewInput,
        ast::{Node, builtin},
    },
};
use std::{collections::BTreeMap, ops::Range};
use syn::{Expr, Path, Type, ext::IdentExt, spanned::Spanned};

/// A Rust path and the byte offset where its names are in scope.
#[derive(Clone, Debug)]
pub struct SourcePath {
    /// Module or lexical path segments.
    pub segments: Vec<String>,
    /// A position in the source whose scope should be used for resolution.
    pub at: usize,
}

impl SourcePath {
    pub(crate) fn new(path: &Path, at: usize) -> Self {
        Self {
            segments: path
                .segments
                .iter()
                .map(|s| s.ident.unraw().to_string())
                .collect(),
            at,
        }
    }
}

/// Syntactic evidence for the type of a value. Rust remains the type checker.
#[derive(Clone, Debug)]
pub enum ValueSource {
    /// An explicit type or a struct literal.
    Type(SourcePath),
    /// The declared result of a called function.
    Call(SourcePath),
    /// A local value with a type annotation or initializer.
    Name(SourcePath),
    /// The result of another member access in the same source.
    Member(usize),
}

pub(crate) fn type_source(ty: &Type) -> Option<ValueSource> {
    match ty {
        Type::Path(p) if p.qself.is_none() => Some(ValueSource::Type(SourcePath::new(
            &p.path,
            p.span().byte_range().start,
        ))),
        Type::Reference(r) => type_source(&r.elem),
        Type::Paren(p) => type_source(&p.elem),
        Type::Group(g) => type_source(&g.elem),
        _ => None,
    }
}

pub(crate) fn expression_source(expr: &Expr) -> Option<ValueSource> {
    match expr {
        Expr::Call(call) => {
            let Expr::Path(p) = call.func.as_ref() else {
                return None;
            };
            Some(ValueSource::Call(SourcePath::new(
                &p.path,
                p.span().byte_range().start,
            )))
        }
        Expr::Path(p) => Some(ValueSource::Name(SourcePath::new(
            &p.path,
            p.span().byte_range().start,
        ))),
        Expr::Struct(s) => Some(ValueSource::Type(SourcePath::new(
            &s.path,
            s.span().byte_range().start,
        ))),
        Expr::MethodCall(call) => Some(ValueSource::Member(call.method.span().byte_range().start)),
        Expr::Field(field) => Some(ValueSource::Member(field.member.span().byte_range().start)),
        Expr::Reference(r) => expression_source(&r.expr),
        Expr::Paren(p) => expression_source(&p.expr),
        Expr::Group(g) => expression_source(&g.expr),
        Expr::Block(b) => match b.block.stmts.last()? {
            syn::Stmt::Expr(tail, None) => expression_source(tail),
            _ => None,
        },
        Expr::Macro(m)
            if m.mac
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "view") =>
        {
            view_value(&m.mac.parse_body::<ViewInput>().ok()?)
        }
        _ => None,
    }
}

/// How the source refers to a Rust member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberUse {
    /// An attribute lowered to a Builder call.
    Attribute,
    /// An explicit Rust method call.
    Method,
    /// A named field in a struct value.
    Field,
}

impl MemberUse {
    /// Declaration category selected by this use.
    pub fn declaration(self) -> Kind {
        match self {
            Self::Attribute | Self::Method => Kind::Method,
            Self::Field => Kind::Field,
        }
    }
}

/// The Rust operation denoted by an explicit source token.
#[derive(Clone, Debug)]
pub enum Binding {
    /// A factory or component path.
    Item(SourcePath),
    /// A field or inherent method selected on a particular receiver.
    Member {
        /// Receiver type evidence.
        receiver: ValueSource,
        /// Field or method name.
        name: String,
        /// How this token selects the member.
        usage: MemberUse,
    },
    /// The first typed input to a component tag.
    Props(SourcePath),
}

/// A binding and its original source range in UTF-8 bytes.
#[derive(Clone, Debug)]
pub struct BoundUse {
    /// Exact token to highlight, navigate or include in references.
    pub range: Range<usize>,
    /// What the token means after lowering.
    pub binding: Binding,
}

pub(crate) fn record(uses: &mut BTreeMap<usize, BoundUse>, range: Range<usize>, binding: Binding) {
    if !range.is_empty() {
        uses.entry(range.start)
            .or_insert(BoundUse { range, binding });
    }
}

fn view_value(view: &ViewInput) -> Option<ValueSource> {
    match &view.root {
        Node::Element { tag, .. } => {
            let mut path = SourcePath::new(&view.core, tag.span().byte_range().start);
            path.segments
                .push(if builtin(tag) { "node" } else { "component" }.into());
            Some(ValueSource::Call(path))
        }
        Node::Expression(expr) => expression_source(expr),
    }
}
pub(crate) fn record_view(view: &ViewInput, uses: &mut BTreeMap<usize, BoundUse>) {
    record_node(&view.root, &view.core, uses);
}

fn record_node(node: &Node, core: &Path, uses: &mut BTreeMap<usize, BoundUse>) {
    let Node::Element {
        tag,
        closing,
        attributes,
        children,
    } = node
    else {
        return;
    };
    let at = tag.span().byte_range().start;
    if builtin(tag) {
        let mut factory = SourcePath::new(core, at);
        factory.segments.push("node".into());
        for span in std::iter::once(tag.span()).chain(*closing) {
            record(uses, span.byte_range(), Binding::Item(factory.clone()));
        }
        for attr in attributes {
            record(
                uses,
                attr.name.span().byte_range(),
                Binding::Member {
                    receiver: ValueSource::Call(factory.clone()),
                    name: attr.name.unraw().to_string(),
                    usage: MemberUse::Attribute,
                },
            );
        }
    } else {
        for attr in attributes.iter().filter(|a| a.name == "props") {
            record(
                uses,
                attr.name.span().byte_range(),
                Binding::Props(SourcePath::new(tag, at)),
            );
        }
    }
    for child in children {
        record_node(child, core, uses);
    }
}
