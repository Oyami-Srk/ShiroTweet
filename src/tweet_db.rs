use crate::utils::extract_twitter_url;
use crate::utils::Error;
use crate::utils::Error::{TweetRestricted, TwitterAccountNotExisted, TwitterAccountSuspended};
use anyhow::Result;
use log::error;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub struct ThreadInfo {
    pub tweet_id: u64,
    pub thread_id: u64,
    pub reply_to: u64,
}

#[derive(Debug)]
pub struct Tweet {
    pub id: u64,
    pub author: String,
    pub content: String,
    pub create_time: u64, // sec timestamp
}

#[derive(Debug)]
pub struct Media {
    pub id: String,
    pub tweet_id: u64,
    pub url: String,
    pub width: u64,
    pub height: u64,
    pub no: i32,
    pub _type: String,
}

pub enum TweetFailReason {
    Restricted,
    Deleted,
    AccountSuspended,
    AccountNotExisted,
}

impl ToString for TweetFailReason {
    fn to_string(&self) -> String {
        match self {
            Self::Restricted => "restricted",
            Self::Deleted => "deleted",
            Self::AccountSuspended => "account suspended",
            Self::AccountNotExisted => "account not existed",
        }
        .into()
    }
}

impl Into<Error> for TweetFailReason {
    fn into(self) -> Error {
        match self {
            Self::Restricted => Error::TweetRestricted,
            Self::Deleted => Error::TweetNotExists,
            Self::AccountSuspended => Error::TwitterAccountSuspended,
            Self::AccountNotExisted => Error::TwitterAccountNotExisted,
        }
    }
}

impl TryFrom<String> for TweetFailReason {
    type Error = ();

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        let value = value.as_str();
        match value {
            "restricted" => Ok(Self::Restricted),
            "deleted" => Ok(Self::Deleted),
            "account suspended" => Ok(Self::AccountSuspended),
            "account not existed" => Ok(Self::AccountNotExisted),
            _ => Err(()),
        }
    }
}

#[derive(Clone)]
pub struct TweetDB {
    conn_pool: r2d2::Pool<SqliteConnectionManager>,
}

impl TweetDB {
    pub fn new(db_path: &Path) -> Result<Self> {
        if db_path.exists() {
            if !db_path.is_file() {
                return Err(Error::CustomError {
                    msg: "DB is not a file.".to_string(),
                }
                .into());
            }
            let db = SqliteConnectionManager::file(db_path);
            let conn_pool = r2d2::Pool::builder()
                .connection_timeout(Duration::from_secs(2 * 60 * 60))
                .build(db)?;
            Ok(Self { conn_pool })
        } else {
            let db = SqliteConnectionManager::file(db_path);
            let conn_pool = r2d2::Pool::builder()
                .connection_timeout(Duration::from_secs(2 * 60 * 60))
                .build(db)?;
            conn_pool.get()?.execute_batch(
                r#"
CREATE TABLE "tweet" (
	"id"	INTEGER NOT NULL UNIQUE,
	"author"	TEXT NOT NULL,
	"content"	TEXT NOT NULL,
	"create_time"	TIMESTAMP NOT NULL,
	"index_time"	TIMESTAMP NOT NULL DEFAULT (STRFTIME('%s', 'now')),
	"fetch_time"	TIMESTAMP NOT NULL DEFAULT (STRFTIME('%s', 'now')),
	PRIMARY KEY("id")
);
CREATE TABLE "media" (
	"id"	TEXT NOT NULL UNIQUE,
	"tweet_id"	INTEGER NOT NULL,
	"url"	TEXT NOT NULL UNIQUE,
	"width" INTEGER,
	"height" INTEGER,
	"no"	INTEGER,
	"type" TEXT,
	PRIMARY KEY("id")
);
CREATE TABLE "thread" (
    "tweet_id"          INTEGER NOT NULL UNIQUE,
    "thread_master_id"  INTEGER NOT NULL,
	"in_reply_to"	    INTEGER,
	PRIMARY KEY("tweet_id"),
	FOREIGN KEY("tweet_id") REFERENCES "tweet"("id"),
	FOREIGN KEY("thread_master_id") REFERENCES "tweet"("id"),
	FOREIGN KEY("in_reply_to") REFERENCES "tweet"("id")
);
CREATE TABLE "fail" (
    "id" INTEGER,
    "tweet_id" INTEGER NOT NULL,
    "url" TEXT NOT NULL,
    "type" TEXT NOT NULL CHECK ("type" IN ('restricted', 'deleted', 'account suspended', 'account not existed')),
    PRIMARY KEY("id")
);
                "#,
            )?;
            Ok(Self { conn_pool })
        }
    }

