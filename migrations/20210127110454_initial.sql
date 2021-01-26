-- Add migration script here
CREATE TABLE IF NOT EXISTS frontend (
    id BIGSERIAL PRIMARY KEY,
    url TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL
);

CREATE TABLE IF NOT EXISTS scope (
    id BIGSERIAL PRIMARY KEY,
    scope TEXT NOT NULL,
    frontend_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,

    FOREIGN KEY (frontend_id) REFERENCES frontend(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS unique_scope ON scope (scope);
