DROP TABLE IF EXISTS chat;

DELETE FROM class WHERE kind = 'chat';

ALTER TYPE class_type RENAME TO class_type_old;

ALTER TABLE class ALTER COLUMN conference_room_id SET NOT NULL;
ALTER TABLE class DROP CONSTRAINT conference_room_id_presence;

CREATE TYPE class_type AS ENUM('webinar', 'p2p', 'minigroup');
ALTER TABLE class ALTER COLUMN kind TYPE class_type USING (
    CASE kind::text
    WHEN 'webinar'      THEN 'webinar'::text::class_type
    WHEN 'p2p'          THEN 'p2p'::text::class_type
    WHEN 'minigroup'    THEN 'minigroup'::text::class_type
    WHEN 'chat'         THEN 'p2p'::text::class_type -- were deleted before so thats ok
    ELSE 'p2p'
    END
);

DROP TYPE class_type_old;
