-- Add migration script here
CREATE TABLE IF NOT EXISTS chat (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    audience text NOT NULL,
    scope text NOT NULL,
    tags json,
    event_room_id uuid NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,

    PRIMARY KEY (id)
);
