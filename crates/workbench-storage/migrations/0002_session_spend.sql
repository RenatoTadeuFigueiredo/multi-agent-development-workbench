-- Durable redacted session paid-inference spend (Feature 014 / issue #31).
-- Micros of USD only; never API keys or request/response bodies.

ALTER TABLE sessions ADD COLUMN spend_usd_micros INTEGER NOT NULL DEFAULT 0
    CHECK (spend_usd_micros >= 0);

PRAGMA user_version = 2;
