use super::twitter_def;
use super::utils::Error;
use anyhow::Result;
use headless_chrome::protocol::cdp::Fetch::{RequestPattern, RequestStage};
use headless_chrome::protocol::cdp::Network::ResourceType;
use headless_chrome::{Browser, LaunchOptions};
use lazy_static::lazy_static;
use regex::Regex;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::thread::sleep;
use std::time::Duration;

use crate::utils::extract_twitter_url;
use log::{debug, error, info, trace, warn};
use rusqlite::params;

pub struct TweetFetcher {
    browser_instance: Browser,
}

impl TweetFetcher {
    pub fn new<P: AsRef<Path>>(user_data_dir: P, headless: bool) -> Result<Self> {
        let browser = Browser::new(LaunchOptions {
            headless,
            idle_browser_timeout: Duration::from_secs(24 * 60 * 60),
            user_data_dir: Some(user_data_dir.as_ref().to_path_buf()),
            ..Default::default()
        })?;
        // nap a gap
        sleep(Duration::from_secs(1));
        Ok(Self {
            browser_instance: browser,
        })
    }

    pub fn get_username(&self) -> Result<Option<String>> {
        const ANALYTICS_URL: &str = "https://analytics.twitter.com/";
        const ANALYTICS_NONE_URL: &str = "https://analytics.twitter.com/about";
        let tab = self.browser_instance.wait_for_initial_tab()?;
        tab.navigate_to(ANALYTICS_URL)?;
        tab.wait_until_navigated()?;
        let jump_url = tab.get_url();
        if jump_url.contains(ANALYTICS_NONE_URL) {
            Ok(None)
        } else {
            lazy_static! {
                static ref REGEXP: Regex =
                    Regex::new(r#"https://analytics.twitter.com/user/(.*?)/"#).unwrap();
            }
            if let Some(cap) = REGEXP.captures(jump_url.as_str()) {
                if let Some(username) = cap.get(1).map(|v| v.as_str()) {
                    return Ok(Some(username.to_string()));
                }
            }
            return Err(Error::CustomError {
                msg: "Regex to capture username failed.".to_string(),
            }
            .into());
        }
    }

    pub fn login<S: AsRef<str>>(
        &self,
        // username: &str,
        // password: &str,
        // verification_username: Option<&str>,
        login_cred: Option<(S, S, Option<S>)>,
    ) -> Result<()> {
        let tab = self.browser_instance.wait_for_initial_tab()?;
        tab.navigate_to(twitter_def::LOGIN_URL)?;
        if let Some((username, password, verification_username)) = login_cred {
            let username = username.as_ref();
            let password = password.as_ref();
            info!("Shirotweet is using automatically login.");
            info!("Login with username {}", username);
            // Username Input
            let input = tab.wait_for_element_with_custom_timeout(
                twitter_def::LOGIN_USERNAME_SELECTOR,
                Duration::from_secs(10),
            )?;
            input.type_into(username)?;
            let btn = tab.wait_for_element(twitter_def::LOGIN_BUTTON_SELECTOR_NEXT)?;
            btn.click()?;
            // get next input box
            let input = tab.wait_for_element_with_custom_timeout(
                {
                    twitter_def::LOGIN_VALIDATE_SELECTOR.to_owned()
                        + ", "
                        + twitter_def::LOGIN_PASSWORD_SELECTOR
                }
                .as_str(),
                Duration::from_secs(10),
            )?;
            if let Some(attr) = input.get_attributes()? {
                let attr = attr
                    .iter()
                    .position(|r| r == "type")
                    .map(|v| attr[v + 1].as_str())
                    .unwrap_or("");
                if attr == "password" {
                    debug!("Login requires no need for verification, input password directly.");
                    input.type_into(password)?;
                } else {
                    debug!("Login need for verification.");
                    let vname = if let Some(vname) = verification_username {
                        vname.as_ref().to_string()
                    } else {
                        info!("You need to type your username for verification: ");
                        std::io::stdout().flush().unwrap();

                        let mut vname = String::new();
                        std::io::stdin().read_line(&mut vname).unwrap();
                        if vname.ends_with('\n') {
                            vname.remove(vname.len() - 1);
                        }

                        if vname.is_empty() {
                            return Err(Error::LoginFailed {
                                msg: "No verification provided.".to_string(),
                            }
                            .into());
                        }
                        vname
                    };
                    input.type_into(vname.as_str())?;
                    let btn = tab.find_element(twitter_def::LOGIN_BUTTON_SELECTOR_VERIFY)?;
                    btn.click()?;
                    let input = tab.wait_for_element_with_custom_timeout(
                        twitter_def::LOGIN_PASSWORD_SELECTOR,
                        Duration::from_secs(10),
                    )?;
                    debug!("Login input password.");
                    input.type_into(password)?;
                }
            } else {
                return Err(Error::CustomError {
                    msg: "No attribute for input box.".to_string(),
                }
                .into());
            }

            let btn = tab.find_element(twitter_def::LOGIN_BUTTON_SELECTOR_LOGIN)?;
            btn.click()?;
            while !tab.get_url().contains("home") {
                sleep(Duration::from_millis(500));
            }
        } else {
            info!("Shirotweet is using manually login.");
            info!("Please type enter after you get logged in.");
        }
        if let Ok(Some(username)) = self.get_username() {
            info!("Successfully logined as {}", username);
            Ok(())
        } else {
            error!("Login failed, can't get username.");
            Err(Error::LoginFailed {
                msg: "Can't get username".to_string(),
            }
            .into())
        }
    }

    fn __get_tweet(&self, url: &str) -> Result<String> {
        // Running in single process, only requiring one tab
        let tab = self.browser_instance.wait_for_initial_tab()?;
        let (tx, rx) = mpsc::sync_channel::<String>(1);

        const PATTERN_TWITTER_DETAILS: &str = "https://twitter.com/i/api/graphql/*";
        let patterns = vec![RequestPattern {
            url_pattern: Some(PATTERN_TWITTER_DETAILS.to_string()),
            resource_Type: Some(ResourceType::Xhr),
            request_stage: Some(RequestStage::Response),
        }];
        tab.enable_fetch(Some(&patterns), Some(false))?;

        let url_owned = url.to_owned();

        tab.register_response_handling(
            "handler",
            Box::new(move |resp, fetch_body| {
                let req_url = resp.response.url.as_str();
                if twitter_def::TWEET_JSON_URL_REGEXP.is_match(req_url) {
                    // contains what we need
                    sleep(Duration::from_millis(10));
                    let mut retries_counter = 0;
                    let body = loop {
                        let body = fetch_body();
                        if body.is_ok() {
                            break body.unwrap();
                        } else if retries_counter > 6 {
                            trace!("Give up for {}", url_owned);
                            return;
                        }
                        retries_counter += 1;
                        sleep(Duration::from_millis(500));
                    };
                    tx.send(body.body).unwrap();
                }
            }),
        )?;

        tab.navigate_to(url)?;
        let recv_result = rx.recv_timeout(Duration::from_secs(30));
        if let Ok(body) = recv_result {
            tab.stop_loading().unwrap();
            tab.disable_fetch().unwrap();
            tab.deregister_response_handling_all().unwrap();
            if !body.starts_with('{') {
                if body.contains("limit") {
                    Err(Error::RateLimitExceeded.into())
                } else {
                    Err(Error::CustomError {
                        msg: "Invalied TweetDetail return".to_string(),
                    }
                    .into())
                }
            } else {
                let obj: serde_json::Value = serde_json::from_str(body.as_str())
                    .map_err(|_v| Error::TweetJsonSchemaInvalid)?;
                let obj = obj.as_object().ok_or(Error::TweetJsonSchemaInvalid)?;
                if obj.contains_key("errors") {
                    for error in obj["errors"]
                        .as_array()
                        .ok_or(Error::TweetJsonSchemaInvalid)?
                    {
                        let msg = error
                            .get("message")
                            .map(|v| v.as_str().unwrap_or(""))
                            .unwrap_or("");
                        if msg.contains("Rate limit exceeded") {
                            return Err(Error::RateLimitExceeded.into());
                        } else if msg.contains("OverCapacity") {
                            return Err(Error::RateLimitExceeded.into());
                        }
                    }
                    Ok(body)
                } else {
                    Ok(body)
                }
            }
        } else {
            Err(Error::CustomError {
                msg: format!(
                    "Cannot wait for data: {}.",
                    recv_result.unwrap_err().to_string()
                )
                .to_string(),
            }
            .into())
        }
    }

    pub fn get_tweet<'a>(&self, url: &'a str) -> (&'a str, Result<String>) {
        if !url.starts_with("https://twitter.com/") {
            (url, Err(Error::NotATweet.into()))
        } else {
            (url, self.__get_tweet(url))
        }
    }

