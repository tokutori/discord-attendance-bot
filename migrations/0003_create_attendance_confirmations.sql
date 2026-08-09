CREATE TABLE attendance_confirmations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    code TEXT NOT NULL UNIQUE,
    action TEXT NOT NULL CHECK (action IN ('edit', 'delete', 'revert')),
    session_id INTEGER NOT NULL,
    operation_id INTEGER,

    expected_started_at INTEGER NOT NULL,
    expected_ended_at INTEGER,
    expected_note TEXT,
    expected_deleted_at INTEGER,

    target_started_at INTEGER,
    target_ended_at INTEGER,
    target_note TEXT,

    requested_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER,

    FOREIGN KEY (session_id) REFERENCES attendance_sessions (id),
    FOREIGN KEY (operation_id) REFERENCES attendance_operations (id)
);

CREATE INDEX attendance_confirmations_owner
ON attendance_confirmations (guild_id, user_id, expires_at)
WHERE consumed_at IS NULL;
