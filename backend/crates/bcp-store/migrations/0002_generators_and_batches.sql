-- Generic generation platform: jobs record which generator (agent bundle) produced them, jobs can belong to a
-- batch (one request covering several repositories), and users can save generator repositories.

ALTER TABLE jobs
    ADD COLUMN batch_id          uuid,
    -- 'builtin' = the bundle baked into the worker image; 'curated' = operator-configured repository;
    -- 'user' = a repository the user added (must match the operator allowlist).
    ADD COLUMN generator_source  text NOT NULL DEFAULT 'builtin'
        CHECK (generator_source IN ('builtin', 'curated', 'user')),
    ADD COLUMN generator_id      text,
    ADD COLUMN generator_name    text,
    ADD COLUMN generator_owner   text,
    ADD COLUMN generator_repo    text,
    ADD COLUMN generator_branch  text,
    ADD COLUMN generator_path    text,
    -- Commit of the generator repository the job runs: the worker downloads exactly this revision.
    ADD COLUMN generator_sha     text,
    -- Stable identity used for duplicate prevention ('builtin' or 'owner/repo#path').
    ADD COLUMN generator_key     text NOT NULL DEFAULT 'builtin',
    -- The bundle's output allowlist at creation time; publishing re-checks every file against it.
    ADD COLUMN output_paths      jsonb NOT NULL DEFAULT '[]'::jsonb;

CREATE INDEX jobs_batch_idx ON jobs (batch_id) WHERE batch_id IS NOT NULL;

-- At most one active generation per user / repository / branch / model / generator.
DROP INDEX jobs_active_unique;
CREATE UNIQUE INDEX jobs_active_unique
    ON jobs (user_id, lower(owner), lower(repo), base_branch, model, generator_key)
    WHERE status IN ('queued', 'running');

CREATE TABLE user_generators (
    id          uuid PRIMARY KEY,
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name        text        NOT NULL,
    owner       text        NOT NULL,
    repo        text        NOT NULL,
    branch      text,
    path        text        NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX user_generators_unique
    ON user_generators (user_id, lower(owner), lower(repo), coalesce(branch, ''), path);
