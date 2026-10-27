ALTER TABLE jobs
    ADD COLUMN publish_title text,
    ADD COLUMN publish_central boolean NOT NULL DEFAULT false,
    ADD COLUMN central_publish_branch text,
    ADD COLUMN central_publish_commit_sha text,
    ADD COLUMN central_publish_pr_number integer,
    ADD COLUMN central_publish_pr_url text;

ALTER TABLE jobs DROP CONSTRAINT jobs_publish_stage_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_publish_stage_check CHECK (publish_stage IN (
    'queued', 'checking_base', 'creating_commit', 'creating_branch', 'creating_pull_request',
    'checking_central_base', 'creating_central_commit', 'creating_central_branch',
    'creating_central_pull_request', 'completed'
));