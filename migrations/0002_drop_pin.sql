-- Access control lives in front of the app (reverse proxy), so sessions carry no PIN.
ALTER TABLE sessions DROP COLUMN pin_hash;