    #[allow(dead_code)]
    pub fn sleep(&self, dur: Duration) -> Result<()> {
        let tab = self.browser_instance.wait_for_initial_tab()?;
        tab.stop_loading()?;
        headless_chrome::util::Wait::with_sleep(dur)
            .until::<_, u64>(|| None)
            .unwrap();
        Ok(())
    }
}

pub struct TweetDownloadDB {
    conn_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
}

impl TweetDownloadDB {
    pub fn new<S: AsRef<Path>>(db_path: S) -> Self {
        let need_create = !db_path.as_ref().exists();
        let conn_pool = r2d2::Pool::builder()
            .connection_timeout(Duration::from_secs(2 * 60 * 60))
            .build(r2d2_sqlite::SqliteConnectionManager::file(db_path))
            .unwrap();
        if need_create {
            conn_pool
                .get()
                .unwrap()
                .execute_batch(
                    r#"
                        CREATE TABLE "tweet" (
                            id INTEGER PRIMARY KEY NOT NULL UNIQUE,
                            url TEXT NOT NULL UNIQUE,
                            json BLOB NOT NULL,
                    	    fetch_time	INTEGER NOT NULL DEFAULT strftime ("%s", "now")
                        );
                        "#,
                )
                .unwrap();
        }
        Self { conn_pool }
    }

