CREATE TABLE attendance_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    session_id INTEGER NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('start', 'end', 'continue', 'edit', 'delete')),

    before_started_at INTEGER,
    before_ended_at INTEGER,
    before_note TEXT,
    before_deleted_at INTEGER,

    after_started_at INTEGER NOT NULL,
    after_ended_at INTEGER,
    after_note TEXT,
    after_deleted_at INTEGER,

    created_at INTEGER NOT NULL,
    reverted_at INTEGER,

    FOREIGN KEY (session_id) REFERENCES attendance_sessions (id)
);

CREATE INDEX attendance_operations_latest
ON attendance_operations (guild_id, user_id, id DESC)
WHERE reverted_at IS NULL;
