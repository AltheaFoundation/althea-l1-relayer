//! Custom error types for the relayer application

use std::fmt;

/// Main error type for relayer operations
#[derive(Debug)]
pub enum RelayerError {
    /// Error fetching or processing network data
    NetworkError(String),

    /// Error parsing or validating transaction data
    TransactionError(String),

    /// Invalid configuration or parameters
    ConfigurationError(String),

    /// Error interacting with Web3/Ethereum
    Web3Error(web30::jsonrpc::error::Web3Error),

    /// Generic error with message
    Other(String),
}

impl fmt::Display for RelayerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RelayerError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            RelayerError::TransactionError(msg) => write!(f, "Transaction error: {}", msg),
            RelayerError::ConfigurationError(msg) => write!(f, "Configuration error: {}", msg),
            RelayerError::Web3Error(err) => write!(f, "Web3 error: {}", err),
            RelayerError::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl std::error::Error for RelayerError {}

impl From<web30::jsonrpc::error::Web3Error> for RelayerError {
    fn from(err: web30::jsonrpc::error::Web3Error) -> Self {
        RelayerError::Web3Error(err)
    }
}

impl From<String> for RelayerError {
    fn from(err: String) -> Self {
        RelayerError::Other(err.to_string())
    }
}

impl From<&str> for RelayerError {
    fn from(err: &str) -> Self {
        RelayerError::Other(err.to_string())
    }
}

impl From<clarity::Error> for RelayerError {
    fn from(err: clarity::Error) -> Self {
        RelayerError::Other(format!("Clarity error: {:?}", err))
    }
}
