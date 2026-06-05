//! SOAP envelope construction and parsing utilities.

use quick_xml::de::from_str;
use serde::{de::DeserializeOwned, Serialize};

/// The SOAP 1.1 Envelope namespace URI.
pub const SOAP_NS_ENVELOPE: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// The SOAP 1.2 Envelope namespace URI.
pub const SOAP_NS_ENVELOPE_12: &str = "http://www.w3.org/2003/05/soap-envelope";

const BODY_OPEN: &str = "<Body>";
const BODY_CLOSE: &str = "</Body>";

/// The SOAP protocol version an operation speaks.
///
/// Defaults to [`SoapVersion::V11`] in the [`crate::SoapOperation`] trait
/// for backward compatibility. Set explicitly on a hand-written operation
/// or let the `#[derive(SoapOperation)]` macro detect it from the WSDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoapVersion {
    /// SOAP 1.1 — `text/xml` Content-Type, `SOAPAction` header, `soap:Body`/`soap:Fault`.
    #[default]
    V11,
    /// SOAP 1.2 — `application/soap+xml` Content-Type with `action` parameter,
    /// `env:Body`/`env:Fault` with the 1.2 fault structure.
    V12,
}

impl SoapVersion {
    /// The XML namespace URI for this version's envelope.
    pub fn namespace(self) -> &'static str {
        match self {
            Self::V11 => SOAP_NS_ENVELOPE,
            Self::V12 => SOAP_NS_ENVELOPE_12,
        }
    }

    /// The prefix used in serialized XML for this version's envelope tags.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::V11 => "soap",
            Self::V12 => "env",
        }
    }
}

/// Helper to serialize a request type to XML body string with a specific root element name.
///
/// # Errors
/// Returns [`quick_xml::se::SeError`] if the payload cannot be serialized.
pub fn serialize_request<T: Serialize>(
    body_name: &str,
    payload: &T,
) -> Result<String, quick_xml::se::SeError> {
    quick_xml::se::to_string_with_root(body_name, payload)
}

/// Extract the inner XML content from inside `<soap:Body>` (or `<env:Body>`) tags.
///
/// Tries the version-specific prefixes (`soap:`, `env:`), then falls back to
/// the unprefixed `<Body>`. Generic `<*:Body>` prefixes are not matched —
/// WSDLs in practice use only `soap:` (1.1) or `env:` (1.2).
///
/// # Errors
/// Returns [`quick_xml::de::DeError`] if no Body tag is found.
pub fn extract_body(xml: &str) -> Result<String, quick_xml::de::DeError> {
    for prefix in &["soap", "env"] {
        let open = format!("<{}:Body>", prefix);
        let close = format!("</{}:Body>", prefix);
        if let Some(body) = extract_tagged_body(xml, &open, &close) {
            return Ok(body);
        }
    }
    if let Some(body) = extract_tagged_body(xml, BODY_OPEN, BODY_CLOSE) {
        return Ok(body);
    }

    Err(quick_xml::de::DeError::Custom(
        "could not find Body tag in SOAP envelope".into(),
    ))
}

/// Extract content between `open_tag` and `close_tag` in `xml`, if both are present
/// and `open_tag` occurs before `close_tag`.
fn extract_tagged_body(xml: &str, open_tag: &str, close_tag: &str) -> Option<String> {
    let start = xml.find(open_tag)?;
    let body_start = start + open_tag.len();
    if body_start >= xml.len() {
        return None;
    }
    let end = xml[body_start..].find(close_tag)?;
    Some(xml[body_start..body_start + end].trim().to_string())
}

/// Deserialize a SOAP response string into a typed struct.
/// Automatically extracts the content from inside `<soap:Body>` before deserializing.
///
/// # Errors
/// Returns [`quick_xml::de::DeError`] if the body cannot be extracted or the
/// body content cannot be deserialized into `T`.
pub fn deserialize_response<T: DeserializeOwned>(xml: &str) -> Result<T, quick_xml::de::DeError> {
    let body_content = extract_body(xml)?;
    from_str(&body_content)
}

/// Serialize a SOAP fault into an XML string for manual transport.
///
/// Uses the SOAP 1.1 fault structure (`<faultcode>` / `<faultstring>`) for
/// [`SoapVersion::V11`] and the SOAP 1.2 structure (`<Code><Value>` / `<Reason><Text>`)
/// for [`SoapVersion::V12`].
pub fn serialize_fault(version: SoapVersion, code: &str, message: &str) -> String {
    let prefix = version.prefix();
    let ns = version.namespace();
    match version {
        SoapVersion::V11 => format!(
            r#"<{prefix}:Fault xmlns:{prefix}="{ns}"><faultcode>{code}</faultcode><faultstring>{msg}</faultstring></{prefix}:Fault>"#,
            prefix = prefix,
            ns = ns,
            code = code,
            msg = message
        ),
        SoapVersion::V12 => format!(
            r#"<{prefix}:Fault xmlns:{prefix}="{ns}"><Code><Value>{code}</Value></Code><Reason><Text xml:lang="en">{msg}</Text></Reason></{prefix}:Fault>"#,
            prefix = prefix,
            ns = ns,
            code = code,
            msg = message
        ),
    }
}

