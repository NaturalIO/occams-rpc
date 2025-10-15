extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ImplItem, Item, ItemEnum, ReturnType, Type};

#[proc_macro_attribute]
pub fn method(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    match input {
        Item::Impl(item_impl) => {
            let self_ty = &item_impl.self_ty;
            let mut generics = item_impl.generics.clone(); // Clone the original generics

            // Add C to the generics
            generics.params.push(syn::GenericParam::Type(syn::TypeParam {
                attrs: Vec::new(),
                ident: syn::Ident::new("C", proc_macro2::Span::call_site()),
                bounds: syn::punctuated::Punctuated::new(),
                colon_token: None,
                default: None,
                eq_token: None,
            }));

            let (impl_generics, _, where_clause) = generics.split_for_impl(); // Use the modified generics

            let methods_data: Vec<(_, _, _)> = item_impl
                .items
                .iter()
                .filter_map(|item| {
                    if let ImplItem::Method(method) = item {
                        if item_impl.trait_.is_some()
                            || method.attrs.iter().any(|attr| attr.path.is_ident("method"))
                        {
                            let method_name = method.sig.ident.clone();
                            let arg_ty: Type = method
                                .sig
                                .inputs
                                .iter()
                                .filter_map(|arg| {
                                    if let FnArg::Typed(pat_type) = arg {
                                        Some((*pat_type.ty).clone())
                                    } else {
                                        None
                                    }
                                })
                                .nth(0)
                                .expect("Method should have one argument besides &self");

                            let is_async_method = method.sig.asyncness.is_some();

                            let returns_pin_box_future = if let ReturnType::Type(_, ty) =
                                &method.sig.output
                            {
                                if let Type::Path(type_path) = &**ty {
                                    type_path.path.segments.last().map_or(false, |segment| {
                                        segment.ident == "Pin" && type_path.path.segments.len() > 1
                                    })
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            let should_await = is_async_method || returns_pin_box_future;

                            Some((method_name, should_await, arg_ty))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            let method_handlers = methods_data.iter().map(|(method_name, should_await, arg_ty)| {
                let method_name_str = method_name.to_string();

                let await_token = if *should_await {
                    quote! { .await }
                } else {
                    quote! {}
                };

                quote! {
                    #method_name_str => {
                        let arg = match req.req.as_ref() {
                            None => {
                                req.set_error(occams_rpc_core::error::RPC_ERR_DECODE);
                                return;
                            }
                            Some(buf) => {
                                match req.codec.decode::<#arg_ty>(&buf) {
                                    Ok(arg) => arg,
                                    Err(_) => {
                                        req.set_error(occams_rpc_core::error::RPC_ERR_DECODE);
                                        return;
                                    }
                                }
                            }
                        };

                        let res = self.#method_name(arg) #await_token;

                        match res {
                            Ok(resp) => {
                                req.set_result(resp);
                            }
                            Err(e) => {
                                req.set_error(e);
                            }
                        }
                    }
                }
            });

            let expanded = quote! {
                #item_impl

                impl #impl_generics ServiceTrait<C> for #self_ty #where_clause
                where
                    C: Codec,
                { fn serve(&self, req: Request<C>) -> impl std::future::Future<Output = ()> + Send {
                        async move {
                            match req.method.as_str() {
                                #(#method_handlers)*
                                _ => {
                                    req.set_error(occams_rpc_core::error::RPC_ERR_METHOD_NOT_FOUND);
                                }
                            }
                        }
                    }
                }
            };

            TokenStream::from(expanded)
        }
        _ => panic!("The `service` attribute can only be applied to impl blocks."),
    }
}

#[proc_macro_attribute]
pub fn service_enum(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemEnum);
    let enum_name = &input.ident;
    let variants = &input.variants;

    let variant_handlers = variants.iter().map(|variant| {
        let variant_name = &variant.ident;
        match &variant.fields {
            syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                quote! {
                    #enum_name::#variant_name(service) => service.serve(req).await,
                }
            }
            _ => panic!("service_enum variants must be newtype variants holding a service"),
        }
    });

    let expanded = quote! {
        #input

        impl<C: Codec> ServiceTrait<C> for #enum_name {
            fn serve(&self, req: Request<C>) -> impl std::future::Future<Output = ()> + Send {
                async move {
                    match self {
                        #(#variant_handlers)*
                    }
                }
            }
        }
    };

    TokenStream::from(expanded)
}
