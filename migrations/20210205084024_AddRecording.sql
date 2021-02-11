-- Add migration script here
CREATE TABLE IF NOT EXISTS recording (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    class_id uuid NOT NULL,
    rtc_id uuid NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    segments INT8RANGE[] NOT NULL,
    modified_segments INT8RANGE[],
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
    adjusted_at TIMESTAMPTZ,
    transcoded_at TIMESTAMPTZ,

    FOREIGN KEY (class_id) REFERENCES class (id) ON DELETE CASCADE
);
