ALTER TABLE class ADD COLUMN established BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE class
ADD CONSTRAINT rooms_ids_presence
CHECK (
    CASE WHEN established THEN (conference_room_id IS NOT NULL) AND (event_room_id IS NOT NULL)
        ELSE true
    END
);

ALTER TABLE class ALTER COLUMN conference_room_id DROP NOT NULL;
ALTER TABLE class ALTER COLUMN event_room_id DROP NOT NULL;
