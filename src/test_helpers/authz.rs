use svc_agent::AccountId;
use svc_authz::{
    Authenticable, ClientMap, Config, ConfigMap, IntentObject, LocalWhitelistConfig,
    LocalWhitelistRecord,
};

use crate::app::AuthzObject;
use crate::test_helpers::USR_AUDIENCE;

///////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Debug)]
pub struct TestAuthz {
    records: Vec<LocalWhitelistRecord>,
    audience: String,
}

impl TestAuthz {
    pub fn new() -> Self {
        Self {
            records: vec![],
            audience: USR_AUDIENCE.to_owned(),
        }
    }

    pub fn set_audience(&mut self, audience: &str) -> &mut Self {
        self.audience = audience.to_owned();
        self
    }

    pub fn allow<A: Authenticable>(&mut self, subject: &A, object: Vec<&str>, action: &str) {
        let object: Box<dyn IntentObject> = AuthzObject::new(&object).into();
        let record = LocalWhitelistRecord::new(subject, object, action);
        self.records.push(record);
    }
}

impl From<TestAuthz> for ClientMap {
    fn from(authz: TestAuthz) -> Self {
        let config = LocalWhitelistConfig::new(authz.records);

        let mut config_map = ConfigMap::new();
        config_map.insert(authz.audience.to_owned(), Config::LocalWhitelist(config));

        let account_id = AccountId::new("dispatcher", &authz.audience);

        ClientMap::new(&account_id, None, config_map, None).expect("Failed to build authz")
    }
}
