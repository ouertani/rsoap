//! SOAP envelope construction and parsing utilities.

use quick_xml::de::from_str;
use serde::{de::DeserializeOwned, Serialize};

/// The SOAP 1.1 Envelope namespace URI.
pub const SOAP_NS_ENVELOPE: &str = "http://schemas.xmlsoap.org/soap/envelope/";

const SOAP_BODY_OPEN: &str = "<soap:Body>";
const SOAP_BODY_CLOSE: &str = "</soap:Body>";
const BODY_OPEN: &str = "<Body>";
const BODY_CLOSE: &str = "</Body>";

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

/// Extract the inner XML content from inside `<soap:Body>` tags.
///
/// Tries `<soap:Body>` first, then falls back to the unprefixed `<Body>`.
///
/// # Errors
/// Returns [`quick_xml::de::DeError`] if neither `<soap:Body>` nor `<Body>` tags are found.
pub fn extract_body(xml: &str) -> Result<String, quick_xml::de::DeError> {
    if let Some(body) = extract_tagged_body(xml, SOAP_BODY_OPEN, SOAP_BODY_CLOSE) {
        return Ok(body);
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
pub fn serialize_fault(code: &str, message: &str) -> String {
    format!(
        r#"<soap:Fault xmlns:soap="{ns}"><faultcode>{code}</faultcode><faultstring>{msg}</faultstring></soap:Fault>"#,
        ns = SOAP_NS_ENVELOPE,
        code = code,
        msg = message
    )
}

/// Deserialize a SOAP fault from XML content.
/// Accepts either raw `<soap:Fault>` XML or a full `<soap:Envelope>` wrapping it.
/// Returns `Ok((code, message))` if valid, or an error if parsing fails.
///
/// # Errors
/// Returns [`quick_xml::de::DeError`] if the fault XML cannot be deserialized.
pub fn parse_soap_fault(xml: &str) -> Result<(String, String), quick_xml::de::DeError> {
    let payload = extract_body(xml).unwrap_or_else(|_| xml.to_string());

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

    let fault: Fault = from_str(&payload)?;

    Ok((
        fault.faultcode.unwrap_or_else(|| "unknown".to_string()),
        fault
            .faultstring
            .map(|s| s.value)
            .unwrap_or_else(|| "no details".to_string()),
    ))
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
    fn test_serialize_fault() {
        let xml = serialize_fault("ServerFault", "Something went wrong");
        assert!(xml.contains("<faultcode>ServerFault</faultcode>"));
        assert!(xml.contains("<faultstring>Something went wrong</faultstring>"));
    }

    #[test]
    fn test_extract_body_fallback() {
        let xml = r#"<Envelope><Body><Resp/></Body></Envelope>"#;
        let body = extract_body(xml).unwrap();
        assert_eq!(body, "<Resp/>");
    }
}
