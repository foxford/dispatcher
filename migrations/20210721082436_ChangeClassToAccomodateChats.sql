ALTER TABLE class
ALTER COLUMN conference_room_id DROP NOT NULL;

ALTER TABLE class
ADD CONSTRAINT conference_room_id_presence
CHECK (
    CASE WHEN kind IN ('webinar', 'p2p', 'minigroup') THEN conference_room_id IS NOT NULL
        ELSE true
    END
);


INSERT INTO class (kind, audience, scope, time, tags, event_room_id, created_at)
(
    SELECT 'chat', audience, scope, TSTZRANGE(NULL, NULL), tags, event_room_id, created_at
    FROM chat
);
