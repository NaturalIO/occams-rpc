extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, FnArg, ImplItem, Item, ReturnType, Type};

fn get_result_type_from_future(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::ImplTrait(type_impl) = ty {
        for bound in &type_impl.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound {
                if let Some(segment) = trait_bound.path.segments.last() {
                    if segment.ident == "Future" {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                            for arg in &args.args {
                                if let syn::GenericArgument::Binding(binding) = arg {
                                    if binding.ident == "Output" {
                                        return Some(&binding.ty);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// A marker attribute for methods in an inherent `impl` block that should be exposed as RPC methods.
/// This is not needed when using a trait-based implementation.
#[proc_macro_attribute]
pub fn method(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// The `#[service]` macro is applied to an `impl` block to automatically generate the `ServiceTrait` implementation for the type.
///
/// - When applied to an inherent `impl` block, methods intended as service methods should be marked with `#[method]`.
/// - When applied to a trait `impl` block, all methods defined in the trait will be registered as service methods.
/// - All service methods must return `Result<T, RpcError<E>>`, where `E` is a user-defined error type that implements `RpcErrCodec`.
///
/// The service method recognizes:
/// - `fn` (which is considered non-blocking)
/// - `async fn`
/// - `impl Future`
/// - trait methods wrapped by `async_trait`
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    match input {
        Item::Impl(item_impl) => {
            let self_ty = &item_impl.self_ty;

            let methods_data: Vec<_> = item_impl
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

                            let returns_impl_future =
                                if let ReturnType::Type(_, ty) = &method.sig.output {
                                    get_result_type_from_future(ty).is_some()
                                } else {
                                    false
                                };

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

                            let should_await =
                                is_async_method || returns_pin_box_future || returns_impl_future;

                            Some((method_name, should_await, arg_ty))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            let handler_methods = methods_data.iter().map(|(method_name, should_await, arg_ty)| {
                let handler_name = format_ident!("__handle_{}", method_name);
                let await_token = if *should_await {
                    quote! { .await }
                } else {
                    quote! {}
                };

                quote! {
                    async fn #handler_name<C: Codec>(&self, req: Request<C>) {
                        let arg = match req.req.as_ref() {
                            None => {
                                unreachable!();
                            }
                            Some(buf) => match req.codec.decode::<#arg_ty>(&buf) {
                                Ok(arg) => arg,
                                Err(_) => {
                                    req.set_rpc_error(occams_rpc_core::error::RpcIntErr::Decode);
                                    return;
                                }
                            },
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

            let dispatch_arms = methods_data.iter().map(|(method_name, _, _)| {
                let method_name_str = method_name.to_string();
                let handler_name = format_ident!("__handle_{}", method_name);
                quote! {
                    #method_name_str => self.#handler_name(req).await,
                }
            });

            let (impl_generics, _ty_generics, where_clause) = item_impl.generics.split_for_impl();

            let mut service_trait_generics = item_impl.generics.clone();
            service_trait_generics.params.push(syn::parse_quote!(C: Codec));
            let (service_trait_impl_generics, _, service_trait_where_clause) =
                service_trait_generics.split_for_impl();

            let expanded = quote! {
                impl #impl_generics #self_ty #where_clause {
                    #(#handler_methods)*
                }

                impl #service_trait_impl_generics ServiceTrait<C> for #self_ty #service_trait_where_clause {
                    fn serve(&self, req: Request<C>) -> impl std::future::Future<Output = ()> + Send {
                        async move {
                            match req.method.as_str() {
                                #(#dispatch_arms)*
                                _ => {
                                    req.set_rpc_error(occams_rpc_core::error::RpcIntErr::Method);
                                }
                            }
                        }
                    }
                }
            };

            let final_code = quote! {
                #item_impl
                #expanded
            };

            TokenStream::from(final_code)
        }
        _ => panic!("The `service` attribute can only be applied to impl blocks."),
    }
}

/// The `#[service_mux_struct]` macro is applied to a **struct** to implement `ServiceTrait` on it.
/// It acts as a dispatcher, routing `serve()` calls to the correct service based on the `req.service` field.
///
/// Each field in the struct must hold a service that implements `ServiceTrait` (e.g., wrapped in an `Arc`).
/// The macro generates a `serve` implementation that matches `req.service` against the field names of the struct.
#[proc_macro_attribute]
pub fn service_mux_struct(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_struct = parse_macro_input!(item as Item);
    let (struct_name, fields) = match &item_struct {
        Item::Struct(item_struct) => (&item_struct.ident, &item_struct.fields),
        _ => panic!("The `service_mux_struct` attribute can only be applied to structs."),
    };

    let field_handlers = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        quote! {
            #field_name_str => self.#field_name.serve(req).await,
        }
    });

    let expanded = quote! {
        impl<C: Codec> ServiceTrait<C> for #struct_name {
            fn serve(&self, req: Request<C>) -> impl std::future::Future<Output = ()> + Send {
                async move {
                    match req.service.as_str() {
                        #(#field_handlers)*
                        _ => req.set_rpc_error(occams_rpc_core::error::RpcIntErr::Service),
                    }
                }
            }
        }
    };

    let final_code = quote! {
        #item_struct
        #expanded
    };

    TokenStream::from(final_code)
}
