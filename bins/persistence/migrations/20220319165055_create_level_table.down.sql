-- Add down migration script here
DROP TRIGGER set_updated_at ON levels;
DROP TABLE level_permissions;
DROP TABLE levels;
