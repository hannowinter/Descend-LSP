use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Attribute, Expr, FnArg, Ident, ItemTrait, Lit, Pat, ReturnType, TraitItem, Type};

// Extracts the "methodName" from [route("methodName")]
fn get_function_name(attrs: &Vec<Attribute>) -> Option<String> {
    for attr in attrs {
        let ident = attr.meta.path().get_ident()?;
        if ident.to_string() == "route" {
            let method: Expr = attr.parse_args().ok()?;
            if let Expr::Lit(method) = method {
                if let Lit::Str(method) = method.lit {
                    return Some(method.value());
                }
            }
            return None;
        }
    }
    None
}

#[proc_macro_attribute]
pub fn route(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast: syn::Result<syn::ItemTrait> = syn::parse(item.clone());
    if ast.is_err() {
        return item;
    }
    let ast = ast.unwrap();
    let router_ident = ast.ident; // "Router" identifier, name of the trait
    
    struct FunctionArg {
        ident: Ident,
        typ: Type
    }

    struct Function {
        rpc_method: String,
        ident: Ident,
        args: Vec<FunctionArg>,
        has_return: bool // determines whether it's for a request or notification
    }

    let mut fns: Vec<Function> = Vec::new();

    for trait_item in ast.items { // iterate through all of the traits functions
        if let TraitItem::Fn(fn_item) = trait_item {
            let function = get_function_name(&fn_item.attrs);
            if let None = function {
                continue;
            }
            let function = function.unwrap();

            let mut function_args: Vec<FunctionArg> = Vec::new();
            
            for arg in fn_item.sig.inputs { // iterate though all of the function's arguments
                if let FnArg::Typed(typed) = arg {
                    let pat = *typed.pat;
                    let ident = match pat { 
                        Pat::Ident(ident) => ident, 
                        _ => panic!("Error") 
                    }.ident;

                    function_args.push(FunctionArg { ident, typ: *typed.ty });
                }
            }

            fns.push(Function{ 
                rpc_method: function,
                ident: fn_item.sig.ident, 
                args: function_args,
                has_return: matches!(fn_item.sig.output, ReturnType::Type(..))
            });
        }
    }

    // Constructs the match cases for each request
    //
    // "methodName" => {
    //     struct Params {
    //         param1: Type1, param2: Type2, ...
    //     }
    //     let params = serde_json::from_value::<Params>(request.params).expect("Error"); // parse the params
    //     let result = router_inst.method_name(params.param1, params.param2, ...); // the actual routed function call
    //     match (result) {
    //         Ok(response) => Some(ResponseMessage{
    //             jsonrpc: request.jsonrpc,
    //             id: request.id,
    //             result: Some(serde_json::to_value(response).expect("Error while serializing result!")),
    //             error: None
    //         }),
    //         Err(error) => Some(ResponseMessage{
    //             jsonrpc: request.jsonrpc,
    //             id: request.id,
    //             result: None,
    //             error: Some(error)
    //         })
    //     }
    // }
    let request_match_cases_token = fns.iter().filter(|f| f.has_return).map(|f| {
        let rpc_method = &f.rpc_method;
        let function_name = &f.ident;

        // fields of the Params struct
        //let mut fields = vec![quote! {}; 0];
        let fields: Vec<proc_macro2::TokenStream> = f.args.iter().map(|arg| {
            let field_ident = &arg.ident;
            let field_type = &arg.typ;
            quote! {
                #field_ident: #field_type
            }
        }).collect();

        let field_names = f.args.iter().map(|arg| {
            arg.ident.clone()
        }).collect::<Vec<Ident>>();

        quote! {
            #rpc_method => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #( #fields ),*
                }
                let params = serde_json::from_value::<Params>(request.params).expect("Error while deserializing params!");
                let result = router_inst.#function_name(#( params.#field_names ),*);
                match (result) {
                    Ok(response) => Some(ResponseMessage {
                        jsonrpc: request.jsonrpc,
                        id: request.id,
                        result: Some(serde_json::to_value(response).expect("Error while serializing result!")),
                        error: None
                    }),
                    Err(error) => Some(ResponseMessage {
                        jsonrpc: request.jsonrpc,
                        id: request.id,
                        result: None,
                        error: Some(error)
                    })
                }
            }
        }
    }).collect::<Vec<proc_macro2::TokenStream>>();

    // Constructs the match cases for each notification
    //
    // "methodName" => {
    //     struct Params {
    //         param1: Type1, param2: Type2, ...
    //     }
    //     let params = serde_json::from_value::<Params>(request.params).expect("Error"); // parse the params
    //     router_inst.method_name(params.param1, params.param2, ...); // the actual routed function call
    //     return None
    // }
    let notification_match_cases_token = fns.iter().filter(|f| !f.has_return).map(|f| {
        let rpc_method = &f.rpc_method;
        let function_name = &f.ident;

        // fields of the Params struct
        let fields = f.args.iter().map(|arg| {
            let field_ident = &arg.ident;
            let field_type = &arg.typ;
            quote! {
                #field_ident: #field_type
            }
        }).collect::<Vec<proc_macro2::TokenStream>>();

        let field_names = f.args.iter().map(|arg| {
            arg.ident.clone()
        }).collect::<Vec<Ident>>();

        quote! {
            #rpc_method => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #( #fields ),*
                }
                let params = serde_json::from_value::<Params>(notification.params).expect("Error while deserializing params!");
                router_inst.#function_name(#( params.#field_names ),*);
                return None;
            }
        }
    }).collect::<Vec<proc_macro2::TokenStream>>();

    let item = parse_macro_input!(item as ItemTrait);

    // the actual route_msg function
    let route_fn = quote! {
        fn route_msg(router_inst: &mut impl #router_ident, message: Message) -> Option<ResponseMessage> {
            match message {
                Message::Request(request) => {
                    match request.method.as_str() {
                        #( #request_match_cases_token ),*
                        _ => Some(ResponseMessage {
                            jsonrpc: request.jsonrpc,
                            id: request.id,
                            result: None,
                            error: Some(ResponseError {
                                code: ResponseError::METHOD_NOT_FOUND,
                                message: format(format_args!("Unhandled request {}!", request.method)),
                                data: None
                            })
                        })
                    }
                },
                Message::Notification(notification) => {
                    match notification.method.as_str() {
                        #( #notification_match_cases_token ),*
                        _ => {
                            eprintln!("Unhandled notification {}!", notification.method);
                            None
                        }
                    }
                },
                Message::Response(result) => {
                    return None;
                },
                _ => {
                    eprintln!("Error while determining message type!");
                    return None;
                }
            }
        }
    };
    let tokens = quote! {
        #item
        #route_fn
    };
    tokens.into()
}