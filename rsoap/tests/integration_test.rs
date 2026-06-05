//! Integration tests for rsoap — SOAP client, envelope parsing, and code generation from WSDL.

use rsoap::{SoapClient, SoapOperation};

// ---------------------------------------------------------------------------
// Unit / smoke tests
// ---------------------------------------------------------------------------

/// Test that SoapClient creation works with valid URLs.
#[test]
fn creates_client_with_valid_url() {
    let client = SoapClient::new("https://example.com/soap").unwrap();
    assert_eq!(client.endpoint(), "https://example.com/soap");
}

/// Test that invalid URLs are rejected.
#[test]
fn rejects_invalid_url() {
    SoapClient::new("not-a-url").unwrap_err();
}

/// Test SOAP fault detection and parsing.
#[test]
fn parses_soap_fault() {
    let fault_xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                <soap:Body>
                    <soap:Fault xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                         <faultcode>Server</faultcode>
                        <faultstring>Invalid credentials</faultstring>
                     </soap:Fault>
                 </soap:Body>
             </soap:Envelope>"#;

    let (code, message) = rsoap::envelope::parse_soap_fault(fault_xml).unwrap();
    assert_eq!(code, "Server");
    assert_eq!(message, "Invalid credentials");
}

/// Test that response bodies are correctly extracted from SOAP envelopes.
#[test]
fn extracts_body_from_envelope() {
    let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
           <soap:Body>
             <GetWeatherResponse>
               <temperature>72</temperature>
             </GetWeatherResponse>
            </soap:Body>
          </soap:Envelope>"#;

    let body = rsoap::envelope::extract_body(xml).unwrap();
    assert!(body.contains("GetWeatherResponse"));
    assert!(body.contains("72"));
}

/// Test that SOAP fault strings with no code produce defaults.
#[test]
fn empty_soap_fault_defaults() {
    let fault_xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                <soap:Body>
                    <soap:Fault xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                         <faultstring>Generic error</faultstring>
                     </soap:Fault>
                 </soap:Body>
             </soap:Envelope>"#;

    let (code, message) = rsoap::envelope::parse_soap_fault(fault_xml).unwrap();
    assert_eq!(code, "unknown");
    assert_eq!(message, "Generic error");
}

/// Test that SOAP clients can be constructed with headers.
#[test]
fn client_with_headers() {
    let client = SoapClient::new("https://example.com")
        .unwrap()
        .with_header("X-Auth", "token123")
        .with_header("X-Tenant", "acme");

    let debug = format!("{client:?}");
    assert!(debug.contains("SoapClient"));
    assert!(debug.contains("X-Auth"));
    assert!(debug.contains("X-Tenant"));
    assert!(debug.contains("token123"));
    assert!(debug.contains("acme"));
}

/// Test end-to-end request serialization and response deserialization.
#[test]
fn full_serialize_deserialize_round_trip() {
    #[derive(Debug, serde::Serialize)]
    struct WeatherReq {
        zip: String,
    }

    #[derive(Debug, serde::Deserialize)]
    struct WeatherRsp {
        temp: Option<f64>,
    }

    let req = WeatherReq {
        zip: "90210".into(),
    };
    let xml = rsoap::quick_xml::se::to_string_with_root("GetWeather", &req).unwrap();
    assert!(xml.contains("90210"));

    let resp_xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
               <soap:Body>
                   <GetWeather><temp>72.5</temp>
                </GetWeather></soap:Body>
            </soap:Envelope>"#;

    let rsp: WeatherRsp = rsoap::envelope::deserialize_response(resp_xml).unwrap();
    assert_eq!(rsp.temp, Some(72.5));
}

/// Test that SoapOperation trait is correctly exported and usable.
#[test]
fn soap_operation_trait_exists() {
    // Verify the SoapOperation trait methods exist by constructing a dummy impl
    use rsoap::SoapOperation;

    struct DummyOp;

    impl SoapOperation for DummyOp {
        type Request = ();
        type Response = ();

        const ACTION: &'static str = "http://example.com/DummyOp";
        const ENDPOINT: &'static str = "http://localhost:8080/dummy";
        const BODY_ELEMENT: &'static str = "DummyRequest";

        fn build_request_body(
            &self,
            _request: &Self::Request,
        ) -> Result<(String, String), quick_xml::se::SeError> {
            Ok((Self::ACTION.into(), Self::BODY_ELEMENT.into()))
        }

        fn parse_response(&self, _response_xml: &str) -> Result<Self::Response, rsoap::SoapError>
        where
            Self::Response: serde::de::DeserializeOwned,
        {
            Ok(())
        }
    }

    let _op = DummyOp;
    assert_eq!(
        <DummyOp as SoapOperation>::ACTION,
        "http://example.com/DummyOp"
    );
    // (no ENDPOINT assertion needed - const access confirmed above)
}

