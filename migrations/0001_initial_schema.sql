CREATE TABLE attendance_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    display_name TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    open_since INTEGER,
    note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    CHECK (ended_at IS NULL OR ended_at >= started_at),
    CHECK (
        (ended_at IS NULL AND open_since IS NOT NULL AND open_since >= started_at)
        OR (ended_at IS NOT NULL AND open_since IS NULL)
    ),
    CHECK (length(display_name) BETWEEN 1 AND 100),
    CHECK (note IS NULL OR length(note) <= 500),
    UNIQUE (id, guild_id, user_id)
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

CREATE TRIGGER attendance_sessions_prevent_overlap_insert
BEFORE INSERT ON attendance_sessions
WHEN NEW.deleted_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'attendance session overlaps an existing session')
    WHERE EXISTS (
        SELECT 1 FROM attendance_sessions existing
        WHERE existing.guild_id = NEW.guild_id
          AND existing.user_id = NEW.user_id
          AND existing.deleted_at IS NULL
          AND existing.started_at < COALESCE(NEW.ended_at, 9223372036854775807)
          AND COALESCE(existing.ended_at, 9223372036854775807) > NEW.started_at
    );
END;

CREATE TRIGGER attendance_sessions_prevent_overlap_update
BEFORE UPDATE OF started_at, ended_at, deleted_at ON attendance_sessions
WHEN NEW.deleted_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'attendance session overlaps an existing session')
    WHERE EXISTS (
        SELECT 1 FROM attendance_sessions existing
        WHERE existing.guild_id = NEW.guild_id
          AND existing.user_id = NEW.user_id
          AND existing.id != NEW.id
          AND existing.deleted_at IS NULL
          AND existing.started_at < COALESCE(NEW.ended_at, 9223372036854775807)
          AND COALESCE(existing.ended_at, 9223372036854775807) > NEW.started_at
    );
END;

CREATE TABLE attendance_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    session_id INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('start', 'end', 'continue', 'edit', 'delete')),

    before_started_at INTEGER,
    before_ended_at INTEGER,
    before_open_since INTEGER,
    before_note TEXT,
    before_deleted_at INTEGER,

    after_started_at INTEGER NOT NULL,
    after_ended_at INTEGER,
    after_open_since INTEGER,
    after_note TEXT,
    after_deleted_at INTEGER,

    created_at INTEGER NOT NULL,
    reverted_at INTEGER,

    CHECK (
        (kind = 'start' AND before_started_at IS NULL)
        OR (kind != 'start' AND before_started_at IS NOT NULL
            AND ((before_ended_at IS NULL AND before_open_since IS NOT NULL
                    AND before_open_since >= before_started_at)
                OR (before_ended_at IS NOT NULL AND before_open_since IS NULL)))
    ),
    CHECK (
        (after_ended_at IS NULL AND after_open_since IS NOT NULL
            AND after_open_since >= after_started_at)
        OR (after_ended_at IS NOT NULL AND after_open_since IS NULL)
    ),
    UNIQUE (id, session_id, guild_id, user_id),
    FOREIGN KEY (session_id, guild_id, user_id)
        REFERENCES attendance_sessions (id, guild_id, user_id) ON DELETE CASCADE
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
    expected_open_since INTEGER,
    expected_note TEXT,
    expected_deleted_at INTEGER,

    target_started_at INTEGER,
    target_ended_at INTEGER,
    target_note TEXT,

    requested_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER,

    CHECK (expires_at > requested_at),
    CHECK (
        (expected_ended_at IS NULL AND expected_open_since IS NOT NULL
            AND expected_open_since >= expected_started_at)
        OR (expected_ended_at IS NOT NULL AND expected_open_since IS NULL)
    ),
    CHECK (
        (action = 'edit' AND change_id IS NULL AND target_started_at IS NOT NULL)
        OR (action = 'delete' AND change_id IS NULL
            AND target_started_at IS NULL AND target_ended_at IS NULL AND target_note IS NULL)
        OR (action = 'revert' AND change_id IS NOT NULL
            AND target_started_at IS NULL AND target_ended_at IS NULL AND target_note IS NULL)
    ),
    FOREIGN KEY (session_id, guild_id, user_id)
        REFERENCES attendance_sessions (id, guild_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (change_id, session_id, guild_id, user_id)
        REFERENCES attendance_changes (id, session_id, guild_id, user_id) ON DELETE CASCADE
);

CREATE INDEX pending_attendance_actions_owner
ON pending_attendance_actions (guild_id, user_id, expires_at)
WHERE consumed_at IS NULL;

CREATE TABLE attendance_auto_end_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    automatic_ended_at INTEGER NOT NULL,
    applied_at INTEGER NOT NULL,
    change_id_at_application INTEGER NOT NULL CHECK (change_id_at_application >= 0),
    notified_at INTEGER,
    corrected_at INTEGER,
    FOREIGN KEY (session_id, guild_id, user_id)
        REFERENCES attendance_sessions (id, guild_id, user_id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX attendance_auto_end_events_one_active
ON attendance_auto_end_events (session_id)
WHERE corrected_at IS NULL;

CREATE INDEX attendance_auto_end_events_pending_notice
ON attendance_auto_end_events (guild_id, user_id, notified_at, applied_at DESC);

CREATE TABLE attendance_user_profiles (
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    generation INTEGER,
    real_name TEXT,
    role TEXT,
    name_reading TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (guild_id, user_id),
    CHECK (real_name IS NULL OR length(real_name) BETWEEN 1 AND 100),
    CHECK (role IS NULL OR length(role) BETWEEN 1 AND 100),
    CHECK (name_reading IS NULL OR length(name_reading) BETWEEN 1 AND 100)
);

CREATE INDEX attendance_user_profiles_export_order
ON attendance_user_profiles (guild_id, generation, real_name, user_id);
