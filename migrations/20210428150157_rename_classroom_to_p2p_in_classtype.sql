UPDATE class SET kind = 'p2p' WHERE kind = 'classroom';

ALTER TYPE class_type RENAME TO class_type_old;

CREATE TYPE class_type AS ENUM('webinar', 'p2p', 'minigroup');

ALTER TABLE class DROP CONSTRAINT minigroup_host_present;

ALTER TABLE class ALTER COLUMN kind TYPE class_type USING kind::text::class_type;

DROP TYPE class_type_old;

ALTER TABLE class
ADD CONSTRAINT minigroup_host_present
CHECK (
    CASE kind
        WHEN 'minigroup' THEN host IS NOT NULL
        ELSE true
    END
);
