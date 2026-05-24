CREATE TABLE trips (
    id          UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id    UUID            NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    created_by  UUID            NOT NULL REFERENCES users(id),
    name        TEXT            NOT NULL,
    destination TEXT            NOT NULL,
    description TEXT,
    start_date  DATE,
    end_date    DATE,
    status      TEXT            NOT NULL DEFAULT 'planning'
                CHECK (status IN ('planning', 'upcoming', 'active', 'completed', 'cancelled')),
    created_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_trips_group ON trips (group_id);
CREATE INDEX idx_trips_created_by ON trips (created_by);
