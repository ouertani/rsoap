//! A procedural macro crate for rsoap that parses WSDL files at compile-time
//! and generates typed Rust soap operation structs.

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, DeriveInput, MetaNameValue, Token};

/// Represents a single soap operation parsed from WSDL.
#[derive(Debug)]
#[allow(dead_code)]  // Fields are reserved for future type resolution
struct WsdlOperation {
    name: String,
    action: String,
    endpoint: String,
    namespace: String,
    body_element: String,
    request_fields: Vec<(String, String)>,
    response_fields: Vec<(String, String)>,
}

/// Represents a WSDL message definition with its parts.
#[derive(Debug)]
#[allow(dead_code)]  // Reserved for future XSD type resolution
struct WsdlMessage {
    name: String,
    part_name: String,
    element_name: String,
}

/// A parsed WSDL document with operations and type definitions.
#[derive(Debug)]
#[allow(dead_code)]  // Target namespace and messages are reserved for future use
struct ParsedWsdl {
    target_namespace: String,
    operations: Vec<WsdlOperation>,
    messages: Vec<WsdlMessage>,
}

impl ParsedWsdl {
     /// Parse a WSDL string into structured operation and message data.
    fn parse(wsdl: &str) -> Self {
        let target_namespace = extract_tag_content(wsdl, "targetNamespace")
            .unwrap_or_default();

        // Extract soap:operation blocks from binding section
        let mut operations = Vec::new();
        
        // SOAP bindings are inside <binding>... which contains multiple <operation> blocks
        let op_blocks = extract_tag_contents(wsdl, "operation");
        
        for op_block in &op_blocks {
            if let Ok(op) = parse_single_operation(wsdl, op_block, &target_namespace) {
                operations.push(op);
            }
        }

        // Extract message definitions for type resolution
        let messages: Vec<WsdlMessage> = extract_tag_contents(wsdl, "message")
            .iter()
            .filter_map(|msg| parse_message(msg.as_str()))
            .collect();

        Self {
            target_namespace,
            operations,
            messages,
        }
    }
}

/// Parse a single operation from WSDL XML string.
fn parse_single_operation(
    wsdl: &str,
    op_block: &str,
    namespace: &str,
) -> Result<WsdlOperation, String> {
    // Operation name comes from the <operation> tag attribute
    let name = extract_attribute(op_block, "name")
        .ok_or_else(|| "Could not find operation name".to_string())?;

    // soap:action URI from input block
    let inputs = extract_tag_contents(op_block, "input");
    let action_attr = if let Some(input) = inputs.first() {
        extract_attribute(input.as_str(), "soap:action")
            .unwrap_or_else(|| format!("{namespace}#{name}"))
    } else {
        format!("{namespace}#{name}")
    };

    // Endpoint from soap:address on binding or service
    let address = extract_tag_contents(wsdl, "soap:address")
        .first()
        .map(|s| extract_attribute(s, "location").unwrap_or_default())
        .or_else(|| {
            extract_tag_contents(wsdl, "address")
                .first()
                .map(|s| extract_attribute(s, "location").unwrap_or_default())
        })
        .unwrap_or_default();

    // Extract request/response fields from message parts
    let input_messages = &inputs;
    let (request_fields, response_fields) = parse_input_output(wsdl, input_messages);

    Ok(WsdlOperation {
        name: name.clone(),
        action: action_attr,
        endpoint: address,
        namespace: namespace.to_string(),
        body_element: format!("{name}Request"),
        request_fields,
        response_fields,
    })
}

/// Parse input/output blocks to get field definitions.
#[allow(clippy::type_complexity)]
fn parse_input_output(
    wsdl: &str,
    input_blocks: &[String],
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut request_fields = Vec::new();
    let mut response_fields = Vec::new();

    // Find the message references and their parts
    for input in input_blocks {
        if let Some(msg_ref) = extract_attribute(input, "message") {
             let msg_name = msg_ref.split(':').next_back().unwrap_or(&msg_ref).to_string();
              parse_message_parts(wsdl, &msg_name, true, &mut request_fields);
            }
         }

          // Output blocks follow a similar pattern but with <output> tags
     let output_blocks = extract_tag_contents(wsdl, "output");
     for output in &output_blocks {
         if let Some(msg_ref) = extract_attribute(output, "message") {
             let msg_name = msg_ref.split(':').next_back().unwrap_or(&msg_ref).to_string();
            parse_message_parts(wsdl, &msg_name, false, &mut response_fields);
        }
    }

    (request_fields, response_fields)
}

