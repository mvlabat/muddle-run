-- Add up migration script here

CREATE OR REPLACE FUNCTION set_updated_at_column()
    RETURNS TRIGGER AS
$$
BEGIN
    IF row (NEW.*) IS DISTINCT FROM row (OLD.*) THEN
        NEW.updated_at = now();
        RETURN NEW;
    ELSE
        RETURN OLD;
    END IF;
END;
$$ language 'plpgsql';

CREATE TABLE users
(
    id           bigserial PRIMARY KEY,
    display_name varchar(255) UNIQUE,
    email        varchar(320) UNIQUE,
    created_at   timestamp DEFAULT current_timestamp NOT NULL,
    updated_at   timestamp DEFAULT current_timestamp NOT NULL
);

CREATE TRIGGER set_updated_at
    BEFORE UPDATE
    ON users
    FOR EACH ROW
EXECUTE PROCEDURE set_updated_at_column();

CREATE TABLE openids
(
    id         bigserial PRIMARY KEY,
    user_id    bigint REFERENCES users (id),
    issuer     varchar(2048)                       NOT NULL,
    subject    varchar(255)                        NOT NULL,
    email      varchar(320),
    created_at timestamp DEFAULT current_timestamp NOT NULL,
    updated_at timestamp DEFAULT current_timestamp NOT NULL,
    UNIQUE (issuer, subject)
);

CREATE TRIGGER set_updated_at
    BEFORE UPDATE
    ON openids
    FOR EACH ROW
EXECUTE PROCEDURE set_updated_at_column();
