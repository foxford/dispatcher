use uuid::Uuid;

use crate::db::class;

use super::{find, find_by_scope, find_class_by_scope, AppResult};

pub use commit_edition::commit_edition;
pub use create_timestamp::create_timestamp;
pub use read::{read, read_by_scope};
pub use recreate::recreate;
use serde::Serialize;
use serde_json::Value;
pub use update::{update, update_by_scope};

mod commit_edition;
mod create_timestamp;
mod read;
mod recreate;
mod update;

#[derive(Serialize)]
struct ClassResponseBody {
    class_id: Uuid,
    id: String,
    real_time: RealTimeObject,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    on_demand: Vec<ClassroomVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<ClassStatus>,
    timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<i32>,
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

    pub fn set_position(&mut self, position_secs: i32) {
        self.position = Some(position_secs);
    }
}

impl From<&class::Object> for ClassResponseBody {
    fn from(obj: &class::Object) -> Self {
        Self {
            class_id: obj.id(),
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
            position: None,
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
    conference_room_id: Uuid,
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
