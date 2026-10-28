-- Frayd schema v0.1

PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS media (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('audiobook','ebook','comic','manga','movie','music','episode')),
  path TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  subtitle TEXT,
  author TEXT,
  series TEXT,
  duration_secs REAL,
  size_bytes INTEGER,
  mtime_secs INTEGER,
  meta_json TEXT,            -- chapters, tags, page lists, etc.
  thumb_path TEXT,
  transcode_path TEXT,
  created_at INTEGER DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_media_kind ON media(kind);
CREATE INDEX IF NOT EXISTS idx_media_title ON media(title);

CREATE TABLE IF NOT EXISTS identities (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('device','account')),
  key TEXT NOT NULL UNIQUE,          -- hostname:uuid for device, username for account
  secret_hash TEXT,                  -- device secret or argon2id (M5)
  fingerprint TEXT,
  created_at INTEGER DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS sessions (
  token TEXT PRIMARY KEY,            -- random 128-bit hex
  identity_id INTEGER NOT NULL REFERENCES identities(id),
  created_at INTEGER DEFAULT (unixepoch()),
  expires_at INTEGER NOT NULL        -- unixepoch(); sliding window, refreshed on use
);

CREATE TABLE IF NOT EXISTS progress (
  identity_id INTEGER NOT NULL REFERENCES identities(id),
  media_id INTEGER NOT NULL REFERENCES media(id),
  position REAL NOT NULL,            -- secs for AV, page/entry index for comics/epubs
  completed INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER,
  PRIMARY KEY (identity_id, media_id)
);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL                -- JSON
);

-- Per-identity library visibility. Empty table or no row = see everything.
-- allowed_kinds: NULL = all kinds; else JSON array of kinds visible.
CREATE TABLE IF NOT EXISTS user_perms (
  identity_id INTEGER PRIMARY KEY REFERENCES identities(id),
  is_admin INTEGER NOT NULL DEFAULT 0,
  allowed_kinds TEXT                  -- NULL = all; else JSON ["movie","music",...]
);

CREATE TABLE IF NOT EXISTS albums (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  artist TEXT,
  art_path TEXT
);

CREATE TABLE IF NOT EXISTS tracks (
  media_id INTEGER PRIMARY KEY REFERENCES media(id),
  album_id INTEGER REFERENCES albums(id),
  track_no INTEGER,
  disc_no INTEGER
);

CREATE TABLE IF NOT EXISTS playlists (
  id INTEGER PRIMARY KEY,
  identity_id INTEGER NOT NULL REFERENCES identities(id),
  title TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS playlist_items (
  playlist_id INTEGER NOT NULL REFERENCES playlists(id),
  media_id INTEGER NOT NULL REFERENCES media(id),
  pos INTEGER NOT NULL,
  PRIMARY KEY (playlist_id, pos)
);

-- Exact ownership records only: file-name suffixes never establish ownership.
CREATE TABLE IF NOT EXISTS transcode_artifacts (
  path TEXT PRIMARY KEY,
  owner_media_id INTEGER NOT NULL REFERENCES media(id),
  owner_source_path TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('temp','final')),
  provenance TEXT NOT NULL CHECK(provenance IN ('generated','legacy_reference')),
  CHECK(path != owner_source_path)
);
