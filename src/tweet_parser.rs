use std::collections::HashMap;
use std::fmt::Formatter;

use anyhow::Result;
use chrono::DateTime;
use log::{error, trace, warn};
use serde::Deserialize;

use crate::tweet_db::{Media, ThreadInfo, Tweet};
use crate::twitter_def;
use crate::utils::Error;

type JObj = serde_json::Value;
type JRawValue = serde_json::value::RawValue;

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetMediaOriginalInfo {
    pub height: u64,
    pub width: u64,
}

fn default_bitrate() -> u64 {
    0
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetVideoInfoVariant {
    #[serde(default = "default_bitrate")]
    pub bitrate: u64,
    pub url: String,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetVideoInfo {
    pub variants: Vec<TweetVideoInfoVariant>,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetMedia {
    pub display_url: String,
    pub expanded_url: String,
    pub id_str: String,
    pub indices: Vec<u64>,
    pub media_url_https: String,
    #[serde(rename = "type")]
    pub _type: String,
    pub url: String,
    // don't need
    pub features: Option<Box<JRawValue>>,
    // don't need
    pub sizes: Option<Box<JRawValue>>,
    pub original_info: TweetMediaOriginalInfo,
    // only existed in extended_entities
    pub video_info: Option<TweetVideoInfo>,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetHashTag {
    pub indices: Vec<u64>,
    pub text: String,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetEntities {
    pub media: Option<Vec<TweetMedia>>,
    pub user_mentions: Option<Vec<Box<JRawValue>>>,
    pub urls: Option<Vec<Box<JRawValue>>>,
    pub hashtags: Option<Vec<TweetHashTag>>,
    pub symbols: Option<Vec<Box<JRawValue>>>,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetSelfThread {
    pub id_str: String,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetLegacy {
    pub created_at: String,
    pub id_str: String,
    pub user_id_str: String,
    pub conversation_id_str: String,
    pub full_text: String,
    pub source: Option<String>,
    pub lang: String,
    pub display_text_range: Vec<u64>,
    pub favorite_count: u64,
    pub favorited: bool,
    pub quote_count: u64,
    pub is_quote_status: bool,
    pub reply_count: u64,
    pub retweet_count: u64,
    pub retweeted: bool,
    pub possibly_sensitive: Option<bool>,
    pub possibly_sensitive_editable: Option<bool>,
    pub self_thread: Option<TweetSelfThread>,
    pub in_reply_to_screen_name: Option<String>,
    pub in_reply_to_status_id_str: Option<String>,
    pub in_reply_to_user_id_str: Option<String>,
    pub entities: TweetEntities,
    pub extended_entities: Option<TweetEntities>,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetUserLegacy {
    name: String,
    screen_name: String,
}

#[derive(Deserialize)]
#[allow(unused)]
pub struct TweetUser {
    #[serde(rename = "__typename")]
    typename: String,
    legacy: TweetUserLegacy,
}

#[derive(Deserialize)]
pub struct TweetCoreUserResults {
    result: TweetUser,
}

#[derive(Deserialize)]
pub struct TweetCore {
    user_results: TweetCoreUserResults,
}

#[derive(Deserialize)]
pub struct TweetItem {
    #[serde(rename = "__typename", default = "tweet_type_default")]
    pub type_name: String,
    pub rest_id: String,
    pub core: TweetCore,
    pub legacy: TweetLegacy,
}

fn tweet_type_default() -> String {
    "Tweet".to_string()
}

impl std::fmt::Debug for TweetItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let media_urls = self
            .legacy
            .entities
            .media
            .as_ref()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|v| v.media_url_https.clone())
            .collect::<Vec<String>>();
        if media_urls.is_empty() {
            write!(f, "TweetItem<{}>", self.rest_id,)
        } else {
            write!(f, "TweetItem<{}>[{}]", self.rest_id, media_urls.join(", "))
        }
    }
}

impl TweetItem {
    pub fn as_tweet(&self) -> Tweet {
        Tweet {
            id: self.rest_id.parse().unwrap(),
            author: self.core.user_results.result.legacy.screen_name.clone(),
            content: self.legacy.full_text.clone(),
            create_time: DateTime::parse_from_str(
                self.legacy.created_at.as_str(),
                "%a %b %d %H:%M:%S %z %Y",
            )
            .map_or(0, |dt| dt.timestamp() as u64),
        }
    }

    pub fn as_thread(&self) -> Option<ThreadInfo> {
        if self.legacy.self_thread.is_none() || self.legacy.in_reply_to_status_id_str.is_none() {
            None
        } else {
            Some(ThreadInfo {
                tweet_id: self.rest_id.parse().unwrap(),
                thread_id: self
                    .legacy
                    .self_thread
                    .as_ref()
                    .unwrap()
                    .id_str
                    .parse()
                    .unwrap(),
                reply_to: self
                    .legacy
                    .in_reply_to_status_id_str
                    .as_ref()
                    .unwrap()
                    .parse()
                    .unwrap(),
            })
        }
    }

    pub fn get_medias(&self) -> Vec<Media> {
        let medias = if let Some(medias) = &self.legacy.extended_entities {
            medias
        } else {
            &self.legacy.entities
        };

        let medias = if let Some(medias) = &medias.media {
            medias
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let url = if v._type == "animated_gif" || v._type == "video" {
                        let video_info = v.video_info.as_ref().unwrap();
                        video_info
                            .variants
                            .iter()
                            .max_by_key(|v| v.bitrate)
                            .unwrap()
                            .url
                            .clone()
                    } else {
                        v.media_url_https.clone()
                    };
                    debug_assert!(!url.is_empty());
                    Media {
                        id: v.id_str.clone(),
                        tweet_id: self.rest_id.parse().unwrap(),
                        url,
                        width: v.original_info.width,
                        height: v.original_info.height,
                        no: (i + 1) as i32,
                        _type: v._type.to_string(),
                    }
                })
                .collect()
        } else {
            vec![]
        };
        debug_assert!(medias.len() == self.legacy.entities.media.as_ref().unwrap_or(&vec![]).len());
        medias
    }
}

pub fn extract_all_tweets(id: u64, obj: &JObj) -> Result<HashMap<u64, TweetItem>> {
    let obj = obj.as_object().ok_or(Error::TweetJsonSchemaInvalid)?;

    let timeline_add_entries = if obj.contains_key("errors") {
        let errors = obj["errors"]
            .as_array()
            .ok_or(Error::TweetJsonSchemaInvalid)?;
        for error in errors {
            let message = error["message"].as_str().unwrap_or("");
            if message.contains(twitter_def::TWEET_ERROR_MESSAGE_DELETED) {
                return Err(Error::TweetNotExists.into());
            } else {
                continue;
            }
        }

        obj.get("data")
            .ok_or(Error::TweetJsonSchemaInvalid)?
            .get("threaded_conversation_with_injections_v2")
            .ok_or(Error::TweetJsonSchemaInvalid)?
            .get("instructions")
            .ok_or(Error::TweetJsonSchemaInvalid)?
    } else {
        obj.get("data")
            .ok_or(Error::TweetJsonSchemaInvalid)?
            .get("threaded_conversation_with_injections_v2")
            .ok_or(Error::TweetJsonSchemaInvalid)?
            .get("instructions")
            .ok_or(Error::TweetJsonSchemaInvalid)?
    }
    .as_array()
    .ok_or(Error::TweetJsonSchemaInvalid)?
    .iter()
    .filter(|i| {
        let i = i.as_object();
        i.is_some() && i.unwrap()["type"] == "TimelineAddEntries"
    })
    .collect::<Vec<&serde_json::Value>>();

    let entries = if timeline_add_entries.len() == 0 {
        return Err(Error::TweetJsonSchemaInvalid.into());
    } else if timeline_add_entries.len() > 1 {
        // todo!()
        return Err(Error::Todo("Timelime Add Entries more than once.".to_string()).into());
    } else {
        timeline_add_entries[0]["entries"]
            .as_array()
            .ok_or(Error::TweetJsonSchemaInvalid)?
    };

    let mut tweets: HashMap<u64, TweetItem> = HashMap::new();

    for entry in entries {
        let entry = entry.as_object().ok_or(Error::TweetJsonSchemaInvalid)?;
        let content = &entry["content"]
            .as_object()
            .ok_or(Error::TweetJsonSchemaInvalid)?;

        if content["entryType"] == "TimelineTimelineItem" {
            // single item
            let mut tweet = &content["itemContent"]["tweet_results"]["result"];
            let mut nested = false;
            if tweet["__typename"] == "TweetWithVisibilityResults" {
                tweet = &tweet["tweet"];
                nested = true;
            }
            if tweet["__typename"] != "Tweet" {
                if tweet["__typename"] == "TweetTombstone" {
                    let tweet_id = "tweet-".to_string() + &id.to_string();
                    let tombstone = tweet["tombstone"].as_object().unwrap();
                    if tombstone["__typename"] == "TextTombstone" {
                        let text = tombstone["text"]["text"].as_str().unwrap_or("");
                        trace!("TextTombstone: {}: {}", entry["entryId"], text);
                        if entry["entryId"]
                            .as_str()
                            .unwrap()
                            .eq_ignore_ascii_case(&tweet_id)
                        {
                            if text.contains(twitter_def::TEXT_TOMBSTONE_ACCOUNT_SUSPENDED) {
                                return Err(Error::TwitterAccountSuspended.into());
                            } else if text.contains(twitter_def::TEXT_TOMBSTONE_AUDLT_CONTENT) {
                                return Err(Error::TweetAdultContent.into());
                            } else if text.contains(twitter_def::TEXT_TOMBSTONE_USER_RESTRICTED) {
                                return Err(Error::TweetRestricted.into());
                            } else if text.contains(twitter_def::TEXT_TOMBSTONE_ACCOUNT_NOT_EXISTED)
                            {
                                return Err(Error::TwitterAccountNotExisted.into());
                            } else if text.contains(twitter_def::TEXT_TOMBSTONE_TWEET_ILLEGAL) {
                                return Err(Error::TweetIllegalBan.into());
                            } else if text.contains(twitter_def::TEXT_TOMBSTONE_TWEET_NOT_AVALIABLE)
                            {
                                return Err(Error::TweetNotExists.into());
                            } else {
                                return Err(Error::TweetUnknownError(text.to_string()).into());
                            }
                        }
                    } else {
                        return Err(Error::TweetUnknownError(
                            "Tombstone type unknown.".to_string(),
                        )
                        .into());
                    }
                }
                if !nested {
                    trace!(
                        "Entry {} is not a tweet, but {}.",
                        entry["entryId"],
                        tweet["__typename"]
                    );
                    continue;
                }
            }
            let tweet = TweetItem::deserialize(tweet).or_else(|v| {
                error!("{}", v);
                Err(Error::TweetJsonSchemaInvalid)
            })?;
            let id = tweet
                .rest_id
                .parse::<u64>()
                .or_else(|_v| Err(Error::TweetJsonSchemaInvalid))?;
            tweets.insert(id, tweet);
        } else if content["entryType"] == "TimelineTimelineModule" {
            // multiple item
            let items = &content["items"].as_array();
            if items.is_none() {
                warn!("Entry {} have no items.", entry["entryId"]);
                continue;
            }
            let items = items.unwrap();
            for item in items {
                let mut tweet = &item["item"]["itemContent"]["tweet_results"]["result"];
                let mut nested = false;
                if tweet["__typename"] == "TweetWithVisibilityResults" {
                    tweet = &tweet["tweet"];
                    nested = true;
                }
                if tweet["__typename"] != "Tweet" {
                    if !nested {
                        trace!(
                            "Entry {}, item {} is not a tweet. but {}.",
                            entry["entryId"],
                            item["entryId"],
                            tweet["__typename"]
                        );
                        continue;
                    }
                }
                let tweet = TweetItem::deserialize(tweet).or_else(|v| {
                    error!("{}", v);
                    Err(Error::TweetJsonSchemaInvalid)
                })?;
                let id = tweet
                    .rest_id
                    .parse::<u64>()
                    .or_else(|_v| Err(Error::TweetJsonSchemaInvalid))?;
                tweets.insert(id, tweet);
            }
        } else {
            // unimplemented!();
            return Err(Error::Unimplemented(format!(
                "Entry Type handler for {}",
                content["entryType"]
            ))
            .into());
        }
    }

    if !tweets.contains_key(&id) {
        Err(Error::TweetJsonSchemaInvalid.into())
    } else {
        Ok(tweets)
    }
}

pub fn get_thread(id: u64, tweets: &HashMap<u64, TweetItem>) -> Option<Vec<u64>> {
    if !tweets.contains_key(&id) {
        return None;
    }

    let thread_id = if let Some(thread_id) = &tweets[&id].legacy.self_thread {
        &thread_id.id_str
    } else {
        return None;
    };

    let with_same_id = tweets
        .iter()
        .filter(|(_tid, t)| {
            t.legacy.self_thread.is_some()
                && &t.legacy.self_thread.as_ref().unwrap().id_str == thread_id
        })
        .map(|(tid, _t)| tid.to_owned())
        .collect::<Vec<u64>>();
    if with_same_id.len() == 1 {
        None
    } else {
        Some(with_same_id)
    }
}
