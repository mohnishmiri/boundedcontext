-- GitHub logins asked to review the pull request(s), as a JSON array of strings.
ALTER TABLE jobs ADD COLUMN publish_reviewers jsonb NOT NULL DEFAULT '[]'::jsonb;
