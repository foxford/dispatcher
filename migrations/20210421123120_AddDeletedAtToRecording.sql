-- Add migration script here
ALTER TABLE recording ADD COLUMN deleted_at TIMESTAMPTZ;
