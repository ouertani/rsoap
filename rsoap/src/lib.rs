//! # rsoap
//!
//! A SOAP client library for Rust with compile-time code generation from WSDL files.
//!
//! ## Quick Start
//!
//! ```no_run
//! use rsoap::{SoapClient, SoapOperation};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = SoapClient::new("https://example.com/soap")?;
//!          // Generated operation methods are called on the generated client trait impls.
//!     Ok(())
//! }
//! ```
//!
//! ## Code Generation
//!
//! The `#[derive(SoapOperationMacro)]` macro reads a WSDL file at compile time and generates:
//! - Typed request/response structs matching the WSDL schema
//! - An implementation of [`SoapOperation`] from [`rsoap::client`]
//!
//! ```ignore
//! use rsoap::{SoapClient, SoapOperation};
//!
//! #[derive(SoapOperationMacro)]
//! #[soap(wsdl = "path/to/service.wsdl", operation_name = "GetWeather")]
//! pub struct GetWeather;
//!
//! async fn get_temp(client: &SoapClient) -> anyhow::Result<()> {
//!     let request = getweather::Request { zipcode: "90210".into() };
//!     let response = client.call(&GetWeather, &request).await?;
//!     println!("Temp: {}", response.temperature);
//! }
//! ```

#![warn(missing_docs)]

pub mod client;
pub mod envelope;
pub mod error;

// Re-export core types at crate root for ergonomic access.
pub use self::client::{SoapClient, SoapOperation};
pub use self::envelope::SoapVersion;
pub use self::error::{CertError, SoapError};

// Re-export the proc-macro so users can derive SoapOperationMacro on custom types.
pub use rsoap_macros::SoapOperation as SoapOperationMacro;

// Re-serialize dependencies so users don't need to pin versions.
pub use quick_xml;
pub use serde;
pub use thiserror;
