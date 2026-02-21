//! Error types for the Webcash wallet library

/// Result type alias for Webcash operations
pub type Result<T> = std::result::Result<T, Error>;

/// Comprehensive error type for all Webcash operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O errors (file operations, network, etc.)
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Database errors
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// JSON parsing/serialization errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// HTTP client errors
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Parsing errors for webcash strings
    #[error("Parse error: {message}")]
    Parse { message: String },

    /// Amount validation errors
    #[error("Amount error: {message}")]
    Amount { message: String },

    /// Cryptographic operation errors
    #[error("Crypto error: {message}")]
    Crypto { message: String },

    /// Server communication errors
    #[error("Server error: {message}")]
    Server { message: String },

    /// Wallet state errors
    #[error("Wallet error: {message}")]
    Wallet { message: String },

    /// Terms of service not accepted
    #[error("Terms of service must be accepted")]
    TermsNotAccepted,

    /// Invalid input parameters
    #[error("Invalid input: {message}")]
    InvalidInput { message: String },

    /// Operation not supported
    #[error("Operation not supported: {message}")]
    NotSupported { message: String },

    /// Authentication/authorization errors
    #[error("Authentication error: {message}")]
    Auth { message: String },

    /// Insufficient funds for operation
    #[error("Insufficient funds: needed {needed}, available {available}")]
    InsufficientFunds { needed: String, available: String },

    /// Generic errors with custom messages
    #[error("{message}")]
    Other { message: String },
}

impl Error {
    /// Create a parse error
    pub fn parse<S: Into<String>>(message: S) -> Self {
        Error::Parse {
            message: message.into(),
        }
    }

    /// Create an amount error
    pub fn amount<S: Into<String>>(message: S) -> Self {
        Error::Amount {
            message: message.into(),
        }
    }

    /// Create a crypto error
    pub fn crypto<S: Into<String>>(message: S) -> Self {
        Error::Crypto {
            message: message.into(),
        }
    }

    /// Create a server error
    pub fn server<S: Into<String>>(message: S) -> Self {
        Error::Server {
            message: message.into(),
        }
    }

    /// Create a wallet error
    pub fn wallet<S: Into<String>>(message: S) -> Self {
        Error::Wallet {
            message: message.into(),
        }
    }

    /// Create an invalid input error
    pub fn invalid_input<S: Into<String>>(message: S) -> Self {
        Error::InvalidInput {
            message: message.into(),
        }
    }

    /// Create a not supported error
    pub fn not_supported<S: Into<String>>(message: S) -> Self {
        Error::NotSupported {
            message: message.into(),
        }
    }

    /// Create an auth error
    pub fn auth<S: Into<String>>(message: S) -> Self {
        Error::Auth {
            message: message.into(),
        }
    }

    /// Create an insufficient funds error
    pub fn insufficient_funds<S: Into<String>>(needed: S, available: S) -> Self {
        Error::InsufficientFunds {
            needed: needed.into(),
            available: available.into(),
        }
    }

    /// Create a generic error
    pub fn other<S: Into<String>>(message: S) -> Self {
        Error::Other {
            message: message.into(),
        }
    }

    /// Add context to an error
    pub fn with_context<S: Into<String>>(self, context: S) -> Self {
        match self {
            Error::Io(e) => Error::Io(e),
            Error::Database(e) => Error::Database(e),
            Error::Json(e) => Error::Json(e),
            Error::Http(e) => Error::Http(e),
            Error::Parse { message } => Error::Parse {
                message: format!("{}: {}", context.into(), message),
            },
            Error::Amount { message } => Error::Amount {
                message: format!("{}: {}", context.into(), message),
            },
            Error::Crypto { message } => Error::Crypto {
                message: format!("{}: {}", context.into(), message),
            },
            Error::Server { message } => Error::Server {
                message: format!("{}: {}", context.into(), message),
            },
            Error::Wallet { message } => Error::Wallet {
                message: format!("{}: {}", context.into(), message),
            },
            Error::TermsNotAccepted => Error::TermsNotAccepted,
            Error::InvalidInput { message } => Error::InvalidInput {
                message: format!("{}: {}", context.into(), message),
            },
            Error::NotSupported { message } => Error::NotSupported {
                message: format!("{}: {}", context.into(), message),
            },
            Error::Auth { message } => Error::Auth {
                message: format!("{}: {}", context.into(), message),
            },
            Error::InsufficientFunds { needed, available } => Error::InsufficientFunds {
                needed: format!("{}: {}", context.into(), needed),
                available,
            },
            Error::Other { message } => Error::Other {
                message: format!("{}: {}", context.into(), message),
            },
        }
    }
}
