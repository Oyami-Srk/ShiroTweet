use crate::tweet_db::TweetFailReason;
use crate::twitter_def;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
#[allow(unused)]
pub enum Error {
    CustomError { msg: String },
    LoginFailed { msg: String },
    TweetNotExists,
    TwitterAccountSuspended,
    TwitterAccountNotExisted,
    TweetAdultContent,
    TweetRestricted,
    NotATweet,
    TweetParseFailed(Option<String>),
    TweetUnknownError(String),
    JsonFailed(Option<String>),
    TweetJsonSchemaInvalid,
    Todo(String),
    Unimplemented(String),
    RateLimitExceeded,
    DBError,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::CustomError { msg } => write!(f, "ShiroTweet Error: {}.", msg),
            Error::LoginFailed { msg } => write!(f, "Login failed: {}.", msg),
            Error::TweetNotExists => write!(f, "Tweet does not exist."),
            Error::TwitterAccountSuspended => write!(f, "Twitter accound is suspended."),
            Error::TweetRestricted => write!(f, "Tweet is restricted by author."),
            Error::NotATweet => write!(f, "Url is not a tweet link."),
            Error::JsonFailed(msg) => {
                if let Some(msg) = msg {
                    write!(f, "Json failed: {}.", msg)
                } else {
                    write!(f, "Json failed.")
                }
            }
            Error::TweetUnknownError(msg) => write!(f, "Unknown error for tweet: {}.", msg),
            Error::TweetParseFailed(msg) => {
                if let Some(msg) = msg {
                    write!(f, "Tweet json data parse failed: {}.", msg)
                } else {
                    write!(f, "Tweet json data parse failed.")
                }
            }
            Error::TweetJsonSchemaInvalid => write!(f, "Tweet json schema invalid."),
            Error::Todo(msg) => write!(f, "Todo: {}.", msg),
            Error::Unimplemented(msg) => write!(f, "Unimplemented: {}.", msg),
            Error::RateLimitExceeded => write!(f, "Rate limit exceeded."),
            Error::TweetAdultContent => write!(f, "Tweet adult content, need login."),
            Error::TwitterAccountNotExisted => write!(f, "Twitter account not existed."),
            Error::DBError => write!(f, "Database error."),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn try_make_fail_reason(&self) -> Option<TweetFailReason> {
        match self {
            Self::TweetRestricted => Some(TweetFailReason::Restricted),
            Self::TweetNotExists => Some(TweetFailReason::Deleted),
            Self::TwitterAccountSuspended => Some(TweetFailReason::AccountSuspended),
            Self::TwitterAccountNotExisted => Some(TweetFailReason::AccountNotExisted),
            _ => None,
        }
    }
}

pub fn extract_twitter_url(url: &str) -> Option<(&str, u64)> {
    if let Some(capt) = twitter_def::TWEET_URL_EXTRACTOR.captures(url) {
        let username = capt.get(1).unwrap().as_str();
        let status_id = capt.get(2);
        if status_id.is_none() {
            None
        } else {
            let status_id = status_id.unwrap();
            let status_id = status_id.as_str().parse::<u64>();
            if status_id.is_err() {
                None
            } else {
                Some((username, status_id.unwrap()))
            }
        }
    } else {
        None
    }
}
