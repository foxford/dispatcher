CREATE TABLE IF NOT EXISTS ban_account_op (
    user_account account_id PRIMARY KEY,
    last_op_id bigint NOT NULL,
    is_video_streaming_banned boolean NOT NULL DEFAULT false,
    is_collaboration_banned boolean NOT NULL DEFAULT false
);

CREATE SEQUENCE IF NOT EXISTS ban_entity_seq_id;
