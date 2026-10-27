//! Job state vocabulary. Stored as text in PostgreSQL and serialised verbatim in the API.

use serde::{Deserialize, Serialize};

macro_rules! text_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($text => Some($name::$variant),)+ _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

text_enum!(JobStatus {
    Queued => "queued",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

text_enum!(GenerationStage {
    Queued => "queued",
    Preparing => "preparing",
    FetchingRepository => "fetching_repository",
    RunningAgent => "running_agent",
    ValidatingOutput => "validating_output",
    Completed => "completed",
});

impl GenerationStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Preparing => "Checking access and resolving branch",
            Self::FetchingRepository => "Fetching repository snapshot",
            Self::RunningAgent => "Running bounded-context agent",
            Self::ValidatingOutput => "Validating generated documentation",
            Self::Completed => "Completed",
        }
    }
}

text_enum!(PublishStatus {
    NotStarted => "not_started",
    Queued => "queued",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
});

impl PublishStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

text_enum!(PublishStage {
    Queued => "queued",
    CheckingBase => "checking_base",
    CreatingCommit => "creating_commit",
    CreatingBranch => "creating_branch",
    CreatingPullRequest => "creating_pull_request",
    CheckingCentralBase => "checking_central_base",
    CreatingCentralCommit => "creating_central_commit",
    CreatingCentralBranch => "creating_central_branch",
    CreatingCentralPullRequest => "creating_central_pull_request",
    Completed => "completed",
});

impl PublishStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::CheckingBase => "Checking base branch",
            Self::CreatingCommit => "Creating commit",
            Self::CreatingBranch => "Creating branch",
            Self::CreatingPullRequest => "Opening pull request",
            Self::CheckingCentralBase => "Checking central repository",
            Self::CreatingCentralCommit => "Creating central repository commit",
            Self::CreatingCentralBranch => "Creating central repository branch",
            Self::CreatingCentralPullRequest => "Opening central repository pull request",
            Self::Completed => "Completed",
        }
    }
}

text_enum!(StepState {
    Pending => "pending",
    Active => "active",
    Done => "done",
    Failed => "failed",
    Skipped => "skipped",
});

text_enum!(TaskKind {
    Generate => "generate",
    Publish => "publish",
});

text_enum!(TaskState {
    Pending => "pending",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});

text_enum!(ChangeType {
    Added => "added",
    Modified => "modified",
    Deleted => "deleted",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        for s in JobStatus::ALL {
            assert_eq!(JobStatus::parse(s.as_str()), Some(*s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        assert_eq!(GenerationStage::parse("running_agent"), Some(GenerationStage::RunningAgent));
        assert_eq!(PublishStatus::parse("not_started"), Some(PublishStatus::NotStarted));
        assert_eq!(TaskKind::parse("bogus"), None);
    }
}
