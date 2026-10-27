CREATE TABLE IF NOT EXISTS reading_states (
    filepath TEXT PRIMARY KEY,
    content_id TEXT NOT NULL,
    textwidth INTEGER NOT NULL,
    offset INTEGER NOT NULL,
    screen_row INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS library (
    last_read DATETIME DEFAULT (datetime('now','localtime')),
    filepath TEXT PRIMARY KEY,
    title TEXT,
    author TEXT,
    reading_progress REAL,
    FOREIGN KEY (filepath) REFERENCES reading_states(filepath) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS bookmarks (
    id TEXT PRIMARY KEY,
    filepath TEXT,
    name TEXT,
    content_id TEXT NOT NULL,
    textwidth INTEGER NOT NULL,
    offset INTEGER NOT NULL,
    screen_row INTEGER NOT NULL,
    FOREIGN KEY (filepath) REFERENCES reading_states(filepath) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS annotation_books (
    id TEXT PRIMARY KEY,
    fingerprint TEXT UNIQUE NOT NULL,
    title TEXT NOT NULL,
    author TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS annotation_locations (
    path TEXT PRIMARY KEY,
    book_id TEXT NOT NULL REFERENCES annotation_books(id)
);
CREATE TABLE IF NOT EXISTS annotations (
    id TEXT PRIMARY KEY,
    book_id TEXT NOT NULL REFERENCES annotation_books(id),
    content_order INTEGER NOT NULL,
    start INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS annotations_book_order
    ON annotations(book_id, content_order, start, id);
CREATE TABLE IF NOT EXISTS annotation_exports (
    book_id TEXT PRIMARY KEY REFERENCES annotation_books(id),
    path TEXT NOT NULL,
    digest TEXT,
    revision INTEGER NOT NULL DEFAULT 0,
    exported_revision INTEGER NOT NULL DEFAULT -1
);
