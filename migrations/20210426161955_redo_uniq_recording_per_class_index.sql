-- Add migration script here

DROP INDEX IF EXISTS uniq_recording_per_class;

CREATE UNIQUE INDEX uniq_recording_per_class ON recording (class_id, created_by) WHERE deleted_at IS NULL;
