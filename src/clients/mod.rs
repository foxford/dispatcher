use std::error::Error as StdError;
use std::fmt;

use rand::{distributions::Alphanumeric, thread_rng, Rng};
use svc_agent::error::Error as AgentError;

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ClientError {
    AgentError(AgentError),
    PayloadError(String),
    TimeoutError,
    HttpError(String),
}

impl From<AgentError> for ClientError {
    fn from(e: AgentError) -> Self {
        Self::AgentError(e)
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::AgentError(ae) => write!(f, "Agent inner error: {}", ae),
            ClientError::PayloadError(s) => write!(f, "Payload error: {}", s),
            ClientError::TimeoutError => write!(f, "Timeout"),
            ClientError::HttpError(s) => write!(f, "Http error: {}", s),
        }
    }
}

impl StdError for ClientError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

////////////////////////////////////////////////////////////////////////////////

const CORRELATION_DATA_LENGTH: usize = 16;

fn generate_correlation_data() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(CORRELATION_DATA_LENGTH)
        .collect()
}

////////////////////////////////////////////////////////////////////////////////

pub mod conference;
pub mod event;
pub mod tq;
