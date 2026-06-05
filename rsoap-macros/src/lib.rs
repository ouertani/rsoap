//! A procedural macro crate for rsoap that parses WSDL files at compile-time
//! and generates typed SOAP client structs with serde rename attributes.

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{parse::ParseStream, parse_macro_input, DeriveInput, MetaNameValue, Token};

// ─────────── Data structures ───────────

/// A single struct field derived from WSDL / XSD.
#[derive(Clone)]
struct WsdlField {
    rust_name: String, // snake_case identifier
    xml_name: String,  /* original camelCase or XSD name for #[serde(rename)] */
    rust_type: String, // e.g. `String`, `i32`, `Vec<String>`
}

/// Parsed SOAP operation from WSDL.
#[allow(dead_code)]
struct WsdlOperation {
    name: String,
    action: String,
    endpoint: String,
    namespace: String,
    body_element: String,
    request_fields: Vec<WsdlField>,
    response_fields: Vec<WsdlField>,
}

/// Represents a WSDL message definition.
#[allow(dead_code)]
struct WsdlMessage {
    name: String,
    part_name: String,
    element_name: String,
}

/// Parse a WSSL message definition into a struct.
fn parse_message(content: &str) -> Option<WsdlMessage> {
    let name = extract_attribute(content, "name")?;

    let part = extract_tag_contents(content, "part").into_iter().next()?;
    let part_name = extract_attribute(&part.open, "name")?;
    let element_ref =
        extract_attribute(&part.open, "element").or_else(|| extract_attribute(&part.open, "type"));

    Some(WsdlMessage {
        name,
        part_name,
        element_name: element_ref.unwrap_or_default(),
    })
}

/// A parsed WSDL document with operations and an XSD element map.
#[allow(dead_code)]
struct ParsedWsdl {
    target_namespace: String,
    xsd_prefix: Option<String>,
    wsdl_ns_prefix: Option<String>,
    operations: Vec<WsdlOperation>,
    messages: Vec<WsdlMessage>,
    /// Maps XSD element name -> list of fields.
    elements: std::collections::HashMap<String, Vec<WsdlField>>,
}

// ─────────── XSD -> Rust type mapping ───────────

fn xsd_to_rust(typ: &str) -> &'static str {
    match typ.trim() {
        "xs:string" | "string" => "String",
        "xs:int" | "xs:integer" | "int" | "integer" => "i32",
        "xs:long" | "long" => "i64",
        "xs:float" | "xs:double" | "xs:decimal" | "float" | "double" | "decimal" => "f64",
        "xs:boolean" | "boolean" => "bool",
        "xs:date" | "xs:dateTime" | "date" | "dateTime" => "String",
        _ => "String", // fallback
    }
}

// ─────────── Namespace-prefix detection ───────────

fn detect_ns_prefix(wsdl: &str, uri: &str) -> Option<String> {
    wsdl.lines().find_map(|line| {
        let t = line.trim();
        if t.contains("xmlns:") && t.contains(uri) {
            let pos = t.find("xmlns:")?;
            let after = &t[pos + 6..];
            Some(after.split('=').next()?.trim().to_string())
        } else {
            None
        }
    })
}

// ─────────── XSD element map builder ───────────

/// Parse all `<xs:schema>` blocks in a WSDL and build an element -> fields map.
fn build_element_map(wsdl: &str) -> std::collections::HashMap<String, Vec<WsdlField>> {
    let mut map = std::collections::HashMap::new();

    // Search for <schema>, <xs:schema>, <xsd:schema>, etc.
    for schema_tag in &["schema", "xs:schema", "xsd:schema"] {
        for schema in extract_tag_contents(wsdl, schema_tag) {
            // Also search all element tag variants inside each schema.
            for elem in fetch_elems(&schema.body) {
                if let Some(elem_name) = extract_attribute(&elem.open, "name") {
                    // Try namespace-prefixed variants of complexType
                    let cplx_xml = ["complexType", "xs:complexType", "xsd:complexType"]
                        .into_iter()
                        .find_map(|t| extract_tag_content(&elem.body, t));
                    if let Some(cplx_xml) = cplx_xml {
                        let fields = parse_complex_type(cplx_xml.as_str());
                        map.insert(elem_name.clone(), fields);
                    } else {
                        // Simple element (no complex type -> just a string).
                        map.insert(
                            elem_name.clone(),
                            vec![WsdlField {
                                rust_name: camel_to_snake(&elem_name),
                                xml_name: elem_name,
                                rust_type: "String".into(),
                            }],
                        );
                    }
                }
            }
        }
    }

    map
}