/// Parse message parts and their referenced types to build field lists.
fn parse_message_parts(
    wsdl: &str,
    msg_name: &str,
    _is_request: bool,
    fields_out: &mut Vec<(String, String)>,
) {
    // Find the message element with this name in WSDL <message> definitions
    let mut found = false;
    for msg_block in &extract_tag_contents(wsdl, "message") {
        if extract_attribute(msg_block, "name").as_deref() == Some(msg_name) {
            // Extract each <part .../> inside this message
            for part in &extract_tag_contents(msg_block, "part") {
                if let Some(part_name) = extract_attribute(part, "name") {
                    fields_out.push((
                        camel_to_snake(&part_name),
                        "String".into(), // Default to String; full XSD resolution is complex
                    ));
                 } else if let Some(elem_ref) = extract_attribute(part, "element") {
                    let ty = elem_ref.split(':').map(str::trim).next_back().unwrap_or(elem_ref.as_str());
                     fields_out.push((
                         camel_to_snake(ty),
                         ty.to_string(),
                       ));
                }
            }
            found = true;
        }
    }

    if !found {
        // If message lookup fails, generate a generic String field as fallback
        fields_out.push(("data".into(), "String".into()));
    }
}

/// Parse a WSDL message definition into a struct.
fn parse_message(content: &str) -> Option<WsdlMessage> {
     let name = extract_attribute(content, "name")?;

     let part_str = extract_tag_contents(content, "part").into_iter().next()?;
      let part_name = extract_attribute(part_str.as_str(), "name")?;
     let element_ref = extract_attribute(part_str.as_str(), "element")
           .or_else(|| extract_attribute(part_str.as_str(), "type"));

     Some(WsdlMessage {
         name,
         part_name,
         element_name: element_ref.unwrap_or_default(),
       })
}

fn camel_to_snake(s: &str) -> String {
    s.chars()
        .enumerate()
        .fold(String::new(), |mut acc, (i, c)| {
            if i > 0 && c.is_uppercase() {
                acc.push('_');
            }
            acc.extend(c.to_lowercase());
            acc
        })
}

// ─────────── MAIN MACRO ENTRY POINT ───────────

/// The main derive macro entry point.
///
/// # Usage
/// ```ignore
/// use rsoap::SoapOperation;
///
/// #[derive(SoapOperation)]
/// #[soap(wsdl = "path/to/service.wsdl", operation_name = "GetWeather")]
/// pub struct GetWeather;
/// ```
///
/// The macro reads the WSDL file at compile time, parses the types and operations,
/// and generates:
/// - Request and response Rust structs with `serde::Serialize` / `serde::Deserialize`
/// - An implementation of `SoapOperation` trait provided by `rsoap`
#[proc_macro_derive(SoapOperation, attributes(soap))]
pub fn soap_operation(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;


     // Extract the wsdl and operation_name attributes from #[soap(...)]
      #[allow(clippy::cmp_owned)]
      let soap_attr = input.attrs.iter().find(|a| {
          a.path()
                .get_ident()
                .map(|p| p.to_string() == "soap")
                .unwrap_or(false)
        })
            .expect("`#[soap(wsdl = \"...\", operation_name = \"...\")]` attribute is required");

     let attrs = extract_soap_meta(soap_attr);

    let wsdl_path = attrs.get("wsdl").cloned();
    let operation_name = attrs.get("operation_name").cloned();

    let wsdl_str = match &wsdl_path {
        Some(path) => std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!("Failed to read WSDL file '{path}': {e}")
          }),
        None => String::new(),
     };

    let operation_name = operation_name.as_deref().unwrap_or("");

    let generated = if wsdl_str.is_empty() {
        generate_placeholder(struct_name, operation_name)
     } else {
        let parsed = ParsedWsdl::parse(&wsdl_str);
        
        match parsed.operations.iter().find(|op| op.name == operation_name) {
            Some(op) => generate_from_wsdl(op, struct_name),
            None => panic!(
                 "operation '{}' not found in WSDL (available: {})",

                operation_name,
                parsed.operations.iter()
                      .map(|op| op.name.as_str())
                      .collect::<Vec<_>>()
                      .join(", ")
            ),
        }
    };

    TokenStream::from(quote! { #generated })
}

/// Extract key-value pairs from soap() attribute using syn's Meta API.
fn extract_soap_meta(attr: &syn::Attribute) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut attrs = HashMap::new();

    if let syn::Meta::List(list) = &attr.meta {
         // Parse content inside soap(...) as comma-separated name=value pairs
         let parser = |content: syn::parse::ParseStream| -> syn::Result<Vec<MetaNameValue>> {
             let pairs: Punctuated<MetaNameValue, Token![,]> =
                 Punctuated::parse_terminated(content)?;
             Ok(pairs.into_iter().collect())
         };

         if let Ok(nvs) = list.parse_args_with(parser) {
             for nv in &nvs {
                 let key = nv.path.get_ident()
                      .map(|i| i.to_string())
                      .unwrap_or_default();

                 if let Some(value_str) = parse_lit_from_expr(&nv.value) {
                     attrs.insert(key, value_str);
                  }
             }
         }
     }

    attrs
}