    pub fn is_exist(&self, id: u64) -> bool {
        let conn = self.conn_pool.get().unwrap();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tweet WHERE id=?1) OR EXISTS(SELECT 1 FROM fail WHERE tweet_id=?1);",
            params![id],
            |v| v.get(0),
        )
        .unwrap()
    }

    fn do_rusqlite_error<S: AsRef<str>>(
        err_title: S,
        err: rusqlite::Error,
        allow_sql_errcode: Option<rusqlite::ErrorCode>,
    ) {
        if let Some(allow) = allow_sql_errcode {
            if let rusqlite::Error::SqliteFailure(rusqlite::ffi::Error { code: c, .. }, _) = err {
                if c == allow {
                    // allow
                } else {
                    error!("{}: {}", err_title.as_ref(), err.to_string());
                }
            } else {
                error!("{}: {}", err_title.as_ref(), err.to_string());
                panic!();
            }
        } else {
            error!("{}: {}", err_title.as_ref(), err.to_string());
            panic!();
        }
    }

    pub fn insert_tweet(&self, tweet: &Tweet) {
        let conn = self.conn_pool.get().unwrap();
        if let Err(e) = conn.execute(
            r#"INSERT INTO tweet 
                    (id, author, content, create_time) 
                    VALUES (?1, ?2, ?3, ?4);"#,
            params![tweet.id, tweet.author, tweet.content, tweet.create_time],
        ) {
            Self::do_rusqlite_error(
                format!("Error when inserting tweet {}/{}", tweet.author, tweet.id),
                e,
                Some(rusqlite::ErrorCode::ConstraintViolation),
            );
        }
    }

    pub fn get_tweet(&self, id: u64) -> Result<Tweet> {
        let conn = self.conn_pool.get().unwrap();
        let t = conn.query_row(
            "SELECT author, content, create_time FROM tweet WHERE id = ?",
            params![id],
            |row| {
                Ok(Tweet {
                    id,
                    author: row.get(0)?,
                    content: row.get(1)?,
                    create_time: row.get(2)?,
                })
            },
        );
        if let Err(e) = t {
            let err: String = if let Ok(err) = conn.query_row(
                "SELECT type FROM fail WHERE tweet_id = ?",
                params![id],
                |row| Ok(row.get(0)?),
            ) {
                err
            } else {
                return Err(Error::CustomError {
                    msg: "Not exists in TwDB.".to_string(),
                }
                .into());
            };
            if let Ok(fail) = TweetFailReason::try_from(err) {
                let err: Error = fail.into();
                Err(err.into())
            } else {
                Err(Error::DBError.into())
            }
        } else {
            Ok(t.unwrap())
        }
    }

    pub fn insert_media(&self, media: &Media) {
        let conn = self.conn_pool.get().unwrap();
        if let Err(e) = conn.execute(
            r#"INSERT INTO media 
                    (id, tweet_id, url, width, height, no, type) 
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);"#,
            params![
                media.id,
                media.tweet_id,
                media.url,
                media.width,
                media.height,
                media.no,
                media._type
            ],
        ) {
            Self::do_rusqlite_error(
                format!("Error when inserting media {}/{}", media.tweet_id, media.id),
                e,
                Some(rusqlite::ErrorCode::ConstraintViolation),
            );
        }
    }

    pub fn get_medias(&self, tweet_id: u64) -> Result<Vec<Media>> {
        let conn = self.conn_pool.get().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, url, width, height, no, type FROM media WHERE tweet_id=?;")?;
        let result = stmt
            .query_map(params![tweet_id], |row| {
                Ok(Media {
                    id: row.get(0)?,
                    tweet_id,
                    url: row.get(1)?,
                    width: row.get(2)?,
                    height: row.get(3)?,
                    no: row.get(4)?,
                    _type: row.get(5)?,
                })
            })?
            .map(|v| v.unwrap())
            .collect();

        Ok(result)
    }

    pub fn insert_thread(&self, thread_info: &ThreadInfo) {
        let conn = self.conn_pool.get().unwrap();
        if let Err(e) = conn.execute(
            r#"INSERT INTO thread 
                    (tweet_id, thread_master_id, in_reply_to) 
                    VALUES (?1, ?2, ?3);"#,
            params![
                thread_info.tweet_id,
                thread_info.thread_id,
                thread_info.reply_to
            ],
        ) {
            Self::do_rusqlite_error(
                format!("Error when inserting thread {}", thread_info.tweet_id),
                e,
                Some(rusqlite::ErrorCode::ConstraintViolation),
            );
        }
    }

    pub fn insert_fail(&self, url: &str, reason: TweetFailReason) {
        let id = extract_twitter_url(url).ok_or(Error::NotATweet).unwrap().1;
        let conn = self.conn_pool.get().unwrap();
        if let Err(e) = conn.execute(
            r#"INSERT INTO fail 
                    (tweet_id, url, type) 
                    VALUES (?1, ?2, ?3);"#,
            params![id, url, reason.to_string()],
        ) {
            error!("Error when inserting fail {}: {}", url, e.to_string());
        }
    }
}
