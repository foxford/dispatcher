pub const SVC_AUDIENCE: &'static str = "dev.svc.example.org";
pub const USR_AUDIENCE: &'static str = "dev.usr.example.com";
pub const TOKEN_ISSUER: &'static str = "iam.svc.example.com";
pub const PUBKEY_PATH: &str = "data/keys/svc.public_key.p8.der.sample";

pub mod prelude {
    #[allow(unused_imports)]
    pub use super::{
        agent::TestAgent, authz::TestAuthz, db::TestDb, factory,
        outgoing_envelope::OutgoingEnvelopeProperties, shared_helpers, state::TestState,
        SVC_AUDIENCE, USR_AUDIENCE,
    };

    pub use shared_helpers::*;

    pub use chrono::Utc;
    pub use std::ops::Bound;
    pub use std::time::Duration;
    pub use uuid::*;
}

pub mod agent;
pub mod authz;
pub mod db;
pub mod factory;
pub mod outgoing_envelope;
pub mod shared_helpers;
pub mod state;
