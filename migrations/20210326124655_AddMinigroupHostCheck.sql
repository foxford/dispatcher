-- Add migration script here
DO $$ BEGIN
    CREATE TYPE account_id AS (
        label text,
        audience text
    );
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

ALTER TABLE class ADD COLUMN host account_id;

ALTER TABLE class
ADD CONSTRAINT minigroup_host_present
CHECK (
	CASE kind
        WHEN 'minigroup' THEN host IS NOT NULL
        ELSE true
    END
);

