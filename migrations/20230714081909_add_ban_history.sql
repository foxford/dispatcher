CREATE TABLE IF NOT EXISTS ban_history (
    target_account account_id NOT NULL,
    class_id uuid NOT NULL,
    banned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    banned_operation_id BIGINT NOT NULL,
    unbanned_at TIMESTAMPTZ,
    unbanned_operation_id BIGINT,

    FOREIGN KEY (class_id) REFERENCES class (id) ON DELETE CASCADE,
    PRIMARY KEY (banned_operation_id)
);
