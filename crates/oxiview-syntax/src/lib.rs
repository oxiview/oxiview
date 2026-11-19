//! Shared compile-time grammar and byte-positioned source analysis for oxiview.

extern crate proc_macro;

mod analysis;
mod bindings;
mod oxi;
mod view;

pub use analysis::{Analysis, Declaration, Import, Inclusion, Kind, Problem, analyze};
pub use bindings::{Binding, BoundUse, MemberUse, SourcePath, ValueSource};
pub use oxi::{Source, include};

/// Releases standalone lexer source maps on a dedicated editor analysis thread.
/// Call after all token streams and spans on the thread have been dropped.
pub fn release_span_cache() {
    proc_macro2::extra::invalidate_current_thread_spans();
}

/// Expands the inline `view!` token stream with the compiler's original spans.
pub fn inline(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    syn::parse::<view::ViewInput>(input)
        .and_then(view::expand)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
