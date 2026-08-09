CREATE TABLE attendance_auto_ends (
    session_id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    automatic_ended_at INTEGER NOT NULL,
    applied_at INTEGER NOT NULL,
    notified_at INTEGER,
    corrected_at INTEGER,
    FOREIGN KEY (session_id) REFERENCES attendance_sessions (id)
);

CREATE INDEX attendance_auto_ends_pending_notice
ON attendance_auto_ends (guild_id, user_id, notified_at, applied_at DESC);