/// Helper to extract all element blocks with any namespace prefix.
fn fetch_elems(content: &str) -> Vec<TagMatch> {
    let mut elems = Vec::new();
    for tag in &["element", "xs:element", "xsd:element"] {
        elems.extend(extract_tag_contents(content, tag));
    }
    elems
}

/// Extract fields from a complex type block.
fn parse_complex_type(content: &str) -> Vec<WsdlField> {
    let mut fields = Vec::new();

    // Search for sequence/complexType with any namespace prefix.
    for seq_tag in &["sequence", "xs:sequence", "xsd:sequence"] {
        for seq in extract_tag_contents(content, seq_tag) {
            // Search for element blocks with any prefix inside each sequence.
            for elem in fetch_elems(&seq.body) {
                if let Some(name) = extract_attribute(&elem.open, "name") {
                    let xsd_type_raw = extract_attribute(&elem.open, "type");
                    let xsd_type = xsd_type_raw.as_deref().unwrap_or("xs:string");

                    let max_occurs = extract_attribute(&elem.open, "maxOccurs");
                    let rust_ty = match max_occurs.as_deref() {
                        Some("unbounded") => format!("Vec<{}>", xsd_to_rust(xsd_type)),
                        _ => xsd_to_rust(xsd_type).into(),
                    };

                    fields.push(WsdlField {
                        rust_name: camel_to_snake(&name),
                        xml_name: name,
                        rust_type: rust_ty,
                    });
                }
            }
        }
    }

    fields
}

// ─────────── ParsedWsdl impl ───────────

impl ParsedWsdl {
    fn parse(wsdl: &str) -> Self {
        // targetNamespace is usually an attribute on <definitions>.
        let target_namespace = extract_attribute(wsdl, "targetNamespace").unwrap_or_default();

        let wsdl_ns_prefix = detect_ns_prefix(wsdl, "http://schemas.xmlsoap.org/wsdl/");
        let xsd_prefix = detect_ns_prefix(wsdl, "http://www.w3.org/2001/XMLSchema");

        // Build the XSD element map before parsing operations so it's ready.
        let elements = build_element_map(wsdl);

        // Extract <operation> blocks from portType and binding sections.
        let op_blocks = extract_tag_contents(wsdl, "operation");

        let mut operations = Vec::new();
        for op in &op_blocks {
            if let Ok(parsed) = parse_single_operation(op, wsdl, &target_namespace, &elements) {
                operations.push(parsed);
            }
        }

        let messages: Vec<WsdlMessage> = extract_tag_contents(wsdl, "message")
            .iter()
            .filter_map(|msg| parse_message(&msg.open))
            .collect();

        Self {
            target_namespace,
            xsd_prefix,
            wsdl_ns_prefix,
            operations,
            messages,
            elements,
        }
    }
}

// ─────────── Single operation parsing ───────────

fn parse_single_operation(
    op: &TagMatch, // raw XML of one <operation>...</operation>
    wsdl: &str,    // entire WSDL (for soap:address / global messages)
    namespace: &str,
    elements: &std::collections::HashMap<String, Vec<WsdlField>>,
) -> Result<WsdlOperation, String> {
    let name = extract_attribute(&op.open, "name")
        .ok_or_else(|| "Could not find operation name".to_string())?;

    // soap:action from the input block, or wsdlsoap:operation on the binding.
    let inputs = extract_tag_contents(&op.body, "input");
    let action = inputs
        .first()
        .and_then(|i| extract_attribute(&i.open, "soap:action"))
        .or_else(|| {
            // wsdlsoap:operation tag may carry soapAction
            extract_tag_contents(&op.body, "operation")
                .first()
                .and_then(|o| extract_attribute(&o.open, "soapAction"))
        })
        .unwrap_or_else(|| format!("{namespace}/{name}"));

    // Endpoint from soap:address on binding or service; try namespace-prefixed variants.
    let address = extract_tag_contents(wsdl, "address")
        .into_iter()
        .chain(extract_tag_contents(wsdl, "wsdlsoap:address"))
        .chain(extract_tag_contents(wsdl, " soap:address"))
        .next()
        .map(|s| extract_attribute(&s.open, "location").unwrap_or_default())
        .unwrap_or_default();

    // Resolve request/response fields via XSD element map.
    let (request_fields, response_fields) = resolution::parse_input_output(&inputs, wsdl, elements);

    Ok(WsdlOperation {
        name: name.clone(),
        action,
        endpoint: address,
        namespace: namespace.to_string(),
        body_element: format!("{name}Request"),
        request_fields,
        response_fields,
    })
}

