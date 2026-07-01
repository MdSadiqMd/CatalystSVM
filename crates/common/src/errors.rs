use thiserror::Error;

#[derive(Debug, Error)]
pub enum IngressError {
    #[error("invalid transaction format: {0}")]
    InvalidFormat(String),

    #[error("invalid sender: {0}")]
    InvalidSender(String),

    #[error("invalid program ID: {0}")]
    InvalidProgram(String),

    #[error("compute budget {requested} exceeds maximum {max}")]
    ComputeBudgetExceeded { requested: u64, max: u64 },

    #[error("empty instruction data")]
    EmptyInstruction,

    #[error("validation failed: {0}")]
    ValidationFailed(String),
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("invalid instruction opcode: {0}")]
    InvalidOpcode(u8),

    #[error("insufficient balance for transfer: need {needed}, have {available}")]
    InsufficientBalance { needed: u64, available: u64 },

    #[error("account not found: {0}")]
    AccountNotFound(String),

    #[error("compute budget exceeded: used {used}, limit {limit}")]
    ComputeBudgetExceeded { used: u64, limit: u64 },

    #[error("batch execution failed: {0}")]
    BatchFailed(String),

    #[error("state corruption detected: {0}")]
    StateCorruption(String),
}

#[derive(Debug, Error)]
pub enum ProveError {
    #[error("invalid trace: {0}")]
    InvalidTrace(String),

    #[error("commitment generation failed: {0}")]
    CommitmentFailed(String),

    #[error("proof serialization failed: {0}")]
    SerializationFailed(String),

    #[error("prover internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("commitment mismatch: expected {expected}, got {actual}")]
    CommitmentMismatch { expected: String, actual: String },

    #[error("invalid proof format: {0}")]
    InvalidProofFormat(String),

    #[error("trace hash mismatch")]
    TraceHashMismatch,

    #[error("state root mismatch: pre={pre_match}, post={post_match}")]
    StateRootMismatch { pre_match: bool, post_match: bool },

    #[error("verification failed: {0}")]
    VerificationFailed(String),
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("ingress error: {0}")]
    Ingress(#[from] IngressError),

    #[error("execution error: {0}")]
    Execution(#[from] ExecError),

    #[error("proving error: {0}")]
    Proving(#[from] ProveError),

    #[error("verification error: {0}")]
    Verification(#[from] VerifyError),

    #[error("queue is empty")]
    EmptyQueue,

    #[error("batch not found: {0}")]
    BatchNotFound(String),

    #[error("configuration error: {0}")]
    Config(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = IngressError::ComputeBudgetExceeded {
            requested: 2_000_000,
            max: 1_400_000,
        };
        assert!(err.to_string().contains("2000000"));
        assert!(err.to_string().contains("1400000"));
    }

    #[test]
    fn test_pipeline_error_from() {
        let ingress_err = IngressError::EmptyInstruction;
        let pipeline_err: PipelineError = ingress_err.into();
        assert!(matches!(pipeline_err, PipelineError::Ingress(_)));
    }
}
