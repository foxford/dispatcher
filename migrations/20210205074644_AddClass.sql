CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TYPE class_type as ENUM ('webinar');

CREATE TABLE IF NOT EXISTS class (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    kind class_type NOT NULL,
    audience text NOT NULL,
    scope text NOT NULL,
    "time" tstzrange NOT NULL,
    tags json,
    conference_room_id uuid NOT NULL,
    event_room_id uuid NOT NULL,
    original_event_room_id uuid,
    modified_event_room_id uuid,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    preserve_history BOOLEAN NOT NULL DEFAULT TRUE,

    PRIMARY KEY (id)
);
