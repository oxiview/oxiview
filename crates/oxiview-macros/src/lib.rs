//! Optional syntax sugar over oxiview's ordinary Rust view Builders.
//!
//! `view!` accepts XML-like nodes, Rust expression children, and function
//! components. It creates no retained tree or reactive runtime of its own.

use proc_macro::TokenStream;

mod oxi;
mod props;
mod view;

/// Lowers nested tags to ordinary `node()`, component, property, and child calls.
///
/// ```ignore
/// view! {
///     <Node style={panel_style}>
///         <Node bind_style={move || Ok(cell_style(score.get()?))} />
///         <score_panel props={ScoreProps { score }} />
///         { show(move || visible.get(), || Ok(details())) }
///     </Node>
/// }
/// ```
///
/// `Node` is the built-in node Builder. Other tags name function components:
/// `props={expr}` supplies their single props argument, or omit it for a
/// zero-argument component. Component functions return a View; fallible component
/// factories can be composed in an expression child. Attributes on Node map
/// directly to same-named Builder methods. Use `crate = path;` before the root
/// tag to select a renamed core or compatible facade path.
///
/// ```compile_fail
/// use oxiview_macros::view;
/// let _ = view! { <Node><Node /></Other> };
/// ```
#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    match syn::parse::<view::ViewInput>(input).and_then(view::expand) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Precompiles a package-relative `.oxi` file into Rust items in this module.
/// Ordinary Rust items, named `view` functions and typed `style` functions
/// can be freely combined. Place this invocation inside `mod name { ... }`
/// to give the exports a separate namespace.
#[proc_macro]
pub fn oxi_module(input: TokenStream) -> TokenStream {
    oxi::include(input, oxi::Source::Module)
}

/// Embeds a package-relative `.oxi` view fragment as a Builder expression.
/// Names resolve at the invocation site, including function-local bindings.
/// Uses the same tag and Rust-expression grammar and expansion as `view!`.
#[proc_macro]
pub fn oxi_view(input: TokenStream) -> TokenStream {
    oxi::include(input, oxi::Source::Fragment)
}

/// Generates a Builder for a named props struct without rerunning components.
///
/// Fields are required unless marked `#[prop(default)]` or
/// `#[prop(default = expression)]`. `PropsType::builder()` returns a Builder with
/// same-named field setters; `build()` returns a checked missing-field error.
#[proc_macro_derive(Props, attributes(prop))]
pub fn props(input: TokenStream) -> TokenStream {
    match syn::parse::<syn::DeriveInput>(input).and_then(props::expand) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
