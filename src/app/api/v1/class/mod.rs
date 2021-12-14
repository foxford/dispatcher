use crate::db::class;

use super::{find, find_by_scope, find_class_by_scope, validate_token, AppResult};

pub use commit_edition::commit_edition;
pub use read::{read, read_by_scope};
pub use recreate::recreate;
use serde::Serialize;
use serde_json::Value;
pub use update::{update, update_by_scope};
use uuid::Uuid;

mod commit_edition;
mod read;
mod recreate;
mod update;

#[derive(Serialize)]
struct ClassResponseBody {
    id: String,
    real_time: RealTimeObject,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    on_demand: Vec<ClassroomVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<ClassStatus>,
    timed_out: bool,
}

impl ClassResponseBody {
    pub fn add_version(&mut self, version: ClassroomVersion) {
        self.on_demand.push(version);
    }

    pub fn set_status(&mut self, status: ClassStatus) {
        self.status = Some(status);
    }

    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.real_time.set_rtc_id(rtc_id);
    }
}

impl From<&class::Object> for ClassResponseBody {
    fn from(obj: &class::Object) -> Self {
        Self {
            id: obj.scope().to_owned(),
            real_time: RealTimeObject {
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
                rtc_id: None,
                fallback_uri: None,
            },
            on_demand: vec![],
            status: None,
            timed_out: obj.timed_out(),
        }
    }
}

#[derive(Serialize)]
pub struct ClassroomVersion {
    version: &'static str,
    event_room_id: Uuid,
    // TODO: this is deprecated and should be removed eventually
    // right now its necessary to generate HLS links
    stream_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    room_events_uri: Option<String>,
}

#[derive(Serialize)]
pub struct RealTimeObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    conference_room_id: Option<Uuid>,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtc_id: Option<Uuid>,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum ClassStatus {
    Transcoded,
    Adjusted,
    Finished,
    RealTime,
    Closed,
}

impl RealTimeObject {
    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.rtc_id = Some(rtc_id);
    }
}
