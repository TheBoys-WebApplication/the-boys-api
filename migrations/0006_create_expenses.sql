CREATE TABLE expenses (
    id          UUID             PRIMARY KEY DEFAULT gen_random_uuid(),
    trip_id     UUID             NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    paid_by     UUID             NOT NULL REFERENCES users(id),
    description TEXT             NOT NULL,
    amount      DOUBLE PRECISION NOT NULL CHECK (amount > 0),
    created_at  TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_expenses_trip    ON expenses (trip_id);
CREATE INDEX idx_expenses_paid_by ON expenses (paid_by);
