CREATE TABLE settlements (
    id       UUID             PRIMARY KEY DEFAULT gen_random_uuid(),
    trip_id  UUID             NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    paid_by  UUID             NOT NULL REFERENCES users(id),
    paid_to  UUID             NOT NULL REFERENCES users(id),
    amount   DOUBLE PRECISION NOT NULL CHECK (amount > 0),
    note     TEXT,
    created_at TIMESTAMPTZ    NOT NULL DEFAULT NOW(),
    CONSTRAINT settlements_different_users CHECK (paid_by <> paid_to)
);

CREATE INDEX idx_settlements_trip    ON settlements (trip_id);
CREATE INDEX idx_settlements_paid_by ON settlements (paid_by);
CREATE INDEX idx_settlements_paid_to ON settlements (paid_to);
