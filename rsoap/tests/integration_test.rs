//! Integration tests for rsoap — SOAP client, envelope parsing, and code generation from WSDL.

use rsoap::{SoapClient, SoapOperation, SoapVersion};

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

// ---------------------------------------------------------------------------
// SOAP 1.2 tests
// ---------------------------------------------------------------------------

/// A 1.2 version of the dummy operation for end-to-end SOAP 1.2 testing.
#[derive(Debug)]
struct TestOp12;

impl SoapOperation for TestOp12 {
    type Request = WeatherReqE2e;
    type Response = WeatherRspE2e;

    const ACTION: &'static str = "http://example.com/GetWeather12";
    const ENDPOINT: &'static str = "http://127.0.0.1:0/mock-soap12";
    const BODY_ELEMENT: &'static str = "GetWeather";
    const VERSION: SoapVersion = SoapVersion::V12;
}

/// End-to-end: SOAP 1.2 request — verify Content-Type includes the action
/// parameter and no `SOAPAction` HTTP header is sent.
#[tokio::test]
async fn e2e_soap12_content_type_carries_action() {
    let mock_server = wiremock::MockServer::start().await;

    let soap_response = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
            <env:Body>
                <GetWeatherResponse>
                    <temperature>65.0</temperature>
                </GetWeatherResponse>
            </env:Body>
        </env:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::header_regex(
            "Content-Type",
            r#"^application/soap\+xml; charset=utf-8; action="http://example.com/GetWeather12""#,
        ))
        .and(|req: &wiremock::Request| {
            !req.headers
                .iter()
                .any(|(k, _)| k.as_str().eq_ignore_ascii_case("SOAPAction"))
        })
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_response))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp12,
            &WeatherReqE2e {
                zip_code: "20001".into(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "expected successful 1.2 call, got error: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().temperature, 65.0);
}

/// End-to-end: SOAP 1.2 request — verify envelope uses env: prefix and 1.2 namespace.
#[tokio::test]
async fn e2e_soap12_envelope_uses_env_namespace() {
    let mock_server = wiremock::MockServer::start().await;

    let soap_response = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
            <env:Body>
                <GetWeatherResponse>
                    <temperature>70.0</temperature>
                </GetWeatherResponse>
            </env:Body>
        </env:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(|req: &wiremock::Request| {
            String::from_utf8(req.body.clone())
                .unwrap_or_default()
                .contains("<env:Envelope")
                && String::from_utf8(req.body.clone())
                    .unwrap_or_default()
                    .contains("http://www.w3.org/2003/05/soap-envelope")
        })
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_response))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp12,
            &WeatherReqE2e {
                zip_code: "94101".into(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "expected successful 1.2 envelope call, got error: {:?}",
        result.err()
    );
}

/// End-to-end: SOAP 1.2 fault — server returns a 1.2 fault, client detects it.
#[tokio::test]
async fn e2e_soap12_fault_detected() {
    let mock_server = wiremock::MockServer::start().await;

    let soap_fault = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
            <env:Body>
                <env:Fault>
                    <Code><Value>env:Sender</Value></Code>
                    <Reason><Text xml:lang="en">Invalid zip code</Text></Reason>
                </env:Fault>
            </env:Body>
        </env:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_fault))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let result: Result<WeatherRspE2e, _> = client
        .call(
            &TestOp12,
            &WeatherReqE2e {
                zip_code: "00000".into(),
            },
        )
        .await;

    assert!(result.is_err(), "expected SoapFault error for 1.2");
    match result.unwrap_err() {
        rsoap::SoapError::SoapFault { code, message } => {
            assert_eq!(code, "env:Sender");
            assert_eq!(message, "Invalid zip code");
        }
        other => panic!("expected SoapFault, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Optional & nillable element support (minOccurs="0" / nillable="true")
// ---------------------------------------------------------------------------

/// End-to-end check that the derive macro maps `minOccurs="0"` and
/// `nillable="true"` XSD attributes to `Option<T>` Rust fields, and that
/// the generated struct round-trips through serde with the expected
/// skip-on-none semantics.
mod optional_nillable {
    use rsoap::SoapOperationMacro;

    #[derive(SoapOperationMacro)]
    #[soap(
        wsdl = "rsoap/tests/wsdl/customer.wsdl",
        operation_name = "GetCustomer"
    )]
    #[allow(dead_code)]
    pub struct GetCustomer;

    #[test]
    fn generated_request_has_optional_and_required_fields() {
        // Required field must be set; optional fields default to None / empty.
        let req = getcustomer::GetCustomerRequest {
            id: 42,
            middle_name: None,
            death_date: None,
            tag: Vec::new(),
        };
        // The values themselves prove the field types at compile time:
        // - `id: i32` (required)            -> not Option
        // - `middle_name: Option<String>`   -> minOccurs="0"
        // - `death_date: Option<String>`    -> nillable="true"
        // - `tag: Vec<String>`              -> unbounded stays Vec, never Option
        assert_eq!(req.id, 42);
        assert!(req.middle_name.is_none());
        assert!(req.death_date.is_none());
        assert!(req.tag.is_empty());
    }

    #[test]
    fn optional_none_fields_are_skipped_during_serialization() {
        let req = getcustomer::GetCustomerRequest {
            id: 7,
            middle_name: None,
            death_date: None,
            tag: vec!["a".into()],
        };
        let xml = quick_xml::se::to_string_with_root("Req", &req).unwrap();
        assert!(xml.contains("<id>7</id>"));
        assert!(xml.contains("<tag>a</tag>"));
        // None-valued optional fields must NOT appear in the wire format.
        assert!(
            !xml.contains("middleName"),
            "middleName should be skipped when None, got: {xml}"
        );
        assert!(
            !xml.contains("deathDate"),
            "deathDate should be skipped when None, got: {xml}"
        );
    }

    #[test]
    fn optional_some_fields_are_serialized_with_xsd_element_name() {
        let req = getcustomer::GetCustomerRequest {
            id: 7,
            middle_name: Some("Quentin".into()),
            death_date: Some("2010-05-12".into()),
            tag: Vec::new(),
        };
        let xml = quick_xml::se::to_string_with_root("Req", &req).unwrap();
        assert!(
            xml.contains("<middleName>Quentin</middleName>"),
            "xml: {xml}"
        );
        assert!(
            xml.contains("<deathDate>2010-05-12</deathDate>"),
            "xml: {xml}"
        );
    }

    #[test]
    fn response_with_missing_optional_field_deserializes_to_none() {
        // The `note` element is `minOccurs="0"` — a server may omit it.
        // Without `Option<T>` + `#[serde(default)]` this would fail to parse.
        let xml = "<GetCustomerResponse><status>OK</status></GetCustomerResponse>";
        let resp: getcustomer::GetCustomerResponse = quick_xml::de::from_str(xml).unwrap();
        assert_eq!(resp.status, "OK");
        assert!(resp.note.is_none());
    }

    #[test]
    fn response_with_present_optional_field_deserializes_to_some() {
        let xml =
            "<GetCustomerResponse><status>OK</status><note>hello</note></GetCustomerResponse>";
        let resp: getcustomer::GetCustomerResponse = quick_xml::de::from_str(xml).unwrap();
        assert_eq!(resp.status, "OK");
        assert_eq!(resp.note.as_deref(), Some("hello"));
    }
}

