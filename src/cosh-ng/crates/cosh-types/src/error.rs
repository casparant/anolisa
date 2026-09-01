use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum ErrorCode {
    // Generic (0xx)
    Ok = 0,
    Unknown = 1,
    InvalidInput = 2,
    PermissionDenied = 3,
    NotFound = 4,
    Timeout = 5,
    UnsupportedDistro = 6,
    OutputTooLarge = 7,
    // Legacy package-operation codes (1xx); retained for wire compatibility.
    PkgNotFound = 100,
    PkgAlreadyInstalled = 101,
    PkgDependencyConflict = 102,
    PkgBackendError = 103,
    // Legacy service-operation codes (2xx); retained for wire compatibility.
    SvcNotFound = 200,
    SvcAlreadyRunning = 201,
    SvcStartFailed = 202,
    SvcStopFailed = 203,
    // Legacy checkpoint-operation codes (3xx); retained for wire compatibility.
    CheckpointDaemonUnavailable = 300,
    CheckpointCreateFailed = 301,
    CheckpointRestoreFailed = 302,
    CheckpointNotFound = 303,
    CheckpointProtocolError = 304,
    // Audit (4xx)
    AuditDenied = 400,
    AuditPolicyError = 401,
    AuditLogError = 402,
    AuditActionMalformed = 403,
    AuditUnavailable = 404,
    AuditCorrupt = 405,
    AuditCursorInvalid = 406,
    AuditExportError = 407,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoshError {
    pub code: ErrorCode,
    pub message: String,
    pub recoverable: bool,
    pub hint: Option<String>,
    pub subsystem: String,
    pub details: Option<serde_json::Value>,
}

impl CoshError {
    pub fn new(code: ErrorCode, message: impl Into<String>, subsystem: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            recoverable: false,
            hint: None,
            subsystem: subsystem.into(),
            details: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn recoverable(mut self, recoverable: bool) -> Self {
        self.recoverable = recoverable;
        self
    }
}

impl std::fmt::Display for CoshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}: {}",
            self.subsystem, self.code as u32, self.message
        )
    }
}

impl std::error::Error for CoshError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialization_roundtrip() {
        let err = CoshError::new(ErrorCode::AuditUnavailable, "audit store missing", "audit")
            .with_hint("check the configured audit state directory")
            .recoverable(true)
            .with_details(serde_json::json!({"source": "user"}));

        let json = serde_json::to_string(&err).unwrap();
        let decoded: CoshError = serde_json::from_str(&json).unwrap();

        assert_eq!(err.code, decoded.code);
        assert_eq!(err.message, decoded.message);
        assert_eq!(err.recoverable, decoded.recoverable);
        assert_eq!(err.hint, decoded.hint);
        assert_eq!(err.subsystem, decoded.subsystem);
    }

    #[test]
    fn test_display_output() {
        let err = CoshError::new(ErrorCode::AuditPolicyError, "invalid policy", "audit");
        let s = format!("{}", err);
        assert!(s.contains("audit"));
        assert!(s.contains("401"));
        assert!(s.contains("invalid policy"));
    }

    #[test]
    fn test_checkpoint_protocol_error_contract() {
        assert_eq!(ErrorCode::CheckpointProtocolError as u32, 304);
        assert_eq!(
            serde_json::to_string(&ErrorCode::CheckpointProtocolError).unwrap(),
            "\"CheckpointProtocolError\""
        );
    }
}
