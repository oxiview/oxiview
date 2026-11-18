use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Data, DeriveInput, Expr, Fields, Result, Token, spanned::Spanned};

enum DefaultValue {
    Required,
    Trait,
    Expression(Box<Expr>),
}
pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let visibility = &input.vis;
    let generics = &input.generics;
    let builder = format_ident!("{}Builder", name);
    let error = format_ident!("{}BuildError", name);
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "Props requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "Props can only be derived for structs",
            ));
        }
    };
    let mut storage = Vec::new();
    let mut empty = Vec::new();
    let mut setters = Vec::new();
    let mut values = Vec::new();
    let mut defaults = Vec::new();
    for field in fields {
        let Some(field_name) = &field.ident else {
            return Err(syn::Error::new_spanned(
                field,
                "Props requires named fields",
            ));
        };
        if field_name == "build" {
            return Err(syn::Error::new(
                field_name.span(),
                "the prop name build conflicts with the generated Builder method",
            ));
        }
        let ty = &field.ty;
        let mut default = DefaultValue::Required;
        let mut configured = false;
        for attr in &field.attrs {
            if !attr.path().is_ident("prop") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if !meta.path.is_ident("default") {
                    return Err(
                        meta.error("supported prop attribute: default or default = expression")
                    );
                }
                if configured {
                    return Err(meta.error("duplicate prop default"));
                }
                configured = true;
                default = if meta.input.peek(Token![=]) {
                    DefaultValue::Expression(Box::new(meta.value()?.parse()?))
                } else {
                    DefaultValue::Trait
                };
                Ok(())
            })?;
        }
        storage.push(quote!(#field_name: ::core::option::Option<#ty>));
        empty.push(quote!(#field_name: ::core::option::Option::None));
        let documentation = format!("Sets the {} prop.", field_name);
        setters.push(quote_spanned!(field.span()=>#[doc=#documentation] #[allow(dead_code)] pub fn #field_name(mut self,value:#ty)->Self {self.#field_name=::core::option::Option::Some(value);self}));
        let value = match default {
            DefaultValue::Required => {
                quote!(match self.#field_name {::core::option::Option::Some(value)=>value,::core::option::Option::None=>return ::core::result::Result::Err(#error {field:stringify!(#field_name)})})
            }
            DefaultValue::Trait => {
                defaults.push(quote!(#ty: ::core::default::Default));
                quote!(self.#field_name.unwrap_or_default())
            }
            DefaultValue::Expression(expression) => {
                quote!(self.#field_name.unwrap_or_else(||#expression))
            }
        };
        values.push(quote!(#field_name:#value));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let default_where = if defaults.is_empty() {
        quote!()
    } else {
        quote!(where #(#defaults,)*)
    };
    Ok(quote! {
        #[doc="Generated checked props Builder."]
        #[allow(dead_code)]
        #visibility struct #builder #generics #where_clause {#(#storage,)*}
        #[doc="A missing required prop."]
        #[allow(dead_code)]
        #[derive(Clone,Copy,Debug,PartialEq,Eq)]
        #visibility struct #error {#[doc="Name of the missing prop."] pub field:&'static str}
        impl ::core::fmt::Display for #error {fn fmt(&self,f:&mut ::core::fmt::Formatter<'_>)->::core::fmt::Result {write!(f,"missing required prop {}",self.field)}}
        impl ::std::error::Error for #error {}
        impl #impl_generics #name #type_generics #where_clause {
            #[doc="Creates a Builder for these props."]
            #[allow(dead_code)]
            pub fn builder()->#builder #type_generics {#builder {#(#empty,)*}}
        }
        impl #impl_generics #builder #type_generics #where_clause {
            #(#setters)*
            #[doc="Builds props or identifies the first missing required field."]
            #[allow(dead_code)]
            pub fn build(self)->::core::result::Result<#name #type_generics,#error> #default_where {::core::result::Result::Ok(#name {#(#values,)*})}
        }
    })
}
