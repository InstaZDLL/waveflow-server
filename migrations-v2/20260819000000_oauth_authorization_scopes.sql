-- Carry the issuing credential's scopes onto the grant.
--
-- Until now nothing on an authorization recorded what asked for it, so the
-- session redeemed from it carried the account's whole authority whatever the
-- credential that minted it. `Access::Unrestricted` closed that path at the
-- one route that mints, which is a local property; this makes it structural.
--
-- The default is the empty list, which is exactly what every grant written
-- before this migration meant: unrestricted, because sessions were the only
-- thing that could reach the authorize route and a session is unscoped.
ALTER TABLE oauth_authorization
    ADD COLUMN scopes_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scopes_json));
