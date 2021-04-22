-- Add migration script here
ALTER TABLE recording ADD COLUMN deleted_at TIMESTAMPTZ;

CREATE UNIQUE INDEX uniq_recording_per_class ON recording (class_id) WHERE deleted_at IS NULL;
