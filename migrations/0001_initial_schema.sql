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
    CHECK (ended_at IS NULL OR ended_at >= started_at),
    CHECK (length(display_name) BETWEEN 1 AND 100),
    CHECK (note IS NULL OR length(note) <= 500)
);

CREATE UNIQUE INDEX attendance_sessions_one_open_per_user
ON attendance_sessions (guild_id, user_id)
WHERE ended_at IS NULL AND deleted_at IS NULL;

CREATE INDEX attendance_sessions_user_history
ON attendance_sessions (guild_id, user_id, started_at DESC)
WHERE deleted_at IS NULL;

CREATE INDEX attendance_sessions_guild_period
ON attendance_sessions (guild_id, started_at, ended_at)
WHERE deleted_at IS NULL;

CREATE TABLE attendance_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    session_id INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('start', 'end', 'continue', 'edit', 'delete')),

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

    FOREIGN KEY (session_id) REFERENCES attendance_sessions (id) ON DELETE CASCADE
);

CREATE INDEX attendance_changes_latest_revertible
ON attendance_changes (guild_id, user_id, id DESC)
WHERE reverted_at IS NULL;

CREATE TABLE pending_attendance_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    code TEXT NOT NULL UNIQUE,
    action TEXT NOT NULL CHECK (action IN ('edit', 'delete', 'revert')),
    session_id INTEGER NOT NULL,
    change_id INTEGER,

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

    FOREIGN KEY (session_id) REFERENCES attendance_sessions (id) ON DELETE CASCADE,
    FOREIGN KEY (change_id) REFERENCES attendance_changes (id) ON DELETE SET NULL
);

CREATE INDEX pending_attendance_actions_owner
ON pending_attendance_actions (guild_id, user_id, expires_at)
WHERE consumed_at IS NULL;

CREATE TABLE attendance_auto_end_events (
    session_id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    automatic_ended_at INTEGER NOT NULL,
    applied_at INTEGER NOT NULL,
    notified_at INTEGER,
    corrected_at INTEGER,
    FOREIGN KEY (session_id) REFERENCES attendance_sessions (id) ON DELETE CASCADE
);

CREATE INDEX attendance_auto_end_events_pending_notice
ON attendance_auto_end_events (guild_id, user_id, notified_at, applied_at DESC);

CREATE TABLE attendance_user_profiles (
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    generation INTEGER,
    real_name TEXT,
    role TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (guild_id, user_id),
    CHECK (real_name IS NULL OR length(real_name) BETWEEN 1 AND 100),
    CHECK (role IS NULL OR length(role) BETWEEN 1 AND 100)
);

CREATE INDEX attendance_user_profiles_export_order
ON attendance_user_profiles (guild_id, generation, real_name, user_id);
