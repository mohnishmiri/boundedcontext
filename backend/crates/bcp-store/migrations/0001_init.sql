-- Bounded Context Portal — initial schema. Migrations are forward-only; never edit an applied file.

-- ---------------------------------------------------------------------------------------------------------
-- Identity
-- ---------------------------------------------------------------------------------------------------------
CREATE TABLE users (
    id              uuid PRIMARY KEY,
    github_id       bigint      NOT NULL UNIQUE,
    login           text        NOT NULL,
    name            text,
    avatar_url      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    last_login_at   timestamptz
);

-- GitHub App user-to-server tokens, AES-256-GCM encrypted by the application (key in Key Vault, not here).
-- The associated data binds each ciphertext to its user id, so rows cannot be swapped between users.
CREATE TABLE github_tokens (
    user_id             uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    access_token        bytea       NOT NULL,
    access_expires_at   timestamptz,
    refresh_token       bytea,
    refresh_expires_at  timestamptz,
    updated_at          timestamptz NOT NULL DEFAULT now()
);

-- Only a SHA-256 of the session id is stored; the id itself lives in the browser's HttpOnly cookie.
CREATE TABLE sessions (
    id_hash         bytea PRIMARY KEY,
    user_id         uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at      timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL,
    last_seen_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sessions_user_idx ON sessions (user_id);
CREATE INDEX sessions_expires_idx ON sessions (expires_at);

-- ---------------------------------------------------------------------------------------------------------
-- Jobs
-- ---------------------------------------------------------------------------------------------------------
CREATE TABLE jobs (
    id                      uuid PRIMARY KEY,
    user_id                 uuid        NOT NULL REFERENCES users (id),
    owner                   text        NOT NULL,
    repo                    text        NOT NULL,
    repo_id                 bigint,
    installation_id         bigint,
    base_branch             text        NOT NULL,
    model                   text        NOT NULL,
    inputs                  jsonb       NOT NULL DEFAULT '{}'::jsonb,
    -- Whether the requester had push permission when the job was created (re-checked before publishing).
    can_push                boolean     NOT NULL DEFAULT false,
    retry_of                uuid REFERENCES jobs (id),

    status                  text        NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    stage                   text        NOT NULL DEFAULT 'queued'
        CHECK (stage IN ('queued', 'preparing', 'fetching_repository', 'running_agent', 'validating_output', 'completed')),
    -- {"<stage>": {"started_at": "...", "finished_at": "...", "state": "done|failed|..."}}
    stage_times             jsonb       NOT NULL DEFAULT '{}'::jsonb,
    base_sha                text,
    agent_id                text        NOT NULL,
    agent_version           text        NOT NULL,
    agent_placeholder       boolean     NOT NULL DEFAULT false,
    cli_version             text,
    error_code              text,
    error_message           text,
    error_retryable         boolean,
    cancel_requested        boolean     NOT NULL DEFAULT false,
    output_primary          text,

    publish_status          text        NOT NULL DEFAULT 'not_started'
        CHECK (publish_status IN ('not_started', 'queued', 'running', 'succeeded', 'failed')),
    publish_stage           text
        CHECK (publish_stage IN ('queued', 'checking_base', 'creating_commit', 'creating_branch', 'creating_pull_request', 'completed')),
    publish_stage_times     jsonb       NOT NULL DEFAULT '{}'::jsonb,
    publish_requested_by    uuid REFERENCES users (id),
    publish_branch          text,
    publish_commit_sha      text,
    publish_pr_number       integer,
    publish_pr_url          text,
    publish_error_code      text,
    publish_error_message   text,
    publish_error_retryable boolean,
    publish_updated_at      timestamptz,

    created_at              timestamptz NOT NULL DEFAULT now(),
    started_at              timestamptz,
    finished_at             timestamptz,
    updated_at              timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX jobs_user_created_idx ON jobs (user_id, created_at DESC);
CREATE INDEX jobs_user_status_idx ON jobs (user_id, status);
-- At most one active generation per user / repository / branch / model.
CREATE UNIQUE INDEX jobs_active_unique
    ON jobs (user_id, lower(owner), lower(repo), base_branch, model)
    WHERE status IN ('queued', 'running');

-- Files the agent produced. Rows with allowed = false are "unexpected changes": recorded (path only) for the
-- review screen and never committed.
CREATE TABLE job_artifacts (
    job_id          uuid        NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    path            text        NOT NULL,
    change_type     text        NOT NULL CHECK (change_type IN ('added', 'modified', 'deleted')),
    allowed         boolean     NOT NULL,
    base_content    text,
    new_content     text,
    unified_diff    text,
    size_bytes      bigint      NOT NULL DEFAULT 0,
    sha256          text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, path),
    CHECK (NOT allowed OR (new_content IS NOT NULL AND change_type <> 'deleted'))
);

-- Redacted log lines and stage transitions. seq is global (bigserial): monotonic per job, with gaps.
CREATE TABLE job_events (
    seq             bigserial PRIMARY KEY,
    job_id          uuid        NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    ts              timestamptz NOT NULL DEFAULT now(),
    level           text        NOT NULL CHECK (level IN ('debug', 'info', 'warn', 'error')),
    stage           text        NOT NULL,
    message         text        NOT NULL
);
CREATE INDEX job_events_job_seq_idx ON job_events (job_id, seq);

-- ---------------------------------------------------------------------------------------------------------
-- Task queue (claimed with FOR UPDATE SKIP LOCKED, leased with a heartbeat)
-- ---------------------------------------------------------------------------------------------------------
CREATE TABLE job_tasks (
    id              uuid PRIMARY KEY,
    job_id          uuid        NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    kind            text        NOT NULL CHECK (kind IN ('generate', 'publish')),
    state           text        NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'running', 'succeeded', 'failed', 'cancelled')),
    attempts        integer     NOT NULL DEFAULT 0,
    max_attempts    integer     NOT NULL CHECK (max_attempts >= 1),
    run_after       timestamptz NOT NULL DEFAULT now(),
    lease_owner     text,
    lease_until     timestamptz,
    last_error      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX job_tasks_pending_idx ON job_tasks (run_after) WHERE state = 'pending';
