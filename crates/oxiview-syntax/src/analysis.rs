//! Source offsets here are UTF-8 bytes. Editors convert them to their protocol encoding.

use crate::{oxi, view};
use proc_macro2::{TokenStream, TokenTree};
use std::{collections::BTreeMap, ops::Range};
use syn::{
    Item, Pat, UseTree,
    parse::Parser,
    spanned::Spanned,
    visit::{self, Visit},
};

/// Declaration category in a source document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Named .oxi view.
    View,
    /// Named typed style.
    Style,
    /// Rust function.
    Function,
    /// Struct, enum, trait or type alias.
    Type,
    /// Rust constant or static.
    Constant,
    /// Rust module.
    Module,
    /// Function parameter, closure parameter or local binding.
    Local,
}

/// A source-backed name and the scope where it is visible.
#[derive(Clone, Debug)]
pub struct Declaration {
    /// Declared identifier.
    pub name: String,
    /// Declaration category.
    pub kind: Kind,
    /// Identifier in the original text.
    pub selection: Range<usize>,
    /// Declaration extent, including its body when present.
    pub extent: Range<usize>,
    /// Lexical visibility for locals.
    pub scope: Range<usize>,
    /// Enclosing inline module segments.
    pub module: Vec<String>,
    /// Signature or binding as written in the file.
    pub detail: String,
}

/// One leaf in a Rust use tree, including aliases and glob imports.
#[derive(Clone, Debug)]
pub struct Import {
    /// Name introduced into this module, or `*`.
    pub name: String,
    /// Path before renaming.
    pub path: Vec<String>,
    /// Enclosing inline module.
    pub module: Vec<String>,
    /// Lexical scope of the import.
    pub scope: Range<usize>,
}

/// An explicit file macro invocation.
#[derive(Clone, Debug)]
pub struct Inclusion {
    /// Cargo package-relative path from the literal.
    pub path: String,
    /// Position of the file literal.
    pub range: Range<usize>,
    /// The embedding mode.
    pub mode: crate::Source,
    /// Enclosing inline module.
    pub module: Vec<String>,
}

/// A compiler grammar diagnostic with a source range.
#[derive(Clone, Debug)]
pub struct Problem {
    /// Offending source bytes.
    pub range: Range<usize>,
    /// English diagnostic text.
    pub message: String,
}

/// Declarations, imports, inclusions and grammar diagnostics for one source revision.
#[derive(Default, Debug)]
pub struct Analysis {
    /// Source declarations and lexical bindings.
    pub declarations: Vec<Declaration>,
    /// Imported names.
    pub imports: Vec<Import>,
    /// Included .oxi sources.
    pub inclusions: Vec<Inclusion>,
    /// Errors from the same parser and tag expansion used by the compiler.
    pub problems: Vec<Problem>,
    /// Identifier tokens in the original text, excluding comments and string contents.
    pub identifiers: BTreeMap<String, Vec<Range<usize>>>,
}

