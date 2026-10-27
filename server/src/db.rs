use rusqlite::Connection;
use std::path::Path;

pub fn open(path: &Path) -> Connection {
    let c = Connection::open(path).expect("база даних");
    c.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            uuid TEXT NOT NULL UNIQUE,
            pass TEXT NOT NULL,
            skin TEXT,
            slim INTEGER NOT NULL DEFAULT 0,
            bio TEXT NOT NULL DEFAULT '',
            created INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS tokens (
            hash TEXT PRIMARY KEY,
            user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            client TEXT NOT NULL,
            created INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS friends (
            a INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            b INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            accepted INTEGER NOT NULL DEFAULT 0,
            created INTEGER NOT NULL,
            PRIMARY KEY (a, b)
         );
         CREATE TABLE IF NOT EXISTS packs (
            id TEXT PRIMARY KEY,
            owner INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            mc_version TEXT NOT NULL,
            loader TEXT NOT NULL,
            visibility TEXT NOT NULL,
            version INTEGER NOT NULL,
            size INTEGER NOT NULL,
            created INTEGER NOT NULL,
            updated INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS subs (
            user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            pack TEXT NOT NULL REFERENCES packs(id) ON DELETE CASCADE,
            PRIMARY KEY (user, pack)
         );
         CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY,
            user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            pack TEXT,
            data TEXT NOT NULL,
            created INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS events_user ON events(user, id);
         CREATE TABLE IF NOT EXISTS profiles (
            id INTEGER PRIMARY KEY,
            user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            uuid TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            mojang_skin TEXT,
            mojang_slim INTEGER NOT NULL DEFAULT 0,
            created INTEGER NOT NULL
         );",
    )
    .expect("схема бази");
    // 1.1.1: токени гри прив'язані до ігрового профілю; токени акаунта — без нього.
    let has_profile: bool = c.prepare("SELECT 1 FROM pragma_table_info('tokens') WHERE name = 'profile'").unwrap().exists([]).unwrap();
    if !has_profile {
        c.execute_batch("ALTER TABLE tokens ADD COLUMN profile INTEGER REFERENCES profiles(id) ON DELETE CASCADE;").expect("міграція tokens");
    }
    c
}
