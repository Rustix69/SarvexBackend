ALTER TABLE auth.users
  ADD COLUMN IF NOT EXISTS display_name TEXT NOT NULL DEFAULT '';

UPDATE auth.users
SET display_name = user_id
WHERE display_name = '';
