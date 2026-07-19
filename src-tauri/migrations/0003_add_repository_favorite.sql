-- Favorite flag for pinning repositories to the top of the list (R7).
ALTER TABLE repositories ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