/// Module-hoisted parsing logic that operates on the element map.
mod resolution {
    use super::*;

    /// Parse input/output operation blocks into (request_fields, response_fields).
    pub fn parse_input_output(
        inputs: &[TagMatch],
        wsdl: &str,
        elements: &std::collections::HashMap<String, Vec<WsdlField>>,
    ) -> (Vec<WsdlField>, Vec<WsdlField>) {
        let mut request_fields = Vec::new();
        let mut response_fields = Vec::new();

        // Resolve requests from input blocks.
        for input in inputs {
            if let Some(msg_ref) = extract_attribute(&input.open, "message") {
                let msg_name = msg_ref
                    .split(':')
                    .next_back()
                    .unwrap_or(&msg_ref)
                    .to_string();
                resolve_message_fields(&msg_name, true, elements, &mut request_fields);
            }
        }

        // Collect output blocks from the whole WSDL (message names are unique in normal WSDLs).
        let output_blocks = extract_tag_contents(wsdl, "output");
        for output in &output_blocks {
            if let Some(msg_ref) = extract_attribute(&output.open, "message") {
                let msg_name = msg_ref
                    .split(':')
                    .next_back()
                    .unwrap_or(&msg_ref)
                    .to_string();
                resolve_message_fields(&msg_name, false, elements, &mut response_fields);
            }
        }

        (request_fields, response_fields)
    }

    fn resolve_message_fields(
        msg_name: &str,
        _is_request: bool,
        elements: &std::collections::HashMap<String, Vec<WsdlField>>,
        fields_out: &mut Vec<WsdlField>,
    ) {
        // Try to find a matching element in the XSD map.
        let found = elements.iter().find(|(elem_name, _)| {
            *elem_name == msg_name
                || elem_name.contains(msg_name)
                || msg_name.contains(elem_name.split('R').next_back().unwrap_or(""))
        });

        match found.map(|(_, v)| v.clone()) {
            Some(fields) => {
                // Insert only new fields to avoid duplicates.
                for f in fields {
                    if !fields_out
                        .iter()
                        .any(|existing| existing.rust_name == f.rust_name)
                    {
                        fields_out.push(f);
                    }
                }
            }
            None => {
                // Fallback: derive one field from the message name itself.
                if fields_out.is_empty() {
                    let rust_name = camel_to_snake(msg_name);
                    fields_out.push(WsdlField {
                        rust_name,
                        xml_name: msg_name.to_string(),
                        rust_type: "String".to_string(),
                    });
                }
            }
        }
    }
}

// ─────────── Code generation ───────────

