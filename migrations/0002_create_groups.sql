CREATE TABLE groups (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT        NOT NULL,
    description TEXT,
    leader_id   UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invite_code TEXT        NOT NULL UNIQUE DEFAULT encode(gen_random_bytes(6), 'hex'),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_groups_leader ON groups (leader_id);

CREATE TABLE group_members (
    id        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id  UUID        NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id   UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role      TEXT        NOT NULL DEFAULT 'member' CHECK (role IN ('leader', 'member')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (group_id, user_id)
);

CREATE INDEX idx_group_members_group ON group_members (group_id);
CREATE INDEX idx_group_members_user  ON group_members (user_id);
