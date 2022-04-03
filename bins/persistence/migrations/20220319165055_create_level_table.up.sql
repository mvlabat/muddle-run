-- Add up migration script here

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE OR REPLACE FUNCTION always_set_updated_at_column()
    RETURNS TRIGGER AS
$$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TABLE levels
(
    id           bigserial PRIMARY KEY,
    title        varchar(255)                        NOT NULL,
    user_id      bigint REFERENCES users (id)        NOT NULL,
    parent_id    bigint                              REFERENCES levels (id) ON DELETE SET NULL,
    data         json                                NOT NULL,
    is_autosaved bool                                NOT NULL,
    created_at   timestamp DEFAULT current_timestamp NOT NULL,
    updated_at   timestamp DEFAULT current_timestamp NOT NULL
);

CREATE INDEX trgm_title_idx ON levels USING GIN (title gin_trgm_ops) WHERE (is_autosaved = FALSE);
CREATE INDEX user_id_idx ON levels (user_id) WHERE (is_autosaved = FALSE);
CREATE INDEX parent_id_idx ON levels (parent_id) WHERE (is_autosaved = FALSE);
CREATE INDEX is_not_autosaved_idx ON levels (is_autosaved) WHERE (is_autosaved = FALSE);
CREATE INDEX autosaved_versions_idx ON levels (parent_id, is_autosaved) WHERE (is_autosaved = TRUE);

CREATE TRIGGER set_updated_at
    BEFORE UPDATE
    ON levels
    FOR EACH ROW
EXECUTE PROCEDURE always_set_updated_at_column();

CREATE TABLE level_permissions
(
    id         bigserial PRIMARY KEY,
    level_id   bigint REFERENCES levels (id) ON DELETE CASCADE NOT NULL,
    user_id    bigint REFERENCES users (id) ON DELETE CASCADE  NOT NULL,
    created_at timestamp DEFAULT current_timestamp             NOT NULL,
    UNIQUE (level_id, user_id)
);
