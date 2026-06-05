//! Example: building a typed SOAP client with rsoap from a WSDL file.
//!
//! This example demonstrates:
//! - Deriving `SoapOperation` from a WSDL at compile time (using generated request/response types)
//! - Creating a configured [`SoapClient`] with custom headers
//! - Calling operations and handling the deserialized response

use rsoap::{SoapClient, SoapOperation};
// In production, these are auto-generated from the WSDL by #[derive(SoapOperation)].
#[derive(Debug, serde::Serialize)]
pub struct GetTemperatureRequest {
    zipcode: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct GetTemperatureResponse {
    temperature: Option<f64>,
    unit: Option<String>,
}

// The SoapOperation trait is implemented by the derive macro.
// In production, you'd write:
//   #[derive(rsoap::SoapOperation)]
//   #[soap(wsdl = "examples/wsdl/weather.wsdl", operation_name = "GetTemperature")]
//   pub struct GetWeather;
#[allow(dead_code)]
pub struct GetWeather;

/// Demonstrates the ideal API once WSDL parsing is fully implemented.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a client with custom headers (e.g., for auth or tracing).
    let client =
        SoapClient::new("http://localhost:8080/weather")?.with_header("X-Api-Key", "test-key");
    println!("Soap client configured for: {}", client.endpoint());

    // Build the typed request.
    let request = GetTemperatureRequest {
        zipcode: "90210".to_string(),
    };

    // Serialize the request body to XML using quick_xml directly.
    let _xml_body =
        rsoap::quick_xml::se::to_string_with_root("ns:GetTemperatureRequest", &request)?;

    // Show what a mock response envelope looks like.
    let mock_response = r#"
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body>
            <ns:GetTemperatureResponse xmlns:ns="http://example.com/weather">
               <temperature>72.5</temperature>
              <unit>Fahrenheit</unit>
             </ns:GetTemperatureResponse>
           </soap:Body>
        </soap:Envelope>"#;

    // Deserialize the mock response body using rsoap's envelope module.
    let response: GetTemperatureResponse = rsoap::envelope::deserialize_response(mock_response)?;
    println!(
        "\nMock Response:\n  Temperature: {} {}",
        response.temperature.unwrap_or(0.0),
        response.unit.as_deref().unwrap_or("")
    );

    // Show how the full SOAP envelope looks when built for an HTTP request.
    let (action, body_xml) = GetWeather.build_request_body(&request)?;
    println!("\n--- Generated SOAP Envelope ---");
    println!("Action: {action}");
    println!("Body:\n{body_xml}");

    // Demonstrate fault detection and parsing.
    let fault_xml = r#"<soap:Fault xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
           <faultcode>Server</faultcode>
           <faultstring>Invalid credentials</faultstring>
        </soap:Fault>"#;

    let (code, message) = rsoap::envelope::parse_soap_fault(fault_xml)?;
    println!("\nParsed SOAP Fault: [{code}] {message}");

    // Show the derive macro syntax for reference.
    println!(
        r#"

    See examples/wsdl/weather.wsdl for a sample WSDL definition.

    When fully implemented, you will write:

      #[derive(rsoap::SoapOperation)]
      #[soap(wsdl = "examples/wsdl/weather.wsdl", operation_name = "GetTemperature")]
      pub struct GetWeather;

     Then at runtime:

     let client = SoapClient::new(GetWeather::ENDPOINT)?;
     let response = client.call(&GetWeather, &request).await?;

    The generated #[derive(SoapOperation)] will produce:

      pub mod gettemperature {{
         pub struct Request /* with fields from WSDL XSD types */;
         pub struct Response /* with fields from WSDL XSD types */;
     }}

     impl SoapOperation for GetWeather {{
         type Request = gettemperature::Request;
         type Response = gettemperature::Response;
         const ACTION: &str = "http://example.com/weather/GetTemperature";
         const ENDPOINT: &str = "http://localhost:8080/weather";
         const BODY_ELEMENT: &str = "ns:GetTemperatureRequest";

         fn build_request_body(&self, req: &Self::Request) -> Result<(String, String), SeError>;
         fn parse_response(&self, xml: &str) -> Result<Self::Response, SoapError>;
     }}"#
    );

    Ok(())
}

/// Stub — in production the SoapOperation trait is implemented by the derive macro.
impl SoapOperation for GetWeather {
    type Request = GetTemperatureRequest;
    type Response = GetTemperatureResponse;

    const ACTION: &'static str = "http://example.com/weather/GetTemperature";
    const ENDPOINT: &'static str = "http://localhost:8080/weather";
    const BODY_ELEMENT: &'static str = "ns:GetTemperatureRequest";

    fn build_request_body(
        &self,
        request: &Self::Request,
    ) -> Result<(String, String), rsoap::quick_xml::se::SeError> {
        let action = Self::ACTION.to_string();
        let xml = rsoap::quick_xml::se::to_string_with_root(Self::BODY_ELEMENT, request)?;
        Ok((action, xml))
    }

    fn parse_response(&self, response_xml: &str) -> Result<Self::Response, rsoap::SoapError> {
        if response_xml.contains("<soap:Fault") || response_xml.contains("<Fault xmlns=") {
            let (code, message) = rsoap::envelope::parse_soap_fault(response_xml)
                .map_err(|e| rsoap::SoapError::DeserializeResponse(Box::new(e)))?;
            return Err(rsoap::SoapError::SoapFault { code, message });
        }

        rsoap::envelope::deserialize_response::<Self::Response>(response_xml)
            .map_err(|e| rsoap::SoapError::DeserializeResponse(Box::new(e)))
    }
}
