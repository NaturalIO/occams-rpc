use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, parse_macro_input};

pub fn service_mux_struct(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_struct = parse_macro_input!(item as Item);
    let (struct_name, fields) = match &item_struct {
        Item::Struct(item_struct) => (&item_struct.ident, &item_struct.fields),
        _ => panic!("The `service_mux_struct` attribute can only be applied to structs."),
    };

    let field_handlers = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        quote! {
            <#field_type as razor_rpc::server::ServiceStatic<C>>::SERVICE_NAME => self.#field_name.serve(req).await,
        }
    });

    let expanded = quote! {
        impl<C: razor_rpc::Codec> razor_rpc::server::ServiceStatic <C> for #struct_name {
            const SERVICE_NAME: &'static str = "";
            fn serve(&self, req: razor_rpc::server::task::APIServerReq<C>) -> impl std::future::Future<Output = ()> + Send {
                async move {
                    match req.service.as_str() {
                        #(#field_handlers)*
                        _ => req.set_rpc_error(razor_rpc::error::RpcIntErr::Service),
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
