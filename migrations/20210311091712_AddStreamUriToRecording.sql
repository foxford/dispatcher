-- Add migration script here
ALTER TABLE recording ADD COLUMN stream_uri TEXT NOT NULL;