fn generate_from_wsdl(op: &WsdlOperation, struct_name: &syn::Ident) -> TokenStream2 {
    let mod_name = format_ident!("{}", op.name.to_lowercase());
    let req_struct = format_ident!("{}Request", op.name);
    let resp_struct = format_ident!("{}Response", op.name);
    let body_element = &op.body_element;

    // Build request field tokens with #[serde(rename)] for correct XML element names.
    let request_fields: Vec<TokenStream2> = op
        .request_fields
        .iter()
        .map(|f| {
            let ident = format_ident!("{}", f.rust_name);
            let rust_ty = &f.rust_type;
            let rename = &f.xml_name;
            quote! { #[serde(rename = #rename)] pub #ident: #rust_ty }
        })
        .collect();

    // Build response field tokens.
    let response_fields: Vec<TokenStream2> = op
        .response_fields
        .iter()
        .map(|f| {
            let ident = format_ident!("{}", f.rust_name);
            let rust_ty = &f.rust_type;
            let rename = &f.xml_name;
            quote! { #[serde(rename = #rename)] pub #ident: #rust_ty }
        })
        .collect();

    let action = &op.action;
    let endpoint = &op.endpoint;

    quote! {
        /// Generated request and response types for the #action operation.
        #[allow(non_camel_case_types)]
        pub mod #mod_name {
            use ::rsoap::serde::{Serialize, Deserialize};

            /// Request struct derived from WSDL / XSD.
            #[derive(Debug, Clone, Serialize, Deserialize)]
            pub struct #req_struct {
                #(#request_fields),*
            }

            /// Response struct derived from WSDL / XSD.
            #[derive(Debug, Clone, Default, Serialize, Deserialize)]
            pub struct #resp_struct {
                #(#response_fields),*
            }
        }

        impl ::rsoap::SoapOperation for #struct_name {
            type Request    = #mod_name::#req_struct;
            type Response   = #mod_name::#resp_struct;

            const ACTION:         &'static str = #action;
            const ENDPOINT:       &'static str = #endpoint;
            const BODY_ELEMENT:   &'static str = #body_element;
        }
    }
}

/// Generate placeholder when no WSDL file is provided — user must define fields manually.
fn generate_placeholder(struct_name: &syn::Ident, operation_name: &str) -> TokenStream2 {
    let mod_name = format_ident!("{}", operation_name.to_lowercase());
    let req_struct = format_ident!("{operation_name}Request");
    let resp_struct = format_ident!("{operation_name}Response");

    quote! {
        #[allow(non_camel_case_types)]
        pub mod #mod_name {
            use ::rsoap::serde::{Serialize, Deserialize};

            #[derive(Debug, Clone, Default, Serialize, Deserialize)]
            pub struct #req_struct;

            #[derive(Debug, Clone, Default, Serialize, Deserialize)]
            pub struct #resp_struct;
        }

        impl ::rsoap::SoapOperation for #struct_name {
            type Request    = #mod_name::#req_struct;
            type Response   = #mod_name::#resp_struct;

            const ACTION:         &'static str = "";
            const ENDPOINT:       &'static str = "";
            const BODY_ELEMENT:   &'static str = "";
        }
    }
}

// ─────────── Proc-macro entry point ───────────