/// Analyzes ordinary Rust (`None`), or a module/fragment in the requested grammar.
pub fn analyze(text: &str, mode: Option<crate::Source>) -> Analysis {
    let mut visitor = Index {
        text,
        result: Analysis::default(),
        module: Vec::new(),
        scope: 0..text.len(),
    };
    let parsed = match mode {
        None => syn::parse_file(text).map(|file| visitor.visit_file(&file)),
        Some(crate::Source::Module) => oxi::module
            .parse_str(text)
            .and_then(syn::parse2::<syn::File>)
            .map(|file| visitor.visit_file(&file)),
        Some(crate::Source::Fragment) => syn::parse_str::<view::ViewInput>(text)
            .and_then(view::expand)
            .and_then(syn::parse2::<syn::Expr>)
            .map(|expr| visitor.visit_expr(&expr)),
    };
    if let Err(errors) = parsed {
        for error in errors {
            let mut range = error.span().byte_range();
            let message = error.to_string();
            if range.is_empty() && message.contains("end of input") {
                range = text.len()..text.len();
            }
            visitor.result.problems.push(Problem { range, message });
        }
        if mode == Some(crate::Source::Module) {
            // Incomplete tags are normal while typing. Keep complete preceding
            // imports and declarations, using the compiler grammar for each prefix.
            let stop = visitor
                .result
                .problems
                .first()
                .map_or(text.len(), |e| e.range.start);
            let before = text.get(..stop).filter(|s| !s.is_empty()).unwrap_or(text);
            for (i, _) in before
                .rmatch_indices('\n')
                .filter(|(i, _)| {
                    text.get(i + 1..).is_some_and(|s| {
                        ["pub", "view ", "style ", "fn ", "use ", "#["]
                            .iter()
                            .any(|prefix| s.starts_with(prefix))
                    })
                })
                .take(4)
            {
                if let Some(file) = text
                    .get(..i + 1)
                    .and_then(|src| oxi::module.parse_str(src).ok())
                    .and_then(|tokens| syn::parse2::<syn::File>(tokens).ok())
                {
                    visitor.visit_file(&file);
                    break;
                }
            }
        }
    }
    if let Ok(tokens) = text.parse::<TokenStream>() {
        identifiers(tokens, text, &mut visitor.result.identifiers);
    }
    visitor.result
}

fn identifiers(tokens: TokenStream, text: &str, found: &mut BTreeMap<String, Vec<Range<usize>>>) {
    for token in tokens {
        match token {
            TokenTree::Ident(ident) => {
                let range = ident.span().byte_range();
                if let Some(name) = text.get(range.clone()).filter(|s| ident == *s) {
                    found
                        .entry(name.trim_start_matches("r#").to_owned())
                        .or_default()
                        .push(range);
                }
            }
            TokenTree::Group(group) => identifiers(group.stream(), text, found),
            _ => {}
        }
    }
}
struct Index<'a> {
    text: &'a str,
    result: Analysis,
    module: Vec<String>,
    scope: Range<usize>,
}

impl Index<'_> {
    fn declare(&mut self, name: &syn::Ident, kind: Kind, extent: Range<usize>, detail_end: usize) {
        let selection = name.span().byte_range();
        let Some(spelling) = self.text.get(selection.clone()).filter(|s| !s.is_empty()) else {
            return;
        };
        let name = spelling.trim_start_matches("r#").to_owned();
        let start = extent.start.min(selection.start);
        let detail = self
            .text
            .get(start..detail_end.max(selection.end))
            .unwrap_or(spelling);
        self.result.declarations.push(Declaration {
            name,
            kind,
            extent: start..extent.end.max(selection.end),
            selection,
            scope: self.scope.clone(),
            module: self.module.clone(),
            detail: detail.split_whitespace().collect::<Vec<_>>().join(" "),
        });
    }

    fn bind(&mut self, pattern: &Pat, scope: Range<usize>, header: Range<usize>) {
        struct Bindings<'a, 'src>(&'a mut Index<'src>, Range<usize>);
        impl<'ast> Visit<'ast> for Bindings<'_, '_> {
            fn visit_pat_ident(&mut self, pat: &'ast syn::PatIdent) {
                self.0
                    .declare(&pat.ident, Kind::Local, self.1.clone(), self.1.end);
                visit::visit_pat_ident(self, pat);
            }
        }
        let outer = std::mem::replace(&mut self.scope, scope);
        Bindings(self, header).visit_pat(pattern);
        self.scope = outer;
    }

    fn function_body(&mut self, signature: &syn::Signature, body: &syn::Block) {
        let braces = body.brace_token.span;
        let scope = braces.open().byte_range().end..braces.close().byte_range().start;
        for arg in &signature.inputs {
            if let syn::FnArg::Typed(arg) = arg {
                self.bind(&arg.pat, scope.clone(), arg.span().byte_range());
            }
        }
        self.visit_block(body);
    }

    fn import(&mut self, tree: &UseTree, segments: &mut Vec<String>) {
        let (name, leaf) = match tree {
            UseTree::Path(path) => {
                segments.push(path.ident.to_string());
                self.import(&path.tree, segments);
                segments.pop();
                return;
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    self.import(item, segments);
                }
                return;
            }
            UseTree::Name(item) => (item.ident.to_string(), item.ident.to_string()),
            UseTree::Rename(item) => (item.rename.to_string(), item.ident.to_string()),
            UseTree::Glob(_) => ("*".into(), "*".into()),
        };
        let mut path = segments.clone();
        if leaf != "self" && leaf != "*" {
            path.push(leaf);
        }
        let name = if name == "self" {
            path.last().cloned().unwrap_or(name)
        } else {
            name
        };
        self.result.imports.push(Import {
            name,
            path,
            module: self.module.clone(),
            scope: self.scope.clone(),
        });
    }
}

