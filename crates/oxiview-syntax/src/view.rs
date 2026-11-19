use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use std::collections::BTreeSet;
use syn::{
    Expr, Ident, Path, Result, Stmt, Token, braced,
    parse::{Parse, ParseStream},
    spanned::Spanned,
};

pub(crate) struct ViewInput {
    core: Path,
    root: Node,
}
struct Attribute {
    name: Ident,
    value: Expr,
}
enum Node {
    Element {
        tag: Path,
        attributes: Vec<Attribute>,
        children: Vec<Node>,
    },
    Expression(Box<Expr>),
}
impl Parse for ViewInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let core = if input.peek(Token![crate]) {
            input.parse::<Token![crate]>()?;
            input.parse::<Token![=]>()?;
            let path = Path::parse_mod_style(input)?;
            input.parse::<Token![;]>()?;
            path
        } else {
            syn::parse2(quote!(::oxiview_core))?
        };
        let root = parse_node(input, 0)?;
        if !input.is_empty() {
            return Err(input.error("view! expects one root; wrap siblings in a Node"));
        }
        Ok(Self { core, root })
    }
}
fn expression(input: ParseStream<'_>) -> Result<Expr> {
    let content;
    let brace_token = braced!(content in input);
    let mut stmts = content.call(syn::Block::parse_within)?;
    if stmts.len() == 1
        && let Some(Stmt::Expr(expr, None)) = stmts.first()
    {
        return Ok(expr.clone());
    }
    Ok(Expr::Block(syn::ExprBlock {
        attrs: Vec::new(),
        label: None,
        block: syn::Block {
            brace_token,
            stmts: std::mem::take(&mut stmts),
        },
    }))
}
fn parse_node(input: ParseStream<'_>, depth: usize) -> Result<Node> {
    if depth >= 64 {
        return Err(input.error("view! nesting exceeds 64 levels"));
    }
    if input.peek(syn::token::Brace) {
        return Ok(Node::Expression(Box::new(expression(input)?)));
    }
    if !input.peek(Token![<]) {
        return Err(input.error("expected a view tag or a Rust {expression} child"));
    }
    input.parse::<Token![<]>()?;
    let tag = Path::parse_mod_style(input)?;
    let mut attributes = Vec::new();
    let mut names = BTreeSet::new();
    while !input.peek(Token![>]) && !input.peek(Token![/]) {
        let name: Ident = input.parse()?;
        if !names.insert(name.to_string()) {
            return Err(syn::Error::new(name.span(), "duplicate view property"));
        }
        input.parse::<Token![=]>()?;
        if !input.peek(syn::token::Brace) {
            return Err(input.error("view property values must be Rust expressions in braces"));
        }
        attributes.push(Attribute {
            name,
            value: expression(input)?,
        });
    }
    if input.peek(Token![/]) {
        input.parse::<Token![/]>()?;
        input.parse::<Token![>]>()?;
        return Ok(Node::Element {
            tag,
            attributes,
            children: Vec::new(),
        });
    }
    input.parse::<Token![>]>()?;
    let mut children = Vec::new();
    loop {
        if input.is_empty() {
            return Err(syn::Error::new_spanned(
                &tag,
                format!("missing closing tag for {}", tag.to_token_stream()),
            ));
        }
        let fork = input.fork();
        if fork.parse::<Token![<]>().is_ok() && fork.peek(Token![/]) {
            input.parse::<Token![<]>()?;
            input.parse::<Token![/]>()?;
            let closing = Path::parse_mod_style(input)?;
            if closing.to_token_stream().to_string() != tag.to_token_stream().to_string() {
                return Err(syn::Error::new_spanned(
                    closing,
                    format!("closing tag does not match {}", tag.to_token_stream()),
                ));
            }
            input.parse::<Token![>]>()?;
            break;
        }
        children.push(parse_node(input, depth + 1)?);
    }
    Ok(Node::Element {
        tag,
        attributes,
        children,
    })
}
pub(crate) fn expand(input: ViewInput) -> Result<TokenStream> {
    expand_node(input.root, &input.core)
}
fn expand_node(node: Node, core: &Path) -> Result<TokenStream> {
    match node {
        Node::Expression(expression) => Ok(quote!(#expression)),
        Node::Element {
            tag,
            attributes,
            children,
        } => {
            let builtin = tag.segments.len() == 1
                && tag
                    .segments
                    .first()
                    .is_some_and(|segment| segment.ident == "Node" || segment.ident == "node");
            let mut result = if builtin {
                let mut result = quote_spanned!(tag.span()=>#core::node());
                for attribute in attributes {
                    let method = attribute.name;
                    let value = attribute.value;
                    result = quote_spanned!(method.span()=>#result.#method(#value));
                }
                result
            } else {
                let mut props = None;
                for attribute in attributes {
                    if attribute.name != "props" {
                        return Err(syn::Error::new(
                            attribute.name.span(),
                            "component tags accept props={...}; Node properties belong on the component's returned Builder",
                        ));
                    }
                    props = Some(attribute.value);
                }
                let call = if let Some(props) = props {
                    quote!(#tag(#props))
                } else {
                    quote!(#tag())
                };
                quote_spanned!(tag.span()=>#core::component(move || ::core::result::Result::Ok(#call)))
            };
            for child in children {
                let child = expand_node(child, core)?;
                result = quote!(#result.child(#child));
            }
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_mismatched_and_duplicate_tags_with_specific_errors() {
        let mismatch = syn::parse_str::<ViewInput>("<Node><Node /></Other>");
        assert!(
            matches!(mismatch,Err(error) if error.to_string().contains("closing tag does not match"))
        );
        let duplicate = syn::parse_str::<ViewInput>("<Node style={a} style={b}/>");
        assert!(
            matches!(duplicate,Err(error) if error.to_string().contains("duplicate view property"))
        );
    }
    #[test]
    fn rejects_unbraced_values_and_multiple_roots() {
        assert!(syn::parse_str::<ViewInput>("<Node style=1/>").is_err());
        assert!(syn::parse_str::<ViewInput>("<Node/><Node/>").is_err());
    }
    #[test]
    fn component_props_are_explicit() -> Result<()> {
        let parsed = syn::parse_str::<ViewInput>("<panel color={1}/>")?;
        assert!(
            matches!(expand(parsed),Err(error) if error.to_string().contains("component tags accept props"))
        );
        Ok(())
    }
}
