CREATE TABLE IF NOT EXISTS account (
    id account_id PRIMARY KEY,
    properties jsonb NOT NULL DEFAULT '{}'::jsonb
);