impl<'ast> Visit<'ast> for Index<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let simple = match item {
            Item::Struct(x) => Some((&x.ident, Kind::Type)),
            Item::Enum(x) => Some((&x.ident, Kind::Type)),
            Item::Type(x) => Some((&x.ident, Kind::Type)),
            Item::Trait(x) => Some((&x.ident, Kind::Type)),
            Item::Const(x) => Some((&x.ident, Kind::Constant)),
            Item::Static(x) => Some((&x.ident, Kind::Constant)),
            _ => None,
        };
        if let Some((name, kind)) = simple {
            self.declare(
                name,
                kind,
                item.span().byte_range(),
                name.span().byte_range().end,
            );
        }
        visit::visit_item(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.declare(
            &item.ident,
            Kind::Module,
            item.span().byte_range(),
            item.ident.span().byte_range().end,
        );
        self.module.push(item.ident.to_string());
        visit::visit_item_mod(self, item);
        self.module.pop();
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.import(&item.tree, &mut Vec::new());
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let ident = &item.sig.ident;
        let kind = match self.text.get(item.sig.fn_token.span.byte_range()) {
            Some("view") => Kind::View,
            Some("style") => Kind::Style,
            _ => Kind::Function,
        };
        let braces = item.block.brace_token.span;
        let start = match item.vis {
            syn::Visibility::Inherited => item.sig.fn_token.span,
            _ => item.vis.span(),
        }
        .byte_range()
        .start;
        self.declare(
            ident,
            kind,
            start..braces.close().byte_range().end,
            braces.open().byte_range().start,
        );
        self.function_body(&item.sig, &item.block);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.function_body(&item.sig, &item.block);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if let Some(body) = &item.default {
            self.function_body(&item.sig, body);
        }
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        self.bind(
            &expr.pat,
            expr.body.brace_token.span.join().byte_range(),
            expr.pat.span().byte_range(),
        );
        visit::visit_expr_for_loop(self, expr);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        let pattern = arm.pat.span().byte_range();
        self.bind(
            &arm.pat,
            pattern.end..arm.body.span().byte_range().end,
            pattern,
        );
        visit::visit_arm(self, arm);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let bytes = block.brace_token.span.join().byte_range();
        let outer = self.scope.clone();
        if !bytes.is_empty() {
            self.scope = bytes;
        }
        visit::visit_block(self, block);
        self.scope = outer;
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        self.bind(
            &local.pat,
            local.semi_token.span.byte_range().end..self.scope.end,
            local.let_token.span.byte_range().start..local.pat.span().byte_range().end,
        );
        visit::visit_local(self, local);
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        let scope = closure.body.span().byte_range();
        for pat in &closure.inputs {
            self.bind(pat, scope.clone(), pat.span().byte_range());
        }
        visit::visit_expr_closure(self, closure);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let Some(last) = mac.path.segments.last() else {
            return;
        };
        let mode = match last.ident.to_string().as_str() {
            "oxi_view" => crate::Source::Fragment,
            "oxi_module" => crate::Source::Module,
            _ => return,
        };
        if let Ok(literal) = mac.parse_body::<syn::LitStr>() {
            self.result.inclusions.push(Inclusion {
                path: literal.value(),
                range: literal.span().byte_range(),
                mode,
                module: self.module.clone(),
            });
        }
    }
}
