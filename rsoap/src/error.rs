//! Error types for Soap operations.

use thiserror::Error;

/// The primary error type for Soap operations.
#[derive(Debug, Error)]
pub enum SoapError {
    /// HTTP transport-level failure (connection, timeout, TLS, etc.).
    /// Wraps the underlying [`reqwest::Error`] for full source-chain visibility.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Server returned a non-success HTTP status code.
    #[error("HTTP {code}: {reason}")]
    HttpStatus {
        /// The numeric HTTP status code (e.g., 500).
        code: u16,
        /// The canonical reason phrase (e.g., "Internal Server Error").
        reason: String,
    },

    /// Failed to serialize the request XML.
    #[error("failed to serialize request: {0}")]
    SerializeRequest(#[source] quick_xml::se::SeError),

    /// Failed to deserialize the response XML.
    #[error("failed to deserialize response: {0}")]
    DeserializeResponse(#[source] Box<quick_xml::de::DeError>),

    /// The SOAP envelope contained a fault from the server.
    #[error("soap fault: [{code}] {message}")]
    SoapFault {
        /// The WSDL-defined fault code (e.g., "Client", "Server").
        code: String,
        /// Human-readable description of the fault.
        message: String,
    },

    /// The requested operation could not be found in the WSDL.
    #[error("operation '{name}' not found in WSDL definition")]
    OperationNotFound {
        /// The name of the operation that was not found.
        name: String,
    },

    /// Missing or invalid endpoint URL.
    #[error("no endpoint URL configured for Soap client")]
    NoEndpoint,
}

impl SoapError {
    /// Construct a `SoapError::HttpStatus` from an HTTP status code.
    pub fn http_status(status: reqwest::StatusCode) -> Self {
        Self::HttpStatus {
            code: status.as_u16(),
            reason: status.canonical_reason().unwrap_or("Unknown").to_string(),
        }
    }

    /// Construct a `SoapError::SerializeRequest` from an XML serialization error.
    pub fn serialize_request(err: quick_xml::se::SeError) -> Self {
        Self::SerializeRequest(err)
    }
}
