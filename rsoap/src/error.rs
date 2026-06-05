//! Error types for Soap operations.

use thiserror::Error;

/// The primary error type for Soap operations.
#[derive(Debug, Error)]
pub enum SoapError {
    /// HTTP request failed.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Failed to serialize the request XML.
    #[error("failed to serialize request: {0}")]
    SerializeRequest(#[source] quick_xml::se::SeError),

    /// Failed to deserialize the response XML.
    #[error("failed to deserialize response: {0}")]
    DeserializeResponse(#[source] Box<quick_xml::de::DeError>),

     /// The SOAP envelope contained a fault from the server.
     #[error("Soap fault: [{code}] {message}")]
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

impl From<reqwest::Error> for SoapError {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.to_string())
    }
}

impl SoapError {
    /// Construct a `SoapError::Http` from an HTTP status code.
    pub fn http(status: reqwest::StatusCode) -> Self {
        Self::Http(format!(
            "HTTP {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        ))
    }

    /// Construct a `SoapError::SerializeRequest` from an XML serialization error.
    pub fn serialize_request(err: quick_xml::se::SeError) -> Self {
        Self::SerializeRequest(err)
    }
}
