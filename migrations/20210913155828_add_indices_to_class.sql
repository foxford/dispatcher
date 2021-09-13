-- Add migration script here

CREATE INDEX class_conference_room_id ON class (conference_room_id);
CREATE INDEX class_event_room_id ON class (event_room_id);
CREATE INDEX class_original_event_room_id ON class (original_event_room_id);
CREATE INDEX class_midified_event_room_id ON class (modified_event_room_id);
