use std::error::Error as StdError;
use std::fmt;

use rand::{distributions::Alphanumeric, thread_rng, Rng};
use svc_agent::error::Error as AgentError;

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ClientError {
    Agent(AgentError),
    Payload(String),
    Timeout,
    Http(String),
}

impl From<AgentError> for ClientError {
    fn from(e: AgentError) -> Self {
        Self::Agent(e)
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Agent(ae) => write!(f, "Agent inner error: {}", ae),
            ClientError::Payload(s) => write!(f, "Payload error: {}", s),
            ClientError::Timeout => write!(f, "Timeout"),
            ClientError::Http(s) => write!(f, "Http error: {}", s),
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
