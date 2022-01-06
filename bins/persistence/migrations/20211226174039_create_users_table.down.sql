-- Add down migration script here
DROP TRIGGER set_updated_at ON openids;
DROP TRIGGER set_updated_at ON users;
DROP TABLE openids;
DROP TABLE users;