/// Derive macro that parses a WSDL at compile-time and generates typed request/response structs.
#[proc_macro_derive(SoapOperation, attributes(soap))]
pub fn soap_operation(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    #[allow(clippy::cmp_owned)]
    let soap_attr = input
        .attrs
        .iter()
        .find(|a| {
            a.path()
                .get_ident()
                .map(|p| p.to_string() == "soap")
                .unwrap_or(false)
        })
        .expect("`#[soap(wsdl = \"...\", operation_name = \"...\")]` attribute is required");

    let attrs = extract_soap_meta(soap_attr);
    let wsdl_path = attrs.get("wsdl").cloned();
    let operation_name = attrs.get("operation_name").cloned().unwrap_or_default();

    let wsdl_str = match &wsdl_path {
        Some(path) => std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("Failed to read WSDL file '{path}': {e}")),
        None => String::new(),
    };

    let generated = if wsdl_str.is_empty() {
        generate_placeholder(struct_name, &operation_name)
    } else {
        let parsed = ParsedWsdl::parse(&wsdl_str);

        match parsed
            .operations
            .iter()
            .find(|op| op.name == operation_name)
        {
            Some(op) => generate_from_wsdl(op, struct_name),
            None => panic!(
                "operation '{}' not found in WSDL (available: {})",
                operation_name,
                parsed
                    .operations
                    .iter()
                    .map(|op| op.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    };

    TokenStream::from(quote! { #generated })
}

// ─────────── Helper functions ───────────

fn extract_soap_meta(attr: &syn::Attribute) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;

    let mut attrs = HashMap::new();

    if let syn::Meta::List(list) = &attr.meta {
        let parser = |content: ParseStream| -> syn::Result<Vec<MetaNameValue>> {
            let pairs: Punctuated<MetaNameValue, Token![,]> =
                Punctuated::parse_terminated(content)?;
            Ok(pairs.into_iter().collect())
        };

        if let Ok(nvs) = list.parse_args_with(parser) {
            for nv in &nvs {
                let key = nv
                    .path
                    .get_ident()
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

/// A single match of `<tag ...>body</tag>` or self-closing `<tag .../>`.
/// `open` holds the opening-tag text (including the trailing `>` for
/// self-closing, or the full opening line up to the first `>` for
/// non-self-closing) so attributes can be extracted.  `body` holds the
/// inner text (empty for self-closing tags).
struct TagMatch {
    open: String,
    body: String,
}

/// Extract inner text (body) of a single tag occurrence.  Handles
/// namespaced tags (`<ns:tag>`), tags with attributes (`<tag attr="...">`)
/// and self-closing tags (`<tag .../>`, body is empty).
fn extract_tag_content(wsdl: &str, tag: &str) -> Option<String> {
    extract_tag_contents(wsdl, tag)
        .into_iter()
        .next()
        .map(|m| m.body)
}

/// Find all occurrences of `<tag ...>...</tag>` or self-closing `<tag .../>`
/// inside **wsdl**.  Handles namespace prefixes (`xs:tag`, `xsd:tag`).
fn extract_tag_contents(wsdl: &str, tag: &str) -> Vec<TagMatch> {
    let open_pat = format!("<{}", tag);
    let close_tag = format!("</{}>", tag);
    let mut result = Vec::new();
    let mut search_start = 0usize;

    while let Some(rel) = wsdl[search_start..].find(&open_pat) {
        let tag_start = rel + search_start;
        let after_tag = &wsdl[tag_start + open_pat.len()..];

        // Find the first `>` after the tag name.  If the char before it is
        // `/`, the tag is self-closing.  We must use the first `>` (not
        // `find("/>")`) so that `/>` in child elements doesn't fool us.
        let gt_pos = match after_tag.find('>') {
            Some(p) => p,
            None => break,
        };
        let is_self_closing = gt_pos > 0 && after_tag.as_bytes()[gt_pos - 1] == b'/';
        let body_end_abs = tag_start + open_pat.len() + gt_pos + 1;

        let open_text = wsdl[tag_start..body_end_abs].trim().to_string();

        if is_self_closing {
            result.push(TagMatch {
                open: open_text,
                body: String::new(),
            });
            search_start = body_end_abs;
            continue;
        }

        // Extract the body between open and close tags
        if let Some(body_end_rel) = wsdl[body_end_abs..].find(&close_tag) {
            let abs_body_end = body_end_abs + body_end_rel;
            let inner = wsdl[body_end_abs..abs_body_end].trim().to_string();
            result.push(TagMatch {
                open: open_text,
                body: inner,
            });
            search_start = abs_body_end + close_tag.len();
        } else {
            break;
        }
    }

    result
}

fn extract_attribute(tag_content: &str, attr_name: &str) -> Option<String> {
    let pattern = format!("{attr_name}=");
    let pos = tag_content.find(&pattern)?;
    let after_eq = &tag_content[pos + pattern.len()..];

    if let Some(s) = after_eq.strip_prefix('"') {
        s.split('"').next().map(String::from)
    } else if let Some(s) = after_eq.strip_prefix('\'') {
        s.split('\'').next().map(String::from)
    } else {
        after_eq.split_whitespace().next().map(String::from)
    }
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

// ─────────── Unit tests ───────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsd_to_rust_maps_correctly() {
        assert_eq!(xsd_to_rust("xs:string"), "String");
        assert_eq!(xsd_to_rust("int"), "i32");
        assert_eq!(xsd_to_rust("xs:decimal"), "f64");
        assert_eq!(xsd_to_rust("xs:boolean"), "bool");
        assert_eq!(xsd_to_rust("unknown"), "String"); // fallback
    }

    #[test]
    fn extract_tag_content_works() {
        let xml = r#"<foo>hello</foo>"#;
        assert_eq!(extract_tag_content(xml, "foo").as_deref(), Some("hello"));
    }

    #[test]
    fn extract_tag_contents_finds_multiple() {
        let xml = "<a>x</a><b>y</b><a>z</a>";
        let r = extract_tag_contents(xml, "a");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].body, "x");
        assert_eq!(r[1].body, "z");
    }

    #[test]
    fn element_map_picks_up_unbounded() {
        let wsdl = r#"<schema>
            <xs:schema targetNamespace="http://ex.com"
                       xmlns:xs="http://www.w3.org/2001/XMLSchema">
                <xs:element name="Forecast">
                    <xs:complexType>
                        <xs:sequence>
                            <xs:element name="days" type="xs:int"/>
                            <xs:element name="summary" type="xs:string" maxOccurs="unbounded"/>
                        </xs:sequence>
                    </xs:complexType>
                </xs:element>
            </xs:schema>
        </schema>"#;
        let map = build_element_map(wsdl);
        let forecast = map.get("Forecast").unwrap();
        assert_eq!(forecast.len(), 2);
        assert_eq!(forecast[1].rust_type, "Vec<String>"); // maxOccurs=unbounded -> Vec<T>
    }

    #[test]
    fn detect_ns_prefix_works() {
        let wsdl = r#"<wsdl defs xmlns:ns0="http://ex.com/foo">"#;
        assert_eq!(
            detect_ns_prefix(wsdl, "http://ex.com/foo"),
            Some("ns0".into())
        );
        assert_eq!(detect_ns_prefix(wsdl, "http://nope.com/"), None);
    }

    #[test]
    fn extract_message_from_element_ref() {
        let element_map = r#"<message name="MyMsg"><part name="p" element="ns:MyElem"/></message>"#;
        assert_eq!(extract_attribute(element_map, "name"), Some("MyMsg".into()));
        assert_eq!(
            extract_attribute(element_map, "element"),
            Some("ns:MyElem".into())
        );
    }

    #[test]
    fn extract_tag_content_handles_nested_tags() {
        let xml = r#"<outer><inner>hello</inner></outer>"#;
        let inner_content = extract_tag_content(xml, "inner");
        assert_eq!(inner_content.as_deref(), Some("hello"));

        // Verify outer contains inner content.
        let outer_content = extract_tag_content(xml, "outer");
        assert!(outer_content.unwrap().contains("inner"));
    }

    #[test]
    fn full_wsdl_parsing_with_type_resolution() {
        let wsdl = r#"<?xml version="1.0"?>
<definitions xmlns="http://schemas.xmlsoap.org/wsdl/"
             xmlns:ns="http://www.w3.org/2001/XMLSchema"
             targetNamespace="http://example.com/weather">
  <types>
    <xs:schema targetNamespace="http://example.com/weather"
               xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:element name="GetTemperatureRequest">
        <xs:complexType>
          <xs:sequence>
            <xs:element name="zip" type="xs:string"/>
          </xs:sequence>
        </xs:complexType>
      </xs:element>
    </xs:schema>
  </types>
  <portType name="WeatherPortType">
    <operation name="GetTemperature">
      <input message="ns:GetTempReq"/><output message="ns:GetTempRsp"/>
    </operation>
  </portType>
  <binding name="WeatherBinding" type="ns:WeatherPortType">
    <wsdlsoap:binding style="document" transport="http://schemas.xmlsoap.org/soap/http"/>
    <operation name="GetTemperature">
      <wsdlsoap:operation soapAction="http://example.com/weather/GetTemperature"/>
      <input><wsdlsoap:body use="literal"/></input>
      <output><wsdlsoap:body use="literal"/></output>
    </operation>
  </binding>
  <service name="WeatherService">
    <port name="WeatherPort" binding="ns:WeatherBinding">
      <wsdlsoap:address location="http://localhost:8080/weather"/>
    </port>
  </service>
</definitions>"#;

        let parsed = ParsedWsdl::parse(wsdl);
        assert_eq!(parsed.target_namespace, "http://example.com/weather");
        assert!(!parsed.operations.is_empty());

        let op = &parsed.operations[0];
        assert_eq!(op.name, "GetTemperature");
        assert_eq!(op.action, "http://example.com/weather/GetTemperature");
        assert_eq!(op.endpoint, "http://localhost:8080/weather");

        // XSD type resolution should have found at least one request field.
        assert!(
            !op.request_fields.is_empty(),
            "should resolve at least one request field"
        );
    }
}