// ---------------------------------------------------------------------------
// End-to-end tests with a wiremock SOAP server
// ---------------------------------------------------------------------------

/// A minimal dummy operation that produces a well-known SOAP envelope payload.
#[derive(Debug)]
struct TestOp;

impl SoapOperation for TestOp {
    type Request = WeatherReqE2e;
    type Response = WeatherRspE2e;

    const ACTION: &'static str = "http://example.com/GetWeather";
    const ENDPOINT: &'static str = "http://127.0.0.1:0/mock-soap"; // port set by mock server at runtime
    const BODY_ELEMENT: &'static str = "GetWeather";
}

#[derive(Debug, serde::Serialize)]
struct WeatherReqE2e {
    zip_code: String,
}

#[derive(Debug, PartialEq, serde::Deserialize)]
struct WeatherRspE2e {
    temperature: f64,
}

/// End-to-end: successful SOAP call through mock server.
#[tokio::test]
async fn e2e_successful_call() {
    let mock_server = wiremock::MockServer::start().await;

    // Arrange a response body matching what the macro-generated op would expect
    let soap_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <GetWeatherResponse>
                    <temperature>72.5</temperature>
                </GetWeatherResponse>
            </soap:Body>
        </soap:Envelope>"#;

    // Mount the mock expectation
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_response))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "90210".into(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "expected successful call, got error: {:?}",
        result.err()
    );
    let rsp = result.unwrap();
    assert_eq!(rsp.temperature, 72.5);
}

/// End-to-end: soap fault returned by server produces SoapError::SoapFault.
#[tokio::test]
async fn e2e_soap_fault() {
    let mock_server = wiremock::MockServer::start().await;

    let soap_fault = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <soap:Fault xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                    <faultcode>Client</faultcode>
                    <faultstring>Invalid API key</faultstring>
                </soap:Fault>
            </soap:Body>
        </soap:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_fault))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "10001".into(),
            },
        )
        .await;

    assert!(result.is_err(), "expected SoapFault error");
    match result.unwrap_err() {
        rsoap::SoapError::SoapFault { code, message } => {
            assert_eq!(code, "Client");
            assert_eq!(message, "Invalid API key");
        }
        other => panic!("expected SoapFault, got {:?}", other),
    }
}

/// End-to-end: server returns non-200 HTTP status → SoapError::Http.
#[tokio::test]
async fn e2e_http_error() {
    let mock_server = wiremock::MockServer::start().await;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "0".into(),
            },
        )
        .await;

    assert!(result.is_err(), "expected HTTP error");
    match result.unwrap_err() {
        rsoap::SoapError::HttpStatus { code, .. } => assert_eq!(code, 500),
        other => panic!("expected SoapError::HttpStatus, got {:?}", other),
    }
}

/// End-to-end: mock server validates that the request body contains expected XML.
#[tokio::test]
async fn e2e_request_body_check() {
    let mock_server = wiremock::MockServer::start().await;

    let soap_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
              <soap:Body>
                  <GetWeatherResponse><temperature>68.0</temperature></GetWeatherResponse>
              </soap:Body>
          </soap:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::header(
            "Content-Type",
            "text/xml; charset=utf-8",
        ))
        .and(|req: &wiremock::Request| {
            String::from_utf8(req.body.clone())
                .unwrap_or_default()
                .contains("90210")
        })
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_response))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "90210".into(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "expected successful call with body check, got error: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().temperature, 68.0);
}

/// End-to-end: custom headers are included in HTTP requests made through the client.
#[tokio::test]
async fn e2e_custom_headers_sent() {
    let mock_server = wiremock::MockServer::start().await;

    // Verify the Authorization header was sent with the correct value
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::header("Authorization", "Bearer mytoken123"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .set_body_string(r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                <soap:Body><GetWeatherResponse><temperature>80.0</temperature></GetWeatherResponse></soap:Body>
            </soap:Envelope>"#))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri())
        .unwrap()
        .with_header("Authorization", "Bearer mytoken123");

    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "30301".into(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "expected successful call with auth header, got error: {:?}",
        result.err()
    );
}
