use svc_agent::mqtt::Address;
use svc_agent::{AccountId, AgentId, Authenticable};
use svc_authn::{jose::Algorithm, token::jws_compact::TokenBuilder};

use super::TOKEN_ISSUER;
use crate::app::API_VERSION;

const TOKEN_EXPIRATION: i64 = 600;
const KEY_PATH: &str = "data/keys/svc.private_key.p8.der.sample";

lazy_static! {
    static ref PRIVATE_KEY: Vec<u8> =
        std::fs::read(KEY_PATH).expect("Failed to read private key file");
}

#[derive(Debug, Clone)]
pub struct TestAgent {
    address: Address,
}

impl TestAgent {
    pub fn new(agent_label: &str, account_label: &str, audience: &str) -> Self {
        let account_id = AccountId::new(account_label, audience);
        let agent_id = AgentId::new(agent_label, account_id.clone());
        let address = Address::new(agent_id, API_VERSION);
        Self { address }
    }

    pub fn address(&self) -> &Address {
        &self.address
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.address.id()
    }

    pub fn account_id(&self) -> &AccountId {
        &self.address.id().as_account_id()
    }

    pub fn token(&self) -> String {
        TokenBuilder::new()
            .issuer(TOKEN_ISSUER)
            .subject(self.account_id())
            .expires_in(TOKEN_EXPIRATION)
            .key(Algorithm::ES256, PRIVATE_KEY.as_slice())
            .build()
            .expect("Failed to build access token")
    }
}
