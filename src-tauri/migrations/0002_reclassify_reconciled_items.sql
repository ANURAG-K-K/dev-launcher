-- Items that the old startup sweep marked 'crashed' have no exit code (a real crash records one).
-- Reclassify them to 'stopped' so a completed run's repos no longer appear crashed (red) after an
-- app restart. Genuine crashes (exit_code present) are left untouched.
UPDATE launch_history_items
SET status = 'stopped'
WHERE status = 'crashed' AND exit_code IS NULL;
