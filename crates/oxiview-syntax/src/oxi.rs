use crate::view::{self, ViewInput};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use std::{env, fs, path::PathBuf};
use syn::{
    Attribute, ExprStruct, FnArg, Generics, Ident, Item, LitStr, Result, Stmt, Token, Visibility,
    braced, parenthesized,
    parse::{ParseStream, Parser, discouraged::Speculative},
    punctuated::Punctuated,
};

mod keyword {
    syn::custom_keyword!(view);
    syn::custom_keyword!(style);
}

/// Grammar selected by the importing file macro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Named Rust, view and style items.
    Module,
    /// A single root view embedded at its use site.
    Fragment,
}

/// Expands a file literal at a procedural macro invocation.
pub fn include(input: proc_macro::TokenStream, kind: Source) -> proc_macro::TokenStream {
    syn::parse::<LitStr>(input)
        .and_then(|name| expand_file(&name, kind))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_file(name: &LitStr, kind: Source) -> Result<TokenStream> {
    let relative = name.value();
    let dir = env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| syn::Error::new(name.span(), "expected a Cargo package directory"))?;
    let path = PathBuf::from(dir).join(&relative);
    if path.extension().is_none_or(|ext| ext != "oxi") {
        return Err(syn::Error::new(name.span(), "expected an .oxi source file"));
    }
    let source = fs::read_to_string(&path)
        .map_err(|error| syn::Error::new(name.span(), format!("{relative}: {error}")))?;
    let generated = match kind {
        Source::Module => module.parse_str(&source),
        Source::Fragment => syn::parse_str::<ViewInput>(&source).and_then(view::expand),
    }
    .map_err(|error| syn::Error::new(name.span(), format!("{relative}: {error}")))?;
    let path = path
        .to_str()
        .ok_or_else(|| syn::Error::new(name.span(), "the Cargo source path must be valid UTF-8"))?;
    // This anonymous const registers the source in rustc's dep-info. Its value
    // is unit, so the application has no template payload or runtime load.
    let dependency = quote!(
        const _: () = {
            let _ = ::core::include_bytes!(#path);
        };
    );
    Ok(match kind {
        Source::Module => quote!(#dependency #generated),
        Source::Fragment => quote!({ #dependency #generated }),
    })
}

pub(crate) fn module(input: ParseStream<'_>) -> Result<TokenStream> {
    let mut items = TokenStream::new();
    while !input.is_empty() {
        let ahead = input.fork();
        let attrs = ahead.call(Attribute::parse_outer)?;
        let visibility: Visibility = ahead.parse()?;
        let is_view = ahead.peek(keyword::view);
        if !(is_view || ahead.peek(keyword::style)) || !ahead.peek2(Ident) {
            input.parse::<Item>()?.to_tokens(&mut items);
            continue;
        }
        input.advance_to(&ahead);
        let keyword: Ident = input.parse()?;
        let function = quote_spanned!(keyword.span()=> fn);
        let name: Ident = input.parse()?;
        let generics: Generics = input.parse()?;
        let args;
        parenthesized!(args in input);
        let args = Punctuated::<FnArg, Token![,]>::parse_terminated(&args)?;
        let where_clause: Option<syn::WhereClause> = input.parse()?;
        let body;
        let braces = braced!(body in input);
        let (ty, expression) = if is_view {
            (quote!(::oxiview_core::View), view_body(&body)?)
        } else {
            (quote!(::oxiview_style::Style), style_body(&body)?)
        };
        let mut block = TokenStream::new();
        braces.surround(&mut block, |tokens| tokens.extend(expression));
        items.extend(quote! {
            #(#attrs)* #visibility #function #name #generics(#args) -> #ty #where_clause #block
        });
    }
    Ok(items)
}

fn view_body(body: ParseStream<'_>) -> Result<TokenStream> {
    let mut setup = TokenStream::new();
    while !body.is_empty() {
        // A block statement or a qualified Rust call can start like a view root.
        let ahead = body.fork();
        let statement = ahead.parse::<Stmt>();
        if (body.peek(Token![<]) || body.peek(syn::token::Brace))
            && (statement.is_err() || ahead.is_empty())
        {
            let root = view::expand(body.parse::<ViewInput>()?)?;
            return Ok(quote!(#setup #root));
        }
        statement?.to_tokens(&mut setup);
        body.advance_to(&ahead);
    }
    Err(body.error("expected a root view after the setup statements"))
}

fn style_body(body: ParseStream<'_>) -> Result<TokenStream> {
    let fields: TokenStream = body.parse()?;
    let mut style: ExprStruct = syn::parse2(quote!(::oxiview_style::Style { #fields }))?;
    if style.rest.is_none() {
        if style.dot2_token.is_some() {
            return Err(body.error("expected a base style after `..`"));
        }
        if !style.fields.is_empty() && !style.fields.trailing_punct() {
            style.fields.push_punct(Default::default());
        }
        style.dot2_token = Some(Default::default());
        style.rest = Some(syn::parse2(quote!(::core::default::Default::default()))?);
    }
    Ok(style.into_token_stream())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_fragment_and_named_views_emit_the_handwritten_builder_calls() -> Result<()> {
        let source = "<Node style={value}><Node/></Node>";
        let builder = quote!(
            ::oxiview_core::node()
                .style(value)
                .child(::oxiview_core::node())
        );
        let root = view::expand(syn::parse_str::<ViewInput>(source)?)?;
        assert_eq!(root.to_string(), builder.to_string());
        let named = module.parse_str(&format!("pub view sample(value: u32) {{ {source} }}"))?;
        let expected = quote!(pub fn sample(value: u32) -> ::oxiview_core::View { #builder });
        assert_eq!(named.to_string(), expected.to_string());
        Ok(())
    }

    #[test]
    fn setup_blocks_and_qualified_calls_precede_the_root() -> Result<()> {
        let view = module.parse_str(
            "view panel(mut n: u32) {
            { n += 1; }
            <u32 as Default>::default();
            <Node style={n}/>
        }",
        )?;
        let expected = quote! {
            fn panel(mut n: u32) -> ::oxiview_core::View {
                { n += 1; }
                <u32 as Default>::default();
                ::oxiview_core::node().style(n)
            }
        };
        assert_eq!(view.to_string(), expected.to_string());
        Ok(())
    }
}
