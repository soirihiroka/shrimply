use hashbrown::HashMap;
use shrimply_project::project::{ManimParameter, ManimParameterValue};
use uuid::Uuid;

#[derive(Clone)]
pub struct SourceIdentity {
    pub item_id: Uuid,
    pub source_revision: u64,
    pub scene: String,
    pub input_parameters: HashMap<String, ManimParameterValue>,
}

impl SourceIdentity {
    pub fn duration(&self, duration: shrimply_math_core::Time) -> Update {
        Update::Duration {
            source: self.clone(),
            duration,
        }
    }

    pub fn parameters(&self, parameters: Vec<ManimParameter>, render_is_current: bool) -> Update {
        Update::Parameters {
            source: self.clone(),
            parameters,
            render_is_current,
        }
    }

    pub fn error(&self, error: Option<String>) -> Update {
        Update::Error {
            source: self.clone(),
            error,
        }
    }
}

#[derive(Clone)]
pub enum Update {
    Duration {
        source: SourceIdentity,
        duration: shrimply_math_core::Time,
    },
    Parameters {
        source: SourceIdentity,
        parameters: Vec<ManimParameter>,
        render_is_current: bool,
    },
    Error {
        source: SourceIdentity,
        error: Option<String>,
    },
}