/// Deserialize a SOAP fault from XML content.
/// Accepts either raw `<soap:Fault>` / `<env:Fault>` XML or a full envelope
/// wrapping a fault. Returns `Ok((code, message))` if valid, or an error
/// if parsing fails.
///
/// Detects the version automatically by looking at the fault element's
/// namespace declaration. Falls back to SOAP 1.1 parsing if the version
/// cannot be determined.
///
/// # Errors
/// Returns [`quick_xml::de::DeError`] if the fault XML cannot be deserialized.
pub fn parse_soap_fault(xml: &str) -> Result<(String, String), quick_xml::de::DeError> {
    let version = detect_fault_version(xml);
    let payload = extract_body(xml).unwrap_or_else(|_| xml.to_string());

    match version {
        SoapVersion::V11 => parse_fault_v11(&payload),
        SoapVersion::V12 => parse_fault_v12(&payload),
    }
}

/// Detect whether `xml` contains a SOAP 1.1 or 1.2 fault by inspecting the
/// `xmlns` declarations on the Fault element. Defaults to 1.1.
fn detect_fault_version(xml: &str) -> SoapVersion {
    if xml.contains(SOAP_NS_ENVELOPE_12) {
        SoapVersion::V12
    } else {
        SoapVersion::V11
    }
}