// ---------------------------------------------------------------------------
// Logger hook
// ---------------------------------------------------------------------------

/// End-to-end: registered logger receives the outbound envelope and the
/// inbound response body in the correct order.
#[tokio::test]
async fn logger_captures_request_and_response_in_order() {
    use std::sync::{Arc, Mutex};

    let mock_server = wiremock::MockServer::start().await;
    let soap_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <GetWeatherResponse>
                    <temperature>55.0</temperature>
                </GetWeatherResponse>
            </soap:Body>
        </soap:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_response))
        .mount(&mock_server)
        .await;

    let captured: Arc<Mutex<Vec<(rsoap::LogDirection, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_for_logger = Arc::clone(&captured);

    let client = SoapClient::new(mock_server.uri())
        .unwrap()
        .with_logger(move |dir, xml| {
            captured_for_logger
                .lock()
                .unwrap()
                .push((dir, xml.to_string()));
        });

    let _ = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "11111".into(),
            },
        )
        .await
        .expect("call should succeed");

    let log = captured.lock().unwrap();
    assert_eq!(log.len(), 2, "logger should be called exactly twice");

    // First call: outbound request.
    assert_eq!(log[0].0, rsoap::LogDirection::Request);
    assert!(
        log[0].1.contains("soap:Envelope"),
        "request log should contain envelope, got: {}",
        log[0].1
    );
    assert!(
        log[0].1.contains("11111"),
        "request log should contain request body, got: {}",
        log[0].1
    );

    // Second call: inbound response.
    assert_eq!(log[1].0, rsoap::LogDirection::Response);
    assert!(
        log[1].1.contains("GetWeatherResponse"),
        "response log should contain response body, got: {}",
        log[1].1
    );
    assert!(log[1].1.contains("55.0"));
}

/// Logger still fires for the response when the server returns a SOAP fault,
/// so users can debug fault payloads.
#[tokio::test]
async fn logger_captures_fault_response_body() {
    use std::sync::{Arc, Mutex};

    let mock_server = wiremock::MockServer::start().await;
    let soap_fault = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <soap:Fault>
                    <faultcode>Server</faultcode>
                    <faultstring>boom</faultstring>
                </soap:Fault>
            </soap:Body>
        </soap:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(500).set_body_string(soap_fault))
        .mount(&mock_server)
        .await;

    let captured: Arc<Mutex<Vec<(rsoap::LogDirection, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_for_logger = Arc::clone(&captured);

    let client = SoapClient::new(mock_server.uri())
        .unwrap()
        .with_logger(move |dir, xml| {
            captured_for_logger
                .lock()
                .unwrap()
                .push((dir, xml.to_string()));
        });

    let err = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "11111".into(),
            },
        )
        .await
        .expect_err("server returned a fault");
    assert!(matches!(err, rsoap::SoapError::SoapFault { .. }));

    let log = captured.lock().unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[1].0, rsoap::LogDirection::Response);
    assert!(log[1].1.contains("boom"));
}

/// A client with no logger registered must continue to work unchanged.
#[tokio::test]
async fn no_logger_does_not_affect_behavior() {
    let mock_server = wiremock::MockServer::start().await;
    let soap_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <GetWeatherResponse>
                    <temperature>21.0</temperature>
                </GetWeatherResponse>
            </soap:Body>
        </soap:Envelope>"#;

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(soap_response))
        .mount(&mock_server)
        .await;

    let client = SoapClient::new(mock_server.uri()).unwrap();
    let rsp = client
        .call(
            &TestOp,
            &WeatherReqE2e {
                zip_code: "22222".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(rsp.temperature, 21.0);
}
