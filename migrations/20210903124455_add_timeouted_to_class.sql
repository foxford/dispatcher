-- Add migration script here
ALTER TABLE class ADD COLUMN timeouted boolean not null default false;
