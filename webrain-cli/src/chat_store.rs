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

/// `settings.json` beside the chat database — same directory, same override, so
/// `WEBRAIN_CHAT_DB` redirects both and a sandboxed run touches one place.
pub fn settings_path() -> PathBuf {
    db_path().with_file_name("settings.json")
}

/// Saved `server:tool` approvals, same directory for the same reason. Beside the
/// settings rather than in them: this list is a standing permission, and a
/// settings file a user edits by hand is the wrong place to keep one.
pub fn approvals_path() -> PathBuf {
    db_path().with_file_name("approvals.json")
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
             );
             -- The whole transcript as the page drew it, one row per conversation:
             -- tool calls with their arguments and results, the steps strip, the
             -- notes. `messages` is only the words, so reopening a conversation
             -- from it lost every card that made the answer checkable. Written
             -- whole because the page owns the shape and a diffed table would be
             -- a second copy of it to keep in sync.
             CREATE TABLE IF NOT EXISTS transcript (
                 convo TEXT PRIMARY KEY,
                 body  TEXT NOT NULL,
                 ts    TEXT NOT NULL DEFAULT (datetime('now'))
             );
             -- The last context-window snapshot the runtime reported for a
             -- conversation. The live attribution only exists during a turn, so a
             -- reload or a restart answered no snapshot while the history was still
             -- on screen; this is what lets the panel say as of the last turn
             -- instead of going blank.
             CREATE TABLE IF NOT EXISTS context (
                 convo TEXT PRIMARY KEY,
                 body  TEXT NOT NULL,
                 ts    TEXT NOT NULL DEFAULT (datetime('now'))
             );
             -- What the page switched on for ONE conversation: the runtime
             -- switches, the tool picks and the model it runs. Kept here rather
             -- than in `settings.json` so two chats cannot overwrite each other;
             -- that file stays the default a conversation with no row takes, which
             -- is also what a new chat starts from.
             CREATE TABLE IF NOT EXISTS settings (
                 convo TEXT PRIMARY KEY,
                 body  TEXT NOT NULL,
                 ts    TEXT NOT NULL DEFAULT (datetime('now'))
             );
             -- When a conversation was last OPENED. The menu is ordered by the
             -- later of this and its newest message, so picking a thread lifts it
             -- to the top and it stays there across a reload — the order ChatGPT
             -- and Claude keep. Same `datetime('now')` format as `messages.ts`, so
             -- the two compare as strings.
             CREATE TABLE IF NOT EXISTS seen (
                 convo TEXT PRIMARY KEY,
                 ts    TEXT NOT NULL DEFAULT (datetime('now'))
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
    /// The last page of a conversation, newest first: the one the reader opened
    /// most recently, else the one whose last message is newest. Opening a thread
    /// therefore lifts it to the top on the next read, which is what every chat app
    /// does and what a menu ordered by message time alone cannot do.
    ///
    /// `limit + 1` is the caller's trick for `more`; nothing here knows about it.
    pub fn convos(&self, limit: i64, offset: i64) -> Vec<Value> {
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        let mut stmt = match db.prepare(
            "SELECT t.convo,
                    COALESCE((SELECT title FROM titles WHERE convo = t.convo),
                             (SELECT body FROM messages m WHERE m.convo = t.convo AND role = 'user'
                              ORDER BY id LIMIT 1)),
                    MAX(ts), COUNT(*),
                    COALESCE((SELECT ts FROM seen s WHERE s.convo = t.convo), MAX(t.ts)),
                    (SELECT ts FROM seen s WHERE s.convo = t.convo) IS NOT NULL
             FROM messages t GROUP BY convo
             -- Both timestamps are whole seconds, so a thread written and opened in
             -- the same second ties: the one actually opened is the tiebreak, and
             -- the newest message decides the rest.
             ORDER BY 5 DESC, 6 DESC, MAX(t.id) DESC LIMIT ?1 OFFSET ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([limit.max(1), offset.max(0)], |r| {
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

    /// Mark a conversation as the one the reader just opened. Its own row, so two
    /// tabs opening two threads do not overwrite one "last opened" value.
    pub fn touch(&self, convo: &str) -> bool {
        if convo.is_empty() {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO seen (convo) VALUES (?1)
                 ON CONFLICT(convo) DO UPDATE SET ts = datetime('now')",
                [convo],
            )
            .is_ok()
        })
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
                && db
                    .execute("DELETE FROM transcript WHERE convo = ?1", [convo])
                    .is_ok()
                && db
                    .execute("DELETE FROM context WHERE convo = ?1", [convo])
                    .is_ok()
                // Its switches go too: a re-used id would otherwise open with a
                // model nobody chose for it.
                && db
                    .execute("DELETE FROM settings WHERE convo = ?1", [convo])
                    .is_ok()
                && db
                    .execute("DELETE FROM seen WHERE convo = ?1", [convo])
                    .is_ok()
        })
    }

    /// Replace the stored transcript of `convo` with what the page just drew.
    /// A row that is not a JSON array is refused rather than stored, so replay
    /// never has to defend against its own database.
    pub fn save_rows(&self, convo: &str, body: &str) -> bool {
        if convo.is_empty() || serde_json::from_str::<Value>(body).is_err() {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO transcript (convo, body) VALUES (?1, ?2)
                 ON CONFLICT(convo) DO UPDATE SET body = excluded.body, ts = datetime('now')",
                params![convo, body],
            )
            .is_ok()
        })
    }

    /// The rows to redraw a conversation from, or `None` when it was saved
    /// before transcripts were kept (those conversations still have `messages`).
    pub fn rows(&self, convo: &str) -> Option<Value> {
        let db = self.lock()?;
        let body: String = db
            .query_row(
                "SELECT body FROM transcript WHERE convo = ?1",
                [convo],
                |r| r.get(0),
            )
            .ok()?;
        serde_json::from_str::<Value>(&body).ok()
    }

    /// Remember the context snapshot the runtime just reported (upsert: one row
    /// per conversation, the newest wins).
    pub fn save_context(&self, convo: &str, body: &str) -> bool {
        if convo.is_empty() || serde_json::from_str::<Value>(body).is_err() {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO context (convo, body) VALUES (?1, ?2)
                 ON CONFLICT(convo) DO UPDATE SET body = excluded.body, ts = datetime('now')",
                params![convo, body],
            )
            .is_ok()
        })
    }

    /// The last snapshot for `convo`, with the time it was taken.
    pub fn context(&self, convo: &str) -> Option<(Value, String)> {
        let db = self.lock()?;
        let (body, ts): (String, String) = db
            .query_row(
                "SELECT body, ts FROM context WHERE convo = ?1",
                [convo],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        Some((serde_json::from_str::<Value>(&body).ok()?, ts))
    }

    /// Remember the switches, tool picks and model one conversation was held with.
    pub fn save_settings(&self, convo: &str, body: &str) -> bool {
        if convo.is_empty() || serde_json::from_str::<Value>(body).is_err() {
            return false;
        }
        self.lock().is_some_and(|db| {
            db.execute(
                "INSERT INTO settings (convo, body) VALUES (?1, ?2)
                 ON CONFLICT(convo) DO UPDATE SET body = excluded.body, ts = datetime('now')",
                params![convo, body],
            )
            .is_ok()
        })
    }

    /// What that conversation was held with, or `None` for one that never saved —
    /// which is the signal to take the defaults instead.
    pub fn settings(&self, convo: &str) -> Option<Value> {
        let db = self.lock()?;
        let body: String = db
            .query_row("SELECT body FROM settings WHERE convo = ?1", [convo], |r| {
                r.get(0)
            })
            .ok()?;
        serde_json::from_str::<Value>(&body).ok()
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

    /// Every word of the query is matched on its own, and the rows are ranked by
    /// how much of the query they answer: more words hit first, then words hit in
    /// the thread's opening message (the text every menu shows as its title),
    /// then the phrase surviving in one piece, then recency.
    ///
    /// That is the difference that makes "fpv sale" find a message saying "fpv
    /// drone for sale": one `LIKE` over the whole query needed the words adjacent
    /// and in order, so the words a person actually types never matched.
    ///
    /// ponytail: `LIKE` + in-process scoring — no FTS5 shadow table, no
    /// embeddings, no network. A personal history is thousands of rows, where
    /// this answers in under a millisecond; switch to FTS5+bm25 (or vectors) once
    /// a LIKE scan is measurably slow, i.e. tens of thousands of messages.
    pub fn search(&self, q: &str) -> Vec<Value> {
        let phrase = q.trim().to_lowercase();
        let words: Vec<&str> = phrase.split_whitespace().collect();
        if words.is_empty() {
            return Vec::new();
        }
        let Some(db) = self.lock() else {
            return Vec::new();
        };
        // `%` and `_` are wildcards in LIKE; a user typing them means them
        // literally.
        let like = |w: &str| {
            format!(
                "%{}%",
                w.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        };
        // One bound parameter per word: any word may hit, and the scoring below
        // decides which rows answer the query best.
        let any = (1..=words.len())
            .map(|i| format!("t.body LIKE ?{i} ESCAPE '\\'"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT t.convo,
                    (SELECT body FROM messages m WHERE m.convo = t.convo AND role = 'user'
                     ORDER BY id LIMIT 1),
                    t.role, substr(t.body, 1, 2000), t.ts, t.id,
                    t.role = 'user' AND t.id = (
                        SELECT MIN(id) FROM messages m WHERE m.convo = t.convo AND role = 'user'
                    ) AS is_title
             FROM messages t
             WHERE {any}
             LIMIT 2000"
        );
        let mut stmt = match db.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let params: Vec<String> = words.iter().map(|w| like(w)).collect();
        let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(params), |r| {
            Ok((
                json!({
                    "convo": r.get::<_, String>(0)?,
                    "title": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "role": r.get::<_, String>(2)?,
                    "body": r.get::<_, String>(3)?,
                    "ts": r.get::<_, String>(4)?,
                    "title_hit": r.get::<_, bool>(6)?,
                }),
                r.get::<_, i64>(5)?,
            ))
        }) else {
            return Vec::new();
        };
        let mut ranked: Vec<(i32, i64, Value)> = rows
            .filter_map(Result::ok)
            .filter_map(|(hit, id)| {
                let body = hit["body"].as_str().unwrap_or_default().to_lowercase();
                let title = hit["title"].as_str().unwrap_or_default().to_lowercase();
                let (mut found, mut score) = (0, 0);
                for w in &words {
                    let in_body = body.contains(w);
                    let in_title = title.contains(w);
                    if in_body || in_title {
                        found += 1;
                    }
                    if in_body {
                        score += 1;
                    }
                    if in_title {
                        score += 4;
                    }
                }
                if found == 0 {
                    return None; // LIKE can only over-match, never miss
                }
                if words.len() > 1 && body.contains(&phrase) {
                    score += 6;
                }
                if hit["title_hit"].as_bool().unwrap_or(false) {
                    score += 2;
                }
                Some((found * 100 + score, id, hit))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        ranked.into_iter().take(40).map(|(_, _, v)| v).collect()
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

    /// One row per conversation, newest wins, and deleting the thread takes its
    /// snapshot with it — a stale context block on a re-used id would be a lie.
    #[test]
    fn context_snapshot_is_one_row_per_conversation_and_dies_with_it() {
        let (s, path) = temp();
        assert!(s.context("c1").is_none(), "nothing saved yet");
        assert!(s.save_context("c1", r#"{"totalTokens":100,"attribution":{"x":1}}"#));
        assert!(s.save_context("c1", r#"{"totalTokens":250}"#), "upsert");
        assert!(s.save_context("other", r#"{"totalTokens":7}"#), "per convo");
        assert!(!s.save_context("c1", "not json"), "a broken row is refused");

        let (v, ts) = s.context("c1").expect("snapshot");
        assert_eq!(v["totalTokens"], 250, "the newest wins");
        assert!(!ts.is_empty(), "the saved time is reported");

        s.forget("c1");
        assert!(s.context("c1").is_none(), "deleting the thread clears it");
        assert!(s.context("other").is_some(), "and only that thread");
        drop(s);
        let _ = std::fs::remove_file(path);
    }

    /// Two chats can hold different models and tool picks, so the row is keyed by
    /// conversation: one row each, newest wins, and a deleted thread takes its
    /// switches with it.
    #[test]
    fn settings_are_one_row_per_conversation() {
        let (s, path) = temp();
        assert!(s.settings("c1").is_none(), "nothing saved yet");
        assert!(s.save_settings("c1", r#"{"model":"gpt-5.2","provider":"openai"}"#));
        assert!(
            s.save_settings("c1", r#"{"model":"claude-sonnet-4.5"}"#),
            "upsert"
        );
        assert!(
            s.save_settings("other", r#"{"model":"gpt-5-mini"}"#),
            "per convo"
        );
        assert!(
            !s.save_settings("c1", "not json"),
            "a broken row is refused"
        );
        assert!(
            !s.save_settings("", r#"{"model":"x"}"#),
            "no conversation, no row"
        );

        assert_eq!(s.settings("c1").expect("row")["model"], "claude-sonnet-4.5");
        assert_eq!(s.settings("other").expect("row")["model"], "gpt-5-mini");

        s.forget("c1");
        assert!(s.settings("c1").is_none(), "deleting the thread clears it");
        assert!(s.settings("other").is_some(), "and only that thread");
        drop(s);
        let _ = std::fs::remove_file(path);
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

        let convos = s.convos(20, 0);
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

    /// Reading a conversation keeps it — one ordered row per conversation, cut into
    /// pages — and *opening* it is what lifts it, even when a newer one exists. That
    /// is the order the rail promises, and it has to survive a reload, so it is a
    /// row and not a page-side sort.
    #[test]
    fn opening_a_conversation_puts_it_first_and_the_list_pages() {
        let (s, path) = temp();
        s.append("old", "user", "oldest");
        s.append("new", "user", "newest");
        assert_eq!(s.convos(20, 0)[0]["id"], "new", "newest message first");
        assert!(s.touch("old"), "opening is recorded");
        assert_eq!(s.convos(20, 0)[0]["id"], "old", "opened one is first now");
        // A page is a window, not a filter: two rows in one-rows pages, no overlap.
        assert_eq!(s.convos(1, 0).len(), 1);
        assert_eq!(s.convos(1, 0)[0]["id"], "old");
        assert_eq!(s.convos(1, 1)[0]["id"], "new");
        assert!(s.convos(1, 2).is_empty(), "past the end");
        // Deleting the thread forgets when it was opened too.
        s.forget("old");
        assert_eq!(s.convos(20, 0)[0]["id"], "new");
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
        assert_eq!(s.search("50%x").len(), 0);
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

    #[test]
    fn a_multi_word_query_matches_the_words_apart_and_ranks_by_coverage() {
        let (s, path) = temp();
        s.append("c_both", "user", "FPV drone for sale, barely used");
        s.append("c_one", "assistant", "the FPV build needs a camera");
        s.append("c_none", "user", "groceries");

        // "fpv sale" is not a substring of anything: the words have to be
        // matched on their own, which is the whole point.
        let hits = s.search("fpv sale");
        assert_eq!(hits.len(), 2, "a row with either word is a candidate");
        assert_eq!(hits[0]["convo"], "c_both", "the row with both words wins");
        assert_eq!(hits[1]["convo"], "c_one");

        assert_eq!(
            s.search("sale zebra").len(),
            1,
            "one word is enough to match"
        );
        assert!(s.search("apples pears").is_empty(), "no word, no row");
        drop(s);
        let _ = std::fs::remove_file(path);
    }
}
