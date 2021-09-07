-- Add migration script here
ALTER TABLE class ADD COLUMN timed_out boolean not null default false;
