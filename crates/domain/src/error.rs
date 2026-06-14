use std::error::Error;
use std::fmt;

/// What the caller can do about an error — categorised by recovery, not origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppErrorKind {
    /// Bad data supplied by the caller (validation, missing fields).
    InvalidInput,
    /// Requested entity does not exist.
    NotFound,
    /// State precondition violated (already exists / already processed).
    Conflict,
    /// Business rule violated that is not a simple state conflict.
    BusinessRuleViolation,
    /// Unexpected infrastructure or I/O failure the caller cannot fix.
    Internal,
    /// Caller is not authenticated or the token is invalid.
    Unauthorized,
    /// Caller is authenticated but lacks permission for the action.
    Forbidden,
    /// Downstream dependency is temporarily unavailable.
    Unavailable,
    /// Caller has exceeded the allowed request rate.
    RateLimited,
}

/// The single application error type.
///
/// `Display` shows only the human-readable, client-safe message (and masks
/// internal errors). `Debug` shows the full tree: operation, context, and the
/// underlying source chain.
pub struct AppError {
    kind: AppErrorKind,
    message: String,
    operation: Option<String>,
    context: Vec<(String, String)>,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl AppError {
    fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            operation: None,
            context: Vec::new(),
            source: None,
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Conflict, message)
    }

    pub fn business_rule_violation(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::BusinessRuleViolation, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Internal, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Unauthorized, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Forbidden, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Unavailable, message)
    }

    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::RateLimited, message)
    }

    pub fn kind(&self) -> AppErrorKind {
        self.kind
    }

    /// Name the operation that failed (`"Module::function"` or `"CommandVariant"`).
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Attach a named input that helps locate the failing record.
    pub fn with_context(mut self, key: impl Into<String>, value: impl fmt::Display) -> Self {
        self.context.push((key.into(), value.to_string()));
        self
    }

    /// Attach the underlying cause, preserving the full error chain.
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

impl fmt::Display for AppError {
    /// Client-safe message only. Internal errors are masked.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == AppErrorKind::Internal {
            write!(f, "Internal error")
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl fmt::Debug for AppError {
    /// Full diagnostic tree for server-side / local investigation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)?;
        if let Some(operation) = &self.operation {
            write!(f, " [operation={operation}]")?;
        }
        for (key, value) in &self.context {
            write!(f, " [{key}={value}]")?;
        }
        if let Some(source) = &self.source {
            write!(f, "\n  caused by: {source:?}")?;
        }
        Ok(())
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|boxed| boxed.as_ref() as &(dyn Error + 'static))
    }
}

/// A fallible operation that yields a value of type `T` on success.
pub type AppResult<T> = Result<T, AppError>;

/// A fallible action performed for its effect, yielding nothing on success.
pub type AppAction = Result<(), AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_are_masked_in_display() {
        let err = AppError::internal("connection pool exhausted");
        assert_eq!(err.to_string(), "Internal error");
    }

    #[test]
    fn non_internal_errors_show_their_message() {
        let err = AppError::not_found("Slice does not exist");
        assert_eq!(err.to_string(), "Slice does not exist");
    }

    #[test]
    fn debug_includes_operation_and_context() {
        let err = AppError::not_found("Slice does not exist")
            .with_operation("BoardService::load_board")
            .with_context("repo", "funkode-io/zfirot");
        let debug = format!("{err:?}");
        assert!(debug.contains("BoardService::load_board"));
        assert!(debug.contains("funkode-io/zfirot"));
    }
}
