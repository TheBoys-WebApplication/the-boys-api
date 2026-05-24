CREATE TABLE activities (
    id             UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    trip_id        UUID            NOT NULL REFERENCES trips(id) ON DELETE CASCADE,
    suggested_by   UUID            NOT NULL REFERENCES users(id),
    name           TEXT            NOT NULL,
    description    TEXT,
    location       TEXT,
    activity_date  TIMESTAMPTZ,
    estimated_cost DOUBLE PRECISION,
    status         TEXT            NOT NULL DEFAULT 'idea'
                   CHECK (status IN ('idea', 'confirmed', 'done')),
    created_at     TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_activities_trip ON activities (trip_id);
CREATE INDEX idx_activities_suggested_by ON activities (suggested_by);
