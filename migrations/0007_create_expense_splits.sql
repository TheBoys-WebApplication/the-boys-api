CREATE TABLE expense_splits (
    id          UUID             PRIMARY KEY DEFAULT gen_random_uuid(),
    expense_id  UUID             NOT NULL REFERENCES expenses(id) ON DELETE CASCADE,
    user_id     UUID             NOT NULL REFERENCES users(id),
    amount      DOUBLE PRECISION NOT NULL CHECK (amount >= 0),
    created_at  TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    UNIQUE (expense_id, user_id)
);

CREATE INDEX idx_expense_splits_expense ON expense_splits (expense_id);
CREATE INDEX idx_expense_splits_user    ON expense_splits (user_id);
