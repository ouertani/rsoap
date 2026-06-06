# rsoap

![Rust](https://img.shields.io/badge/rust-1.95%2B-brightgreen.svg)
[![Version](https://img.shields.io/crates/v/rsoap.svg)](https://crates.io/crates/rsoap)
[![Docs.rs](https://docs.rs/rsoap/badge.svg)](https://docs.rs/rsoap)
[![License](https://img.shields.io/crates/l/rsoap.svg)](LICENSE)
[![Downloads](https://img.shields.io/crates/d/rsoap.svg)](https://crates.io/crates/rsoap)

> A SOAP client library for Rust with compile-time code generation from WSDL files. The `rsoap` workspace provides a runtime client (`rsoap`) and a procedural macro (`rsoap-macros`) that parses a WSDL at compile time and generates typed request/response structs, so SOAP services feel like ordinary Rust APIs.

---

## Workspace

| Crate           | Role                                                              |
|-----------------|-------------------------------------------------------------------|
| `rsoap/`        | Runtime library — `SoapClient`, `SoapOperation` trait, envelope/XML parsing, `SoapError` |
| `rsoap-macros/` | Proc-macro crate — WSDL parser → typed struct generation          |
| `examples/`     | Demo — weather service (hand-written `SoapOperation` impl)        |

---

## Quick Start

Add the runtime crate and the derive macro to your `Cargo.toml`:

```toml
[dependencies]
rsoap = { path = "../rsoap", features = ["wss"] }  # omit "wss" if you don't need mTLS
rsoap-macros = { path = "../rsoap-macros" }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
anyhow = "1"
```

Derive a typed operation from a WSDL at compile time:

```rust,ignore
use rsoap::{SoapClient, SoapOperation, SoapOperation as SoapOperationMacro};

#[derive(SoapOperationMacro)]
#[soap(wsdl = "wsdl/weather.wsdl", operation_name = "GetTemperature")]
pub struct GetTemperature;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = SoapClient::new(GetTemperature::ENDPOINT)?
        .with_header("X-Api-Key", "test-key");

    let request = gettemperature::Request { zipcode: "90210".into() };
    let response = client.call(&GetTemperature, &request).await?;
    println!("{} {}", response.temperature, response.unit);
    Ok(())
}
```

The macro reads the WSDL, resolves the `GetTemperature` operation, generates `gettemperature::Request` and `gettemperature::Response` with `#[serde(rename = "...")]` on every field, and implements `SoapOperation` for the marker struct.

### Two-way SSL / mutual TLS (WS-Security)

For services that require client certificates ("Services will be available only over two-way SSL over HTTP"), enable the `wss` feature and pass a PEM-encoded cert + key to the client:

```rust,ignore
use rsoap::SoapClient;

let client = SoapClient::new("https://example.com/soap")?
    .with_client_cert("/path/to/client.pem")?;
```

The certificate is presented on every request, satisfying the transport-binding requirement of OASIS WSS 1.1. For PKCS#12 (.p12 / .pfx) bundles, build the `reqwest::Identity` yourself (requires reqwest's `default-tls` feature) and use `SoapClient::with_identity`.

---

## Features

- **Compile-time WSDL parsing** — no runtime XML schema download, no manual struct definitions.
- **Typed request/response** — generated structs with `serde` rename attributes match the XSD element names.
- **`maxOccurs="unbounded"`** — automatically wrapped in `Vec<T>`.
- **Optional & nillable elements** — XSD elements with `minOccurs="0"` or `nillable="true"` are mapped to `Option<T>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`, so absent fields deserialize cleanly and `None` values are omitted from the wire format. Repeating elements (`Vec<T>`) are never wrapped — an empty `Vec` already represents absence.
- **XSD → Rust type mapping** — `xs:string` → `String`, `xs:int` → `i32`, `xs:long` → `i64`, `xs:float`/`xs:double`/`xs:decimal` → `f64`, `xs:boolean` → `bool`, `xs:date`/`xs:dateTime` → `String`.
- **SOAP 1.1 & 1.2 support** — version is per-operation (`const VERSION: SoapVersion`) and auto-detected from the WSDL binding namespace. 1.1 uses `text/xml` + `SOAPAction`; 1.2 uses `application/soap+xml` with the action in the Content-Type parameter, and parses the 1.2 fault structure (`<Code><Value>` / `<Reason><Text>`).
- **Fault detection on any HTTP status** — non-2xx responses are still read and checked for a SOAP fault body before reporting `HttpStatus`.
- **WS-Security transport binding (mTLS)** — opt-in via the `wss` Cargo feature. Loads a PEM-encoded client certificate + key and presents it on every request, matching the "two-way SSL over HTTP" requirement from OASIS WSS 1.1.
- **Custom headers** — `.with_header(name, value)` for auth, tracing, etc.
- **Namespace-prefix tolerant** — handles `xs:`, `xsd:`, `wsdl:`, `soap:`, `wsdlsoap:`, `env:`, and bare tags in WSDLs.

---

## `SoapError`

```rust
pub enum SoapError {
    Http(String),
    SerializeRequest,
    DeserializeResponse(Box<dyn std::error::Error + Send + Sync>),
    SoapFault { code: String, message: String },
    OperationNotFound,
    NoEndpoint,
}
```

Implements `From<reqwest::Error>` for ergonomic `?` propagation.

---

## `SoapOperation` trait

```rust
pub trait SoapOperation: Send + Sync {
    type Request: Serialize;
    type Response: for<'de> Deserialize<'de>;

    const ACTION: &'static str;
    const ENDPOINT: &'static str;
    const BODY_ELEMENT: &'static str;
    /// SOAP protocol version. Defaults to V11; the derive macro auto-detects
    /// from the WSDL binding namespace (V12 if the WSDL uses
    /// `http://schemas.xmlsoap.org/wsdl/soap12/`).
    const VERSION: SoapVersion = SoapVersion::V11;

    fn build_request_body(&self, req: &Self::Request) -> Result<(String, String), quick_xml::se::SeError> {
        let action = Self::ACTION.to_string();
        let xml = quick_xml::se::to_string_with_root(Self::BODY_ELEMENT, req)?;
        Ok((action, xml))
    }

    fn parse_response(&self, xml: &str) -> Result<Self::Response, SoapError> { /* default */ }
}
```

A hand-written operation can declare a SOAP 1.2 version explicitly:

```rust,ignore
impl SoapOperation for MyOp {
    // ...
    const VERSION: SoapVersion = SoapVersion::V12;
}
```

---

## Build & Test

```bash
# Build everything
cargo build --workspace

# Lint (correctness is deny-level — hard failures)
cargo clippy --workspace

# Run all tests (unit + integration + doc tests)
cargo test --workspace

# Run specific test suites
cargo test -p rsoap-macros --lib              # macro unit tests
cargo test -p rsoap --test integration_test   # wiremock e2e tests
cargo run -p rsoap-examples                    # run the demo
```

---

## Configuration

The workspace enforces strict lints at the root `Cargo.toml`:

```toml
[workspace.lints.clippy]
correctness = { level = "deny" }
suspicious  = { level = "warn" }
style       = { level = "warn" }
complexity  = { level = "warn" }
perf        = { level = "warn" }

[workspace.lints.rust]
missing_docs = "warn"
unsafe_code  = "deny"
```

`unsafe` is never allowed. `missing_docs` is warn-level — doc comments on public items are encouraged.

---

## Limitations

- The macro uses a lightweight XML walker tuned for WSDL — it handles namespace prefixes (`xs:`, `xsd:`, `wsdlsoap:`, `wsdlsoap12:`, `env:`), self-closing tags (`<xs:element name="x"/>`), and attributes on opening tags. It is not a general XML schema validator: CDATA sections, comments containing tag-like text, or pathological nesting (same-name elements nested inside themselves) may confuse it. Standard WSDLs from SoapUI, WSDL2Java, .NET, and similar tools work correctly.
- No MTOM / attachments.
- `rsoap-macros` reads the WSDL at compile time, so the file path must be valid relative to the crate root where `#[derive]` is invoked.

---

## License

MIT — see [LICENSE](LICENSE).