    pub fn is_exist(&self, id: u64) -> bool {
        self.conn_pool
            .get()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tweet WHERE id=?1);",
                params![id],
                |v| v.get(0),
            )
            .unwrap_or(false)
    }

    pub fn insert(&self, id: u64, url: &str, json: &str) -> Result<()> {
        self.conn_pool.get().unwrap().execute(
            r#"INSERT INTO tweet (id, url, json) VALUES (?1, ?2, ?3)"#,
            params![id, url, json],
        )?;
        Ok(())
    }

    pub fn get_json(&self, id: u64) -> Result<String> {
        let json = self.conn_pool.get().unwrap().query_row(
            "SELECT json FROM tweet WHERE id = ?1;",
            params![id],
            |row| row.get(0),
        )?;
        Ok(json)
    }

    pub fn remove(&self, id: u64) -> Result<()> {
        self.conn_pool
            .get()
            .unwrap()
            .execute("DELETE FROM tweet WHERE id = ?1;", params![id])?;
        Ok(())
    }
}

pub fn fetch_url_lists_to_sqlite(
    fetcher: &TweetFetcher,
    urls: Vec<String>,
    dl_db: &TweetDownloadDB,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut failed: Vec<String> = vec![];
    let mut succeed: Vec<String> = vec![];
    let total = urls.len();
    let mut counter = 1;

    for url in urls {
        let id = extract_twitter_url(url.as_str()).unwrap().1;
        if dl_db.is_exist(id) {
            // already existed
            info!("[{}/{}] Existed: {}", counter, total, url);
            succeed.push(url);
            counter += 1;
            continue;
        }

        if counter % 100 == 0 {
            trace!("Every 100 tweet sleep 10 secs...");
            sleep(Duration::from_secs(10));
        }
        let mut retries_counter = 0;
        let json = loop {
            let (_, json) = fetcher.get_tweet(&url);
            if let Err(ref err) = json {
                if let Some(err) = err.downcast_ref::<Error>() {
                    if let Error::RateLimitExceeded = err {
                        if retries_counter == 0 {
                            warn!("First Rate limit exeeeded. Sleep 60 secs...");
                            sleep(Duration::from_secs(60));
                            info!("Continue...");
                        } else {
                            let secs_to_sleep = 600 + 120 * (retries_counter - 1);
                            warn!(
                                "{} times Rate limit exceeded. Sleep {} secs...",
                                retries_counter + 1,
                                secs_to_sleep
                            );
                            sleep(Duration::from_secs(secs_to_sleep));
                            info!("Continue...");
                        }
                        retries_counter += 1;
                        continue;
                    } else {
                        break json;
                    }
                }
            } else {
                break json;
            }
        };
        if let Ok(json) = json {
            let result = dl_db.insert(id, url.as_str(), json.as_str());
            if let Err(e) = result {
                error!("[{}/{}] DB Failed: {} for {}", counter, total, e, url);
                failed.push(url);
            } else {
                info!("[{}/{}] Done: {}", counter, total, url);
                succeed.push(url);
            }
        } else {
            let err = json.unwrap_err();
            error!("[{}/{}] Failed: {} for {}", counter, total, err, url);
            failed.push(url);
        }
        counter += 1;
        sleep(Duration::from_secs(1));
    }

    Ok((succeed, failed))
}
