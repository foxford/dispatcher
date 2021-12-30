-- Add migration script here
CREATE TABLE IF NOT EXISTS record_timestamp (
    class_id uuid NOT NULL REFERENCES class,
    account_id account_id NOT NULL,
    position_secs INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (class_id, account_id)
);
