-- Central repository (owner/repo) chosen for this job's generator when the central pull request was requested.
ALTER TABLE jobs ADD COLUMN central_publish_repo text;
