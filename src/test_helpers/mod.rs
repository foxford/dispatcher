pub const SVC_AUDIENCE: &'static str = "dev.svc.example.org";
pub const USR_AUDIENCE: &'static str = "dev.usr.example.com";
pub const TOKEN_ISSUER: &'static str = "iam.usr.example.com";

pub mod prelude {
    #[allow(unused_imports)]
    pub use super::{
        agent::TestAgent, authz::TestAuthz, db::TestDb, shared_helpers, state::TestState,
        SVC_AUDIENCE, USR_AUDIENCE,
    };
}

pub mod agent;
pub mod authz;
pub mod db;
pub mod shared_helpers;
pub mod state;
