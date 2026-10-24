//! Agent integration: the agent bundle, the `AgentAdapter` contract, the GitHub Copilot CLI adapter, and the
//! workspace utilities (safe extraction, snapshots, output validation) the worker uses around a run.

pub mod adapter;
pub mod bundle;
pub mod copilot;
pub mod template;
pub mod workspace;

pub use adapter::{
    AgentAdapter, AgentDescriptor, AgentError, AgentRunOutcome, AgentRunRequest, LogLevel, ProgressSink, Secret,
};
pub use bundle::{AgentBundle, InputKind, InputSpec};
pub use copilot::CopilotCliAdapter;
