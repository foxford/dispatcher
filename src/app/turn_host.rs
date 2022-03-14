use std::sync::Arc;

use hashring::HashRing;
use rand::{prelude::SliceRandom, thread_rng};
use serde::{Deserialize, Serialize};

use crate::db::class::{ClassType, Object as Class};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct TurnHost(Arc<str>);

#[cfg(test)]
impl From<&str> for TurnHost {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

struct Ring {
    ring: HashRing<TurnHost>,
}

impl Ring {
    fn new(hosts: &[TurnHost]) -> Self {
        let mut ring = HashRing::new();
        for h in hosts.iter().cloned() {
            ring.add(h);
        }

        Self { ring }
    }

    fn get(&self, key: &str) -> Option<TurnHost> {
        self.ring.get(&key).cloned()
    }
}

struct RandomVec {
    hosts: Vec<TurnHost>,
}

impl RandomVec {
    fn new(hosts: &[TurnHost]) -> Self {
        Self {
            hosts: hosts.to_vec(),
        }
    }

    fn get(&self) -> Option<TurnHost> {
        let mut rng = thread_rng();
        self.hosts.choose(&mut rng).cloned()
    }
}

struct Inner {
    random: RandomVec,
    hash_ring: Ring,
}

#[derive(Clone)]
pub struct TurnHostSelector {
    inner: Arc<Inner>,
}

impl TurnHostSelector {
    pub fn new(hosts: &[TurnHost]) -> Self {
        let random = RandomVec::new(hosts);
        let hash_ring = Ring::new(hosts);
        let inner = Arc::new(Inner { random, hash_ring });
        Self { inner }
    }

    pub fn get(&self, class: &Class) -> Option<TurnHost> {
        match class.kind() {
            ClassType::Webinar | ClassType::Minigroup => self.inner.random.get(),
            ClassType::P2P => self.inner.hash_ring.get(class.scope()),
        }
    }
}