fn parse_fault_v11(payload: &str) -> Result<(String, String), quick_xml::de::DeError> {
    #[derive(Debug, serde::Deserialize)]
    struct Fault {
        faultcode: Option<String>,
        faultstring: Option<FaultString>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct FaultString {
        #[serde(rename = "$text")]
        value: String,
    }

    let fault: Fault = from_str(payload)?;
    Ok((
        fault.faultcode.unwrap_or_else(|| "unknown".to_string()),
        fault
            .faultstring
            .map(|s| s.value)
            .unwrap_or_else(|| "no details".to_string()),
    ))
}

fn parse_fault_v12(payload: &str) -> Result<(String, String), quick_xml::de::DeError> {
    #[derive(Debug, serde::Deserialize)]
    struct Fault {
        #[serde(rename = "Code")]
        code: Option<Code>,
        #[serde(rename = "Reason")]
        reason: Option<Reason>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Code {
        #[serde(rename = "Value")]
        value: Option<String>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Reason {
        #[serde(rename = "Text")]
        text: Option<FaultString>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct FaultString {
        #[serde(rename = "$text")]
        value: String,
    }

    let fault: Fault = from_str(payload)?;
    Ok((
        fault
            .code
            .and_then(|c| c.value)
            .unwrap_or_else(|| "unknown".to_string()),
        fault
            .reason
            .and_then(|r| r.text)
            .map(|t| t.value)
            .unwrap_or_else(|| "no details".to_string()),
    ))
}

/// Quick check for whether an XML payload appears to be a SOAP fault.
/// Returns `true` if the payload contains a `<soap:Fault>`, `<env:Fault>`,
/// or unprefixed `<Fault xmlns=...>` element.
pub fn is_soap_fault(xml: &str) -> bool {
    xml.contains("<soap:Fault") || xml.contains("<env:Fault") || xml.contains("<Fault xmlns=")
}

/// Build a SOAP envelope XML string wrapping `action` and `body_xml` for the
/// given protocol version.
///
/// - SOAP 1.1 uses the `soap:` prefix, the legacy namespace, and embeds
///   the action in a WS-Addressing `<soap:Header><Action>` element.
/// - SOAP 1.2 uses the `env:` prefix, the 1.2 namespace, and puts the
///   action in the HTTP `Content-Type` header (caller's responsibility).
pub fn build_envelope(version: SoapVersion, action: &str, body_xml: &str) -> String {
    let prefix = version.prefix();
    let ns = version.namespace();
    match version {
        SoapVersion::V11 => format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<{prefix}:Envelope xmlns:{prefix}="{ns}">
    <{prefix}:Header>
       <Action soap:mustUnderstand="true" xmlns="http://schemas.xmlsoap.org/ws/2004/08/addressing">{action}</Action>
    </{prefix}:Header>
    <{prefix}:Body>
       {body}
    </{prefix}:Body>
</{prefix}:Envelope>"#,
            prefix = prefix,
            ns = ns,
            action = action,
            body = body_xml,
        ),
        SoapVersion::V12 => format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<{prefix}:Envelope xmlns:{prefix}="{ns}">
    <{prefix}:Header/>
    <{prefix}:Body>
       {body}
    </{prefix}:Body>
</{prefix}:Envelope>"#,
            prefix = prefix,
            ns = ns,
            body = body_xml,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_body_from_soap_envelope() {
        let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
       <soap:Body>
         <GetWeatherResponse>
           <temperature>72</temperature>
         </GetWeatherResponse>
       </soap:Body>
     </soap:Envelope>"#;

        let body = extract_body(xml).unwrap();
        assert!(body.contains("GetWeatherResponse"));
        assert!(body.contains("72"));
    }

    #[test]
    fn test_deserialize_basic() {
        #[derive(Debug, serde::Deserialize)]
        struct TestResponse {
            result: i32,
        }

        let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                <soap:Body>
                    <TestResponse><result>100</result>
                </TestResponse>
              </soap:Body>
          </soap:Envelope>"#;
        let result: TestResponse = deserialize_response(xml).unwrap();
        assert_eq!(result.result, 100);
    }

    #[test]
    fn test_parse_soap_fault() {
        let xml = r#"<soap:Fault xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <faultcode>Client</faultcode>
             <faultstring>Invalid credentials</faultstring>
          </soap:Fault>"#;

        let (code, message) = parse_soap_fault(xml).unwrap();
        assert_eq!(code, "Client");
        assert_eq!(message, "Invalid credentials");
    }

    #[test]
    fn test_serialize_fault_11() {
        let xml = serialize_fault(SoapVersion::V11, "ServerFault", "Something went wrong");
        assert!(xml.contains("<faultcode>ServerFault</faultcode>"));
        assert!(xml.contains("<faultstring>Something went wrong</faultstring>"));
        assert!(xml.contains("xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\""));
    }

    #[test]
    fn test_serialize_fault_12() {
        let xml = serialize_fault(SoapVersion::V12, "soap:Sender", "Bad input");
        assert!(xml.contains("<Value>soap:Sender</Value>"));
        assert!(xml.contains("<Text xml:lang=\"en\">Bad input</Text>"));
        assert!(xml.contains("xmlns:env=\"http://www.w3.org/2003/05/soap-envelope\""));
    }

    #[test]
    fn test_parse_soap_fault_12() {
        let xml = r#"<env:Fault xmlns:env="http://www.w3.org/2003/05/soap-envelope">
            <Code><Value>env:Sender</Value></Code>
            <Reason><Text xml:lang="en">Malformed request</Text></Reason>
        </env:Fault>"#;

        let (code, message) = parse_soap_fault(xml).unwrap();
        assert_eq!(code, "env:Sender");
        assert_eq!(message, "Malformed request");
    }

    #[test]
    fn test_build_envelope_11() {
        let xml = build_envelope(
            SoapVersion::V11,
            "GetTemperature",
            "<req:GetTemperature><lat>40</lat></req:GetTemperature>",
        );
        assert!(xml.contains("<soap:Envelope"));
        assert!(xml.contains("<soap:Header>"));
        assert!(xml.contains("<Action"));
        assert!(xml.contains(">GetTemperature</Action>"));
        assert!(xml.contains("<soap:Body"));
        assert!(xml.contains("<req:GetTemperature>"));
    }

    #[test]
    fn test_build_envelope_12() {
        let xml = build_envelope(
            SoapVersion::V12,
            "GetTemperature",
            "<req:GetTemperature><lat>40</lat></req:GetTemperature>",
        );
        assert!(xml.contains("<env:Envelope"));
        assert!(xml.contains("xmlns:env=\"http://www.w3.org/2003/05/soap-envelope\""));
        assert!(xml.contains("<env:Header"));
        assert!(xml.contains("<env:Body"));
        assert!(xml.contains("<req:GetTemperature>"));
        // 1.2 puts action in HTTP header, not in envelope.
        assert!(!xml.contains("<Action"));
    }

    #[test]
    fn test_is_soap_fault_detects_both_versions() {
        assert!(is_soap_fault("<soap:Fault>code</soap:Fault>"));
        assert!(is_soap_fault("<env:Fault>code</env:Fault>"));
        assert!(is_soap_fault("<Fault xmlns=\"...\">msg</Fault>"));
        assert!(!is_soap_fault(
            "<GetTempResponse><temp>72</temp></GetTempResponse>"
        ));
    }

    #[test]
    fn test_extract_body_fallback() {
        let xml = r#"<Envelope><Body><Resp/></Body></Envelope>"#;
        let body = extract_body(xml).unwrap();
        assert_eq!(body, "<Resp/>");
    }

    #[test]
    fn test_soap_version_default_is_v11() {
        assert_eq!(SoapVersion::default(), SoapVersion::V11);
    }
}
