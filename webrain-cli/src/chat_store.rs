//! Conversation history for `webrain chat --serve`.
//!
//! One file, one table. The page's session menu reads it; nothing else does.
//! History is a convenience, so every failure here is silent: a read-only or
//! locked database must never take the chat down with it.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub struct Store {
    db: Mutex<Connection>,
}

fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("WEBRAIN_CHAT_DB") {
        return p.into();
    }
    webrain_core::vault::vault_dir().join("chat.db")
}

impl Store {
    /// `None` when the file cannot be opened — the caller treats that as "no
    /// history menu" rather than an error.
    pub fn open() -> Option<Self> {
        Self::open_at(db_path())
    }

    fn open_at(path: PathBuf) -> Option<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        let db = Connection::open(path).ok()?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS messages (
                 id    INTEGER PRIMARY KEY AUTOINCREMENT,
                 convo TEXT NOT NULL,
                 role  TEXT NOT NULL,
                 body  TEXT NOT NULL,
                 ts    TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS messages_convo ON messages(convo, id);
             -- A renamed conversation keeps its own title; without one the menu
             -- falls back to the first thing the user said.
             CREATE TABLE IF NOT EXISTS titles (
                 convo TEXT PRIMARY KEY,
                 title TEXT NOT NULL
             );
             -- The right-hand panel, per turn, kept as the page wrote it: the
             -- numbers depend on provider prices and cache discounts that live
             -- in the page, and a second copy of that arithmetic here would
             -- drift. One row per turn, upserted so a replayed turn cannot
             -- double-count.
             CREATE TABLE IF NOT EXISTS turns (
                 convo TEXT NOT NULL,
                 n     INTEGER NOT NULL,
                 body  TEXT NOT NULL,
                 PRIMARY KEY (convo, n)
             );
             -- Providers added from the UI. Keys are never stored: a row names the
             -- environment variable that holds one.
             CREATE TABLE IF NOT EXISTS providers (
                 id       TEXT PRIMARY KEY,
                 base_url TEXT NOT NULL,
                 model    TEXT NOT NULL,
                 key_env  TEXT NOT NULL,
                 created  TEXT NOT NULL DEFAULT (datetime('now'))
             );",
        )
        .ok()?;
        Some(Self { db: Mutex::new(db) })
    }

    fn lock(&self) -> Option<std::sync::MutexGuard<'_, Connection>> {
        // ponytail: a blocked thread here is a 1 ms write on a local file; a
        // channel+worker only pays off if history ever leaves this machine.
        self.db.lock().ok()
    }

    pub fn append(&self, convo: &str, role: &str, body: &str) {
        if body.trim().is_empty() {
            return;
        }
        if let Some(db) = self.lock() {
            let _ = db.execute(
                "INSERT INTO messages (convo, role, body) VALUES (?1, ?2, ?3)",
                params![convo, role, body],
            );
        }
    }

    /// Newest conversation first, titled by its opening user message.
    pub fn convos(&self) -> Vec<Value> {
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        let mut stmt = match db.prepare(
            "SELECT convo,
                    COALESCE((SELECT title FROM titles WHERE convo = t.convo),
                             (SELECT body FROM messages m WHERE m.convo = t.convo AND role = 'user'
                              ORDER BY id LIMIT 1)),
                    MAX(ts), COUNT(*)
             FROM messages t GROUP BY convo ORDER BY MAX(id) DESC LIMIT 50",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "title": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "ts": r.get::<_, String>(2)?,
                "n": r.get::<_, i64>(3)?,
            }))
        });
        rows.map(|r| r.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    pub fn messages(&self, convo: &str) -> Vec<Value> {
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        let mut stmt =
            match db.prepare("SELECT role, body, ts FROM messages WHERE convo = ?1 ORDER BY id") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
        let rows = stmt.query_map([convo], |r| {
            Ok(json!({
                "role": r.get::<_, String>(0)?,
                "body": r.get::<_, String>(1)?,
                "ts": r.get::<_, String>(2)?,
            }))
        });
        rows.map(|r| r.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// The title a conversation shows in the menu: the one it was renamed to, or
    /// the first thing the user said.
    pub fn rename(&self, convo: &str, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO titles (convo, title) VALUES (?1, ?2)
                 ON CONFLICT(convo) DO UPDATE SET title = excluded.title",
                params![convo, title],
            )
            .is_ok()
        })
    }

    /// One turn's instruments, exactly as the page measured them. Malformed JSON
    /// is refused rather than stored, so the history panel can replay a row
    /// without defending against its own database.
    pub fn record_turn(&self, convo: &str, n: u32, body: &str) -> bool {
        if n == 0 || serde_json::from_str::<Value>(body).is_err() {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO turns (convo, n, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT(convo, n) DO UPDATE SET body = excluded.body",
                params![convo, n as i64, body],
            )
            .is_ok()
        })
    }

    /// Newest first, the order the page keeps its own ledger in.
    pub fn turns(&self, convo: &str) -> Vec<Value> {
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        let mut stmt =
            match db.prepare("SELECT body FROM turns WHERE convo = ?1 ORDER BY n DESC LIMIT 200") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
        let rows = stmt.query_map([convo], |r| {
            let body: String = r.get(0)?;
            Ok(serde_json::from_str::<Value>(&body).unwrap_or(Value::Null))
        });
        rows.map(|r| r.filter_map(Result::ok).filter(|v| !v.is_null()).collect())
            .unwrap_or_default()
    }

    /// Drop a conversation and everything in it. The title row goes with it, or a
    /// re-used id would inherit a stale name.
    pub fn forget(&self, convo: &str) -> bool {
        self.lock().is_some_and(|db| {
            db.execute("DELETE FROM messages WHERE convo = ?1", [convo])
                .is_ok()
                && db
                    .execute("DELETE FROM titles WHERE convo = ?1", [convo])
                    .is_ok()
                && db
                    .execute("DELETE FROM turns WHERE convo = ?1", [convo])
                    .is_ok()
        })
    }

    /// Copy the first `upto` messages of `convo` into `into`. A fork is a
    /// *transcript* operation: the runtime keeps one linear context, so the copy
    /// is a clean thread to continue in — not a rewind of the model's memory.
    pub fn fork(&self, convo: &str, upto: usize, into: &str) -> bool {
        if into.is_empty() || upto == 0 {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO messages (convo, role, body, ts)
                 SELECT ?2, role, body, ts FROM messages
                 WHERE convo = ?1 ORDER BY id LIMIT ?3",
                params![convo, into, upto as i64],
            )
            .is_ok_and(|n| n > 0)
        })
    }

    pub fn providers(&self) -> Vec<Value> {
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        let mut stmt = match db
            .prepare("SELECT id, base_url, model, key_env FROM providers ORDER BY created")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "base_url": r.get::<_, String>(1)?,
                "model": r.get::<_, String>(2)?,
                "key_env": r.get::<_, String>(3)?,
            }))
        });
        rows.map(|r| r.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// A provider row is only ever a *target*: base url, model, and the name of the
    /// environment variable that holds the key. The key itself never reaches disk.
    pub fn add_provider(&self, id: &str, base_url: &str, model: &str, key_env: &str) -> bool {
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO providers (id, base_url, model, key_env) VALUES (?1, ?2, ?3, ?4)",
                params![id, base_url, model, key_env],
            )
            .is_ok()
        })
    }

    /// Substring search over every message body, newest first.
    ///
    /// ponytail: `LIKE` over one table — no FTS5 shadow table, no embeddings, no
    /// network. A personal history is thousands of rows, where this answers in
    /// under a millisecond; switch to FTS5+bm25 (or vectors) once a LIKE scan is
    /// measurably slow, i.e. tens of thousands of messages.
    pub fn search(&self, q: &str) -> Vec<Value> {
        let q = q.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        // `%` and `_` are wildcards in LIKE; a user typing them means them
        // literally.
        let pattern = format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut stmt = match db.prepare(
            // A hit in the conversation's opening message (the text every menu
            // shows as its title) outranks one buried mid-thread; recency breaks
            // the tie.
            "SELECT t.convo,
                    (SELECT body FROM messages m WHERE m.convo = t.convo AND role = 'user'
                     ORDER BY id LIMIT 1),
                    t.role, substr(t.body, 1, 2000), t.ts,
                    t.role = 'user' AND t.id = (
                        SELECT MIN(id) FROM messages m WHERE m.convo = t.convo AND role = 'user'
                    ) AS is_title
             FROM messages t
             WHERE t.body LIKE ?1 ESCAPE '\\'
             ORDER BY is_title DESC, t.id DESC LIMIT 40",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([&pattern], |r| {
            Ok(json!({
                "convo": r.get::<_, String>(0)?,
                "title": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "role": r.get::<_, String>(2)?,
                "body": r.get::<_, String>(3)?,
                "ts": r.get::<_, String>(4)?,
                "title_hit": r.get::<_, bool>(5)?,
            }))
        });
        rows.map(|r| r.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (Store, PathBuf) {
        let p = std::env::temp_dir().join(format!(
            "webrain-chat-{}-{:?}.db",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&p);
        (Store::open_at(p.clone()).expect("store"), p)
    }

    #[test]
    fn fork_copies_only_the_prefix_it_was_asked_for() {
        let (s, path) = temp();
        s.append("c1", "user", "one");
        s.append("c1", "assistant", "two");
        s.append("c1", "user", "three");
        s.append("c1", "assistant", "four");

        assert!(s.fork("c1", 2, "c1-fork"));
        let copied = s.messages("c1-fork");
        assert_eq!(copied.len(), 2, "only the first two messages are copied");
        assert_eq!(copied[1]["body"], "two");
        assert_eq!(s.messages("c1").len(), 4, "the source is untouched");

        // A fork of nothing, or into nothing, is not a fork.
        assert!(!s.fork("c1", 0, "empty"));
        assert!(!s.fork("c1", 2, ""));
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn keeps_each_turns_instruments_newest_first_and_forgets_them() {
        let (s, path) = temp();
        s.append("c1", "user", "hello");
        assert!(s.record_turn("c1", 1, r#"{"n":1,"model":"m","input":10,"output":4}"#));
        assert!(s.record_turn("c1", 2, r#"{"n":2,"model":"m","input":20,"output":8}"#));
        // A replayed turn replaces its row instead of counting twice.
        assert!(s.record_turn("c1", 2, r#"{"n":2,"model":"m","input":21,"output":8}"#));
        assert!(!s.record_turn("c1", 0, "{}"), "turn 0 is not a turn");
        assert!(
            !s.record_turn("c1", 3, "not json"),
            "a broken row is refused"
        );

        let turns = s.turns("c1");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0]["input"], 21, "newest first, and upserted");
        assert!(s.turns("nobody").is_empty());

        s.forget("c1");
        assert_eq!(
            s.turns("c1").len(),
            0,
            "the ledger dies with the conversation"
        );
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn round_trips_and_titles_by_first_user_message() {
        let (s, path) = temp();
        s.append("c1", "user", "hello there");
        s.append("c1", "assistant", "hi");
        s.append("c1", "user", "second");
        s.append("", "user", "ignored empty convo id is still stored");
        s.append("c2", "user", ""); // dropped: empty body

        let convos = s.convos();
        assert_eq!(convos.len(), 2);
        let c1 = convos.iter().find(|c| c["id"] == "c1").expect("c1");
        assert_eq!(c1["title"], "hello there");
        assert_eq!(c1["n"], 3);

        let msgs = s.messages("c1");
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["role"], "assistant");
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn search_matches_bodies_and_treats_wildcards_literally() {
        let (s, path) = temp();
        s.append("c1", "user", "find the 50% discount");
        s.append("c1", "assistant", "done: discount applied");
        s.append("c2", "user", "unrelated");
        drop(s);

        let s = Store::open_at(path.clone()).expect("reopen");
        let hits = s.search("DISCOUNT"); // case-insensitive by schema, not by caller
        assert_eq!(hits.len(), 2, "both messages mentioning discount");
        assert_eq!(hits[0]["convo"], "c1");
        assert_eq!(hits[0]["title"], "find the 50% discount");
        // `%` must not act as a wildcard: this only matches the literal text.
        assert_eq!(s.search("50%").len(), 1);
        assert_eq!(s.search("5% d").len(), 0);
        assert!(s.search("   ").is_empty());
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_title_hit_outranks_a_buried_one() {
        let (s, path) = temp();
        // c_old's opening message is newer than c_new's *later* message, so
        // recency alone would put the wrong conversation first.
        s.append("c_buried", "user", "nothing to see");
        s.append("c_buried", "assistant", "later mention of zebra");
        s.append("c_title", "user", "zebra at the title");

        let hits = s.search("zebra");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["convo"], "c_title");
        assert_eq!(hits[0]["title_hit"], true);
        assert_eq!(hits[1]["title_hit"], false);
        drop(s);
        let _ = std::fs::remove_file(path);
    }
}
