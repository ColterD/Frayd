CREATE TABLE IF NOT EXISTS annotations (
  id INTEGER PRIMARY KEY,
  identity_id INTEGER NOT NULL REFERENCES identities(id),
  media_id INTEGER NOT NULL REFERENCES media(id),
  kind TEXT NOT NULL CHECK (kind IN ('highlight','bookmark')),
  spine_idx INTEGER NOT NULL,
  cfi TEXT NOT NULL,
  text TEXT,
  color TEXT DEFAULT 'yellow',
  note TEXT,
  created_at INTEGER DEFAULT (unixepoch()),
  UNIQUE(identity_id, media_id, cfi)
);
CREATE INDEX IF NOT EXISTS idx_annots_user_book ON annotations(identity_id, media_id);
