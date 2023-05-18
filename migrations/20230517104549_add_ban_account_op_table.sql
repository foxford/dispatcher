CREATE TABLE IF NOT EXISTS ban_account_op (
    user_account account_id PRIMARY KEY,
    last_op_id uuid NOT NULL,
    last_op_done boolean NOT NULL DEFAULT false
);
