-- Optional per-user token used only to run Copilot (billing goes to that user), for users whose sign-in token
-- Copilot rejects (classic ghp_). Encrypted by the application; the AAD binds each ciphertext to its user id.
CREATE TABLE copilot_tokens (
    user_id     uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    token       bytea       NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);