/// Extract a literal string value from a syn::Expr.
fn parse_lit_from_expr(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Str(s) => Some(s.value()),
            syn::Lit::Int(i) => Some(i.to_string()),
            syn::Lit::Float(f) => Some(f.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// Generate placeholder code when no WSDL is available (manual field mode).
fn generate_placeholder(
    strukt_name: &syn::Ident,
    operation_name: &str,
) -> TokenStream2 {
    let mod_name = format_ident!("{}", operation_name.to_lowercase());
    let req_struct = format_ident!("{operation_name}Request");
    let resp_struct = format_ident!("{operation_name}Response");

    quote! {
        #[allow(non_camel_case_types)]
        pub mod #mod_name {
            use ::rsoap::serde::{Serialize, Deserialize};

            /// The request struct for this SOAP operation.
            /// Define your fields manually to match the WSDL schema.
            #[derive(Debug, Clone, Default, Serialize, Deserialize)]
            pub struct #req_struct;

            /// The response struct for this soap operation.
            /// Define your fields manually to match the WSDL schema.
            #[derive(Debug, Clone, Default, Serialize, Deserialize)]
            pub struct #resp_struct;
        }

        impl ::rsoap::SoapOperation for #strukt_name {
            type Request = #mod_name::#req_struct;
            type Response = #mod_name::#resp_struct;

            const ACTION: &'static str = "";
            const ENDPOINT: &'static str = "";
            const BODY_ELEMENT: &'static str = "";
        }
    }
}

/// Generate full typed code when WSDL parsing succeeds.
fn generate_from_wsdl(
    wsdl_op: &WsdlOperation,
    strukt_name: &syn::Ident,
) -> TokenStream2 {
    let mod_name = format_ident!("{}", wsdl_op.name.to_lowercase());
    let req_struct = format_ident!("{}Request", wsdl_op.name);
    let resp_struct = format_ident!("{}Response", wsdl_op.name);
    let body_element = &wsdl_op.body_element;

    // Build request field tokens
    let request_fields: Vec<TokenStream2> = wsdl_op
        .request_fields
        .iter()
        .map(|(name, ty)| {
            let ident = format_ident!("{name}");
            quote! { pub #ident: #ty }
        })
        .collect();

    // Build response field tokens
    let response_fields: Vec<TokenStream2> = wsdl_op
        .response_fields
        .iter()
        .map(|(name, ty)| {
            let ident = format_ident!("{name}");
            quote! { pub #ident: #ty }
        })
        .collect();

    let action = &wsdl_op.action;
    let endpoint = &wsdl_op.endpoint;

    quote! {
        /// Generated request and response types for the #action operation.
        #[allow(non_camel_case_types)]
        pub mod #mod_name {
            use ::rsoap::serde::{Serialize, Deserialize};

            /// The request struct for this SOAP operation.
            #[derive(Debug, Clone, Serialize, Deserialize)]
            pub struct #req_struct {
                #(#request_fields),*
            }

            /// The response struct for this soap operation.
            #[derive(Debug, Clone, Default, Serialize, Deserialize)]
            pub struct #resp_struct {
                #(#response_fields),*
            }
        }

        impl ::rsoap::SoapOperation for #strukt_name {
            type Request = #mod_name::#req_struct;
            type Response = #mod_name::#resp_struct;

            const ACTION: &'static str = #action;
            const ENDPOINT: &'static str = #endpoint;
            const BODY_ELEMENT: &'static str = #body_element;
        }
    }
}

// ─────────── WSDL XML PARSING HELPERS ───────────

/// Extract the content inside the first `<tagname>...</tagname>` occurrence.
fn extract_tag_content(wsdl: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");

    let start_pos = wsdl.find(&open)?;
    let body_start = start_pos + open.len();
    
     // Find closing tag position relative to the slice starting at body_start
    let rel_close = wsdl.get(body_start..)?.find(&close)?;
    let end_pos = body_start + rel_close;

    Some(wsdl.get(body_start..end_pos)?.trim().to_string())
}

/// Find all occurrences of `<tagname>...</tagname>` content in the WSDL.
fn extract_tag_contents(wsdl: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");

    let mut result = Vec::new();
    let mut search_start = 0;

    while let Some(tag_start) = wsdl[search_start..].find(&open) {
        let abs_pos = tag_start + search_start;
        let body_start = abs_pos + open.len();

        if let Some(body_end) = wsdl.get(body_start..).and_then(|s| s.find(&close)) {
             // body_end is relative to the slice starting at body_start
            let abs_body_end = body_start + body_end;
             let content = &wsdl[body_start..abs_body_end];
            result.push(content.trim().to_string());

             // Move past this closing tag for next iteration
            search_start = abs_body_end + close.len();
         } else {
            break;      // No matching close tag found
         }
     }

    result
}

/// Extract an XML attribute value from a tag's content string.
/// Supports: attr="value" and attr='value' forms.
fn extract_attribute(tag_content: &str, attr_name: &str) -> Option<String> {
    let pattern = format!("{attr_name}=");
    let pos = tag_content.find(&pattern)?;
    let after_eq = &tag_content[pos + pattern.len()..];

    let result = if let Some(s) = after_eq.strip_prefix('"') {
        s.split('"').next().map(String::from)
    } else if let Some(s) = after_eq.strip_prefix('\'') {
        s.split('\'').next().map(String::from)
    } else {
        // Unquoted value — take until next whitespace or >
        after_eq.split_whitespace().next().map(String::from)
    };

    result
}