CREATE INDEX job_tasks_running_lease_idx ON job_tasks (lease_until) WHERE state = 'running';
CREATE UNIQUE INDEX job_tasks_generate_unique ON job_tasks (job_id) WHERE kind = 'generate';
CREATE UNIQUE INDEX job_tasks_publish_active_unique
    ON job_tasks (job_id) WHERE kind = 'publish' AND state IN ('pending', 'running');

-- ---------------------------------------------------------------------------------------------------------
-- Audit trail (append-only)
-- ---------------------------------------------------------------------------------------------------------
CREATE TABLE audit_events (
    id              bigserial PRIMARY KEY,
    ts              timestamptz NOT NULL DEFAULT now(),
    actor_user_id   uuid REFERENCES users (id),
    actor_login     text,
    action          text        NOT NULL,
    job_id          uuid,
    owner           text,
    repo            text,
    details         jsonb       NOT NULL DEFAULT '{}'::jsonb,
    request_id      text
);
CREATE INDEX audit_events_ts_idx ON audit_events (ts);
CREATE INDEX audit_events_job_idx ON audit_events (job_id);

CREATE FUNCTION audit_events_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_events is append-only';
END;
$$;
CREATE TRIGGER audit_events_no_update BEFORE UPDATE OR DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_append_only();

-- ---------------------------------------------------------------------------------------------------------
-- Notifications: wake idle workers and push progress to SSE subscribers. Payloads carry ids only.
-- ---------------------------------------------------------------------------------------------------------
CREATE FUNCTION notify_task() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.state = 'pending' THEN
        PERFORM pg_notify('bcp_tasks', NEW.kind);
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER job_tasks_notify AFTER INSERT OR UPDATE OF state, run_after ON job_tasks
    FOR EACH ROW EXECUTE FUNCTION notify_task();

CREATE FUNCTION notify_job() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('bcp_jobs', NEW.id::text);
    RETURN NEW;
END;
$$;
CREATE TRIGGER jobs_notify AFTER UPDATE ON jobs
    FOR EACH ROW EXECUTE FUNCTION notify_job();

CREATE FUNCTION notify_job_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('bcp_jobs', NEW.job_id::text);
    RETURN NEW;
END;
$$;
CREATE TRIGGER job_events_notify AFTER INSERT ON job_events
    FOR EACH ROW EXECUTE FUNCTION notify_job_event();
