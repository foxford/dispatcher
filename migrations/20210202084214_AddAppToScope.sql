-- Add migration script here
DROP INDEX IF EXISTS unique_scope;

ALTER TABLE scope
ADD COLUMN app TEXT NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS unique_scope_app ON scope (scope, app);
