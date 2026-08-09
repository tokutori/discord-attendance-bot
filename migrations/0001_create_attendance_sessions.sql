CREATE TABLE attendance_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    display_name TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    CHECK (ended_at IS NULL OR ended_at >= started_at)
);

CREATE UNIQUE INDEX one_open_session_per_user
ON attendance_sessions (guild_id, user_id)
WHERE ended_at IS NULL AND deleted_at IS NULL;

CREATE INDEX attendance_sessions_user_history
ON attendance_sessions (guild_id, user_id, started_at DESC)
WHERE deleted_at IS NULL;
