//! SOAP envelope construction and parsing utilities.

use quick_xml::de::from_str;
use serde::{de::DeserializeOwned, Serialize};

/// The SOAP 1.1 Envelope namespace URI.
pub const SOAP_NS_ENVELOPE: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// Helper to serialize a request type to XML body string with a specific root element name.
pub fn serialize_request<T: Serialize>(body_name: &str, payload: &T) -> Result<String, quick_xml::se::SeError> {
    quick_xml::se::to_string_with_root(body_name, payload)
}

/// Extract the inner XML content from inside `<soap:Body>` tags.
pub fn extract_body(xml: &str) -> Result<String, quick_xml::de::DeError> {
       // Try standard SOAP namespace first
    if let Some(start) = xml.find("<soap:Body>") {
         if start + 11 < xml.len() {
             let body_start = start + 11;     // length of "<soap:Body>"
            if let Some(end) = xml[body_start..].find("</soap:Body>") {
                 return Ok(xml[body_start..body_start + end].trim().to_string());
                }
           }
       }

       // Fallback: try raw `<Body>` without namespace prefix
    if let Some(start) = xml.find("<Body>") {
         if start + 6 < xml.len() {
             let body_start = start + 6;    // length of "<Body>"
            if let Some(end) = xml[body_start..].find("</Body>") {
                 return Ok(xml[body_start..body_start + end].trim().to_string());
               }
           }
       }

    Err(quick_xml::de::DeError::Custom(
         "could not find Body tag in SOAP envelope".into(),
        ))
}

/// Deserialize a SOAP response string into a typed struct.
/// Automatically extracts the content from inside `<soap:Body>` before deserializing.
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
pub fn parse_soap_fault(xml: &str) -> Result<(String, String), quick_xml::de::DeError> {
    let payload = extract_body(xml).unwrap_or_else(|_| xml.to_string());

      #[derive(Debug, serde::Deserialize)]
      struct Fault { faultcode: Option<String>, faultstring: Option<FaultString> }

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
             struct TestResponse { result: i32 }

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
