-- Authorization Code + PKCE grants for native clients (WaveFlow Desktop).
--
-- Only the code's SHA-256 hash is stored, like every other opaque token in this
-- schema: a database read must not yield a usable credential. The row is kept
-- after redemption rather than deleted so a replayed code is detected as
-- "already used" instead of silently looking like an unknown code.
CREATE TABLE oauth_authorization (
    code_hash BLOB PRIMARY KEY,
    user_id TEXT NOT NULL,
    client_id TEXT NOT NULL CHECK (length(client_id) > 0),
    redirect_uri TEXT NOT NULL CHECK (length(redirect_uri) > 0),
    -- Base64url S256 challenge. "plain" is not accepted, so no method column.
    code_challenge TEXT NOT NULL CHECK (length(code_challenge) > 0),
    device_name TEXT NOT NULL CHECK (length(device_name) > 0),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    redeemed_at INTEGER,
    FOREIGN KEY (user_id) REFERENCES account(id) ON DELETE CASCADE
);

-- Supports pruning expired grants without scanning the table.
CREATE INDEX oauth_authorization_expires_idx ON oauth_authorization (expires_at);
