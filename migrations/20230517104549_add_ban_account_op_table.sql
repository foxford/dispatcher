CREATE TABLE IF NOT EXISTS ban_account_op (
    user_account account_id PRIMARY KEY,
    last_op_id uuid NOT NULL,
    video_complete boolean NOT NULL DEFAULT false,
    event_access_complete boolean NOT NULL DEFAULT false
);

CREATE SEQUENCE IF NOT EXISTS ban_entity_seq_id;
