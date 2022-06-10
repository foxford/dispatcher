-- Add migration script here
ALTER TABLE class ADD COLUMN properties jsonb;
