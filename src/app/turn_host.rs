use std::sync::Arc;

use hashring::HashRing;
use rand::{prelude::SliceRandom, thread_rng};
use serde::{Deserialize, Serialize};
use vec1::Vec1;

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
    fn new(hosts: &Vec1<TurnHost>) -> Self {
        let mut ring = HashRing::new();
        for h in hosts.iter().cloned() {
            ring.add(h);
        }

        Self { ring }
    }

    fn get(&self, key: &str) -> TurnHost {
        self.ring.get(&key).cloned().unwrap()
    }
}

struct RandomVec {
    hosts: Vec1<TurnHost>,
}

impl RandomVec {
    fn new(hosts: &Vec1<TurnHost>) -> Self {
        Self {
            hosts: hosts.clone(),
        }
    }

    fn get(&self) -> TurnHost {
        let mut rng = thread_rng();
        self.hosts.choose(&mut rng).cloned().unwrap()
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
    pub fn new(hosts: &Vec1<TurnHost>) -> Self {
        let random = RandomVec::new(hosts);
        let hash_ring = Ring::new(hosts);
        let inner = Arc::new(Inner { random, hash_ring });
        Self { inner }
    }

    pub fn get(&self, class: &Class) -> TurnHost {
        match class.kind() {
            ClassType::Webinar | ClassType::Minigroup => self.inner.random.get(),
            ClassType::P2P => self.inner.hash_ring.get(class.scope()),
        }
    }
}
