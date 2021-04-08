CREATE TYPE agent_id AS (
    account_id account_id,
    label TEXT
);

ALTER TABLE recording ADD COLUMN created_by agent_id NULL;
UPDATE recording SET created_by = '("(anonymous,example.com)",web)'::agent_id;
ALTER TABLE recording ALTER COLUMN created_by SET NOT NULL;
