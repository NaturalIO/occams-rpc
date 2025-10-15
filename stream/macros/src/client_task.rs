use proc_macro::TokenStream;
use quote::quote;
use syn::{AttributeArgs, DeriveInput, Fields, Ident, Meta, NestedMeta, Type, parse_macro_input};

fn check_option_inner_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(syn::TypePath { qself: None, path }) = ty {
        if path.leading_colon.is_none()
            && path.segments.len() == 1
            && path.segments[0].ident == "Option"
        {
            if let syn::PathArguments::AngleBracketed(args) = &path.segments[0].arguments {
                if args.args.len() == 1 {
                    if let syn::GenericArgument::Type(_inner_ty) = &args.args[0] {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn client_task_impl(attr: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as AttributeArgs);
    let mut ast = parse_macro_input!(input as DeriveInput);
    let struct_name = &ast.ident;
    let mut common_field: Option<(Ident, Type)> = None;
    let mut req_field: Option<Ident> = None;
    let mut resp_field: Option<(Ident, Type)> = None;
    let mut req_blob_field: Option<Ident> = None;
    let mut resp_blob_field: Option<(Ident, Type)> = None;
    let mut field_action: Option<(Ident, Type)> = None; // For #[field(action)]
    let mut static_action: Option<NestedMeta> = None; // For #[client_task(action)]
    let mut res_field: Option<(Ident, Type)> = None;
    let mut noti_field: Option<(Ident, Type)> = None;
    let mut gen_debug = false;

    for arg in args {
        match arg {
            NestedMeta::Meta(Meta::Path(path)) if path.is_ident("debug") => {
                if gen_debug {
                    panic!("Duplicate `debug` attribute argument");
                }
                gen_debug = true;
            }
            action_arg => {
                if static_action.is_some() {
                    panic!("Only one action can be specified.");
                }
                static_action = Some(action_arg);
            }
        }
    }

    if let syn::Data::Struct(syn::DataStruct { fields: Fields::Named(fields), .. }) = &mut ast.data
    {
        for field in fields.named.iter_mut() {
            let mut new_attrs = Vec::new();
            for attr in &field.attrs {
                if attr.path.is_ident("field") {
                    if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
                        for nested in meta_list.nested.iter() {
                            if let NestedMeta::Meta(Meta::Path(p)) = nested {
                                if let Some(ident) = p.get_ident() {
                                    let f_name = field.ident.as_ref().unwrap().clone();
                                    let f_type = field.ty.clone();
                                    match ident.to_string().as_str() {
                                        "common" => common_field = Some((f_name, f_type)),
                                        "req" => req_field = Some(f_name),
                                        "resp" => resp_field = Some((f_name, f_type)),
                                        "req_blob" => req_blob_field = Some(f_name),
                                        "resp_blob" => resp_blob_field = Some((f_name, f_type)),
                                        "action" => {
                                            // Handle #[field(action)]
                                            if field_action.is_some() {
                                                panic!("Only one #[field(action)] is allowed.");
                                            }
                                            field_action = Some((f_name, f_type));
                                        }
                                        "res" => res_field = Some((f_name, f_type)),
                                        "noti" => noti_field = Some((f_name, f_type)),
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                } else {
                    new_attrs.push(attr.clone());
                }
            }
            field.attrs = new_attrs;
        }
    }

    // Check for mutual exclusivity
    if field_action.is_some() && static_action.is_some() {
        panic!("Cannot specify both #[field(action)] and #[client_task(action = ...)] attributes.");
    }

    if res_field.is_some() != noti_field.is_some() {
        panic!("#[field(res)] and #[field(noti)] must be specified together.");
    }

    let (common_field_name, common_field_type) =
        common_field.expect("common field must be tagged with #[field(common)]");
    let req_field_name = req_field.expect("req field must be tagged with #[field(req)]");
    let (resp_field_name, resp_field_type) =
        resp_field.expect("resp field must be tagged with #[field(resp)]");

    if !check_option_inner_type(&resp_field_type) {
        panic!("`{}::{}` resp field must be of type Option<T>", struct_name, resp_field_name);
    }

    let get_req_blob_body = if let Some(req_blob_field_name) = req_blob_field {
        quote! {
            #[inline]
            fn get_req_blob(&self) -> Option<&[u8]> {
                Some(self.#req_blob_field_name.as_ref())
            }
        }
    } else {
        quote! {}
    };

    let reserve_resp_blob_body = if let Some((resp_blob_field_name, _)) = &resp_blob_field {
        quote! {
            #[inline]
            fn reserve_resp_blob(&mut self, size: i32) -> Option<&mut [u8]> {
                occams_rpc_core::io::AllocateBuf::reserve(&mut self.#resp_blob_field_name, size)
            }
        }
    } else {
        quote! {}
    };

    let client_task_action_impl = if let Some((f_action_name, f_action_type)) = field_action {
        let action_conversion = if let Type::Path(type_path) = &f_action_type {
            if let Some(segment) = type_path.path.segments.last() {
                if segment.ident == "String" {
                    quote! { occams_rpc_stream::proto::RpcAction::Str(&self.#f_action_name) }
                } else {
                    // Assume numeric or enum
                    quote! { occams_rpc_stream::proto::RpcAction::Num(self.#f_action_name as i32) }
                }
            } else {
                panic!("Unsupported type for #[field(action)]");
            }
        } else {
            panic!("Unsupported type for #[field(action)]");
        };

        quote! {
            impl occams_rpc_stream::client::ClientTaskAction for #struct_name {
                #[inline]
                fn get_action<'a>(&'a self) -> occams_rpc_stream::proto::RpcAction<'a> {
                    #action_conversion
                }
            }
        }
    } else if let Some(s_action) = static_action {
        let action_token_stream = match s_action {
            NestedMeta::Lit(syn::Lit::Str(s)) => {
                let action_str = s.value();
                quote! { occams_rpc_stream::proto::RpcAction::Str(#action_str) }
            }
            NestedMeta::Lit(syn::Lit::Int(i)) => {
                let action_int =
                    i.base10_parse::<i32>().expect("Invalid integer literal for action");
                quote! { occams_rpc_stream::proto::RpcAction::Num(#action_int) }
            }
            NestedMeta::Meta(syn::Meta::Path(p)) => {
                quote! { occams_rpc_stream::proto::RpcAction::Num(#p as i32) }
            }
            _ => panic!(
                "Unsupported action type in #[action(...)]. Only string/integer literals and enum variants are supported."
            ),
        };
        quote! {
            impl occams_rpc_stream::client::ClientTaskAction for #struct_name {
                #[inline]
                fn get_action<'a>(&'a self) -> occams_rpc_stream::proto::RpcAction<'a> {
                    #action_token_stream
                }
            }
        }
    } else {
        quote! {}
    };

    let client_task_done_impl = if let (Some((res_field_name, _)), Some((noti_field_name, _))) =
        (&res_field, &noti_field)
    {
        quote! {
            impl occams_rpc_stream::client::ClientTaskDone for #struct_name {
                #[inline]
                fn get_result(&self) -> Result<(), &occams_rpc_core::error::ServerErr> {
                    match self.#res_field_name.as_ref() {
                        Some(Ok(()))=>return Ok(()),
                        Some(Err(e))=>return Err(e),
                        None=>Err(&occams_rpc_core::error::RPC_ERR_INTERNAL)
                    }
                }

                #[inline]
                fn set_result(mut self, res: Result<(), occams_rpc_core::error::ServerErr>) {
                    self.#res_field_name.replace(res);
                    let noti = self.#noti_field_name.take().unwrap();
                    let _ = noti.send(self.into());
                }
            }
        }
    } else {
        quote! {}
    };

    let debug_impl = if gen_debug {
        quote! {
            impl std::fmt::Debug for #struct_name {
                fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(f, "{}(seq={} req={:?}", stringify!(#struct_name), self.seq, &self.#req_field_name)?;
                    if let Some(resp) = &self.#resp_field_name {
                        write!(f, " resp={:?}", resp)?;
                    }
                    write!(f, ")")
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #ast

        #debug_impl

        #client_task_action_impl

        #client_task_done_impl

        impl std::ops::Deref for #struct_name {
            type Target = #common_field_type;
            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.#common_field_name
            }
        }

        impl std::ops::DerefMut for #struct_name {
            #[inline]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.#common_field_name
            }
        }

        impl occams_rpc_stream::client::ClientTaskEncode for #struct_name {
            #[inline]
            fn encode_req<C: occams_rpc_core::Codec>(&self, codec: &C) -> Result<Vec<u8>, ()> {
                codec.encode(&self.#req_field_name)
            }

            #get_req_blob_body
        }

        impl occams_rpc_stream::client::ClientTaskDecode for #struct_name {
            #[inline]
            fn decode_resp<C: occams_rpc_core::Codec>(&mut self, codec: &C, buffer: &[u8]) -> Result<(), ()> {
                let resp = codec.decode(buffer)?;
                self.#resp_field_name = Some(resp);
                Ok(())
            }

            #reserve_resp_blob_body
        }
    };
    TokenStream::from(expanded)
}

/// ```compile_fail
/// use occams_rpc_stream_macros::*;
/// #[client_task]
/// pub struct FileTaskWrongResp {
///     #[field(common)]
///     common: occams_rpc_stream::client::ClientTaskCommon,
///     #[field(req)]
///     req: (),
///     #[field(resp)]
///     resp: (),
/// }
/// ```
#[doc(hidden)]
#[allow(dead_code)]
fn test_resp_not_option() {}

/// ```compile_fail
/// use occams_rpc_stream_macros::*;
/// #[client_task]
/// pub struct FileTaskNoReq {
///     #[field(common)]
///     common: occams_rpc_stream::client::ClientTaskCommon,
///     #[field(resp)]
///     resp: Option<()>,
/// }
/// ```
#[doc(hidden)]
#[allow(dead_code)]
fn test_missing_req() {}

/// ```compile_fail
/// use occams_rpc_stream_macros::*;
/// #[client_task]
/// pub struct FileTaskNoResp {
///     #[field(common)]
///     common: occams_rpc_stream::client::ClientTaskCommon,
///     #[field(req)]
///     req: (),
/// }
/// ```
#[doc(hidden)]
#[allow(dead_code)]
fn test_missing_resp() {}

/// ```compile_fail
/// use occams_rpc_stream_macros::*;
/// use occams_rpc_stream::client::ClientTaskCommon;
/// use occams_rpc_core::error::ServerErr;
///
/// #[client_task]
/// pub struct MissingNotiField {
///     #[field(common)]
///     common: ClientTaskCommon,
///     #[field(req)]
///     req: (),
///     #[field(resp)]
///     resp: Option<()>,
///     #[field(res)]
///     res: Option<Result<(), ServerErr>>,
/// }
/// ```
#[doc(hidden)]
#[allow(dead_code)]
fn test_missing_noti_field() {}

/// ```compile_fail
/// use occams_rpc_stream_macros::*;
/// use occams_rpc_stream::client::ClientTaskCommon;
/// use occams_rpc_core::error::ServerErr;
/// use crossfire::MTx;
///
/// #[client_task]
/// pub struct MissingResField {
///     #[field(common)]
///     common: ClientTaskCommon,
///     #[field(req)]
///     req: (),
///     #[field(resp)]
///     resp: Option<()>,
///     #[field(noti)]
///     noti: Option<MTx<Self>>,
/// }
/// ```
#[doc(hidden)]
#[allow(dead_code)]
fn test_missing_res_field() {}
