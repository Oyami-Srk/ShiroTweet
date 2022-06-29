#![allow(unused)]
#![allow(dead_code)]

use crate::tweet_db::{Media, ThreadInfo, Tweet, TweetDB, TweetFailReason};
use crate::tweet_fetcher::{TweetDownloadDB, TweetFetcher};
use crate::utils::{extract_twitter_url, Error};
use anyhow::Result;
use lazy_static::lazy_static;
use log::{info, LevelFilter};
use rayon::prelude::*;
use regex::Regex;
use rusqlite::params;
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

mod tweet_db;
mod tweet_fetcher;
mod tweet_parser;
mod twitter_def;
mod utils;

/*
   Suspended: https://twitter.com/longlong_114/status/1496448495527796736
   NotExists: https://twitter.com/oooowasaki/status/1496179502032031754
   Normal: https://twitter.com/kagurayukina1/status/1496364341897179136
   thread: https://twitter.com/onlyyougts/status/1531582206900064256
*/

lazy_static! {
    static ref URL_EXTRACTOR: Regex =
        Regex::new(r#"(https://twitter.com/.*?/status/\d+)\b"#).unwrap();
}

fn run_url_downloader<P: AsRef<Path>>(
    url_list_path: P,
    dl_db_file_path: P,
    tw_db_file_path: P,
    login_creds: (Option<String>, Option<String>, Option<String>),
    no_login: bool,
    manual_login: bool,
    no_headless: bool,
) -> Result<()> {
    info!("Setup un-login fetcher.");
    let unlogin_fetcher = TweetFetcher::new("./chrome-data", !no_headless)?;
    let logged_in_fetcher = if no_login {
        None
    } else {
        info!("Setup logged in fetcher");
        let fetcher = TweetFetcher::new("./chrome-data-logined", !no_headless)?;
        if let Some(username) = fetcher.get_username()? {
            info!("Alread logged in as user `{}`", username);
        } else {
            info!("Not logged in, process login procudure.");

            let (username, password, vname) = login_creds;

            let login_cred = if manual_login {
                None
            } else {
                let username = if let Some(username) = username {
                    username
                } else {
                    println!("You are not specified to manually login. But no username given.");
                    print!("Enter your username (empty to use manual login): ");
                    std::io::stdout().flush().unwrap();
                    let mut username = String::new();
                    io::stdin().read_line(&mut username).unwrap();
                    if username.ends_with('\n') {
                        username.remove(username.len() - 1);
                    }
                    username
                };
                if username.is_empty() {
                    None
                } else {
                    let password = if let Some(password) = password {
                        password
                    } else {
                        loop {
                            print!("Enter your password please: ");
                            std::io::stdout().flush().unwrap();
                            let password = if let Ok(s) = read_password() {
                                s
                            } else {
                                println!("\nThere is an error about hidden input of password. Could you input it as plaintext? (If it's not safe, Ctrl-C and try to use another terminal.)");
                                let mut password = String::new();
                                io::stdin().read_line(&mut password).unwrap();
                                if password.ends_with('\n') {
                                    password.remove(password.len() - 1);
                                }
                                password
                            };
                            if password.is_empty() {
                                print!("Password empty! ReEnter your password please: ");
                                continue;
                            }
                            break password;
                        }
                    };
                    Some((username, password, vname))
                }
            };

            fetcher.login(login_cred)?;
        }
        Some(fetcher)
    };

    let is_tw_db_existed = if tw_db_file_path.as_ref().exists() {
        true
    } else {
        false
    };

    let db = TweetDB::new(tw_db_file_path.as_ref())?;
    let dldb = TweetDownloadDB::new(dl_db_file_path);

    info!("Reading url list from {}", url_list_path.as_ref().display());
    let mut urls = std::fs::read_to_string(url_list_path)?
        .lines()
        .map(|v| {
            if let Some(m) = URL_EXTRACTOR.captures(v) {
                Some(m.get(1).unwrap().as_str().to_string())
            } else {
                None
            }
        })
        .filter(|p| p.is_some())
        .map(|p| p.unwrap())
        .collect::<Vec<String>>();
    info!("Raw has {} entries.", urls.len());
    urls.sort();
    urls.dedup();
    info!("Sorted and deduped has {} entries.", urls.len());

    let urls = if is_tw_db_existed {
        info!("TweetDB is already existed. Remove item that already in db.");
        let urls = urls
            .into_par_iter()
            .filter(|p| {
                let id = utils::extract_twitter_url(p).unwrap().1;
                !db.is_exist(id)
            })
            .collect::<Vec<String>>();
        urls
    } else {
        urls
    };
    let total_len = urls.len();
    info!("{} to be downloaded.", total_len);

    info!("Using non-login fetcher for the first round.");

    let (succeed, failed) =
        tweet_fetcher::fetch_url_lists_to_sqlite(&unlogin_fetcher, urls, &dldb)?;
    info!(
        "Non-login succeed: {}, failed: {}, expected total: {}, actual total: {}. (Succeed is not always useful...)",
        succeed.len(),
        failed.len(),
        total_len,
        succeed.len() + failed.len()
    );

    let remaining = Arc::new(Mutex::new(Vec::from(failed)));

    let success_count = Arc::new(Mutex::new(0));
    let account_suspended_count = Arc::new(Mutex::new(0));
    let account_not_existed_count = Arc::new(Mutex::new(0));
    let restricted_count = Arc::new(Mutex::new(0));
    let deleted_count = Arc::new(Mutex::new(0));

    let tweet_without_media = Arc::new(Mutex::new(Vec::new()));

    let status_printer = || {
        info!("-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-");
        info!("Success: {}", success_count.lock().unwrap());
        info!("Remaining: {}", remaining.lock().unwrap().len());
        info!(
            "Account suspended: {}",
            account_suspended_count.lock().unwrap()
        );
        info!(
            "Account not existed: {}",
            account_not_existed_count.lock().unwrap()
        );
        info!("Deleted: {}", deleted_count.lock().unwrap());
        info!("Restricted: {}", restricted_count.lock().unwrap());
        info!("-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-");
    };

    let processor = |url: &str, retry_restricted: bool| {
        let id = extract_twitter_url(url).unwrap().1;
        let json: String = dldb.get_json(id).unwrap();

        let tweets_result = tweet_parser::extract_all_tweets(
            id.to_owned(),
            &serde_json::from_str(json.as_str()).unwrap(),
        );

        if let Ok(tweet) = tweets_result {
            // Tweet OK
            let thread = tweet_parser::get_thread(id, &tweet);
            let (tweets, medias, threads) = if let Some(ids) = thread {
                let thread_tweets = ids
                    .into_iter()
                    .map(|v| tweet.get(&v).unwrap())
                    .collect::<Vec<&TweetItem>>();
                let medias = thread_tweets
                    .iter()
                    .map(|v| v.get_medias())
                    .flatten()
                    .collect::<Vec<Media>>();
                let tweets = thread_tweets
                    .iter()
                    .map(|v| v.as_tweet())
                    .collect::<Vec<Tweet>>();
                let threads = thread_tweets
                    .iter()
                    .map(|v| v.as_thread())
                    .filter(|p| p.is_some())
                    .map(|v| v.unwrap())
                    .collect::<Vec<ThreadInfo>>();
                (tweets, medias, threads)
            } else {
                let tweet = tweet.get(&id).unwrap();
                (vec![tweet.as_tweet()], tweet.get_medias(), vec![])
            };

            if medias.is_empty() {
                tweet_without_media.lock().unwrap().push(url.to_string());
            }

            // insert into db
            tweets.iter().for_each(|tweet| db.insert_tweet(tweet));
            medias.iter().for_each(|media| db.insert_media(media));
            threads.iter().for_each(|thread| db.insert_thread(thread));
            // succeed
            *success_count.lock().unwrap() += 1;
        } else {
            let err = tweets_result.err().unwrap();
            // println!("Failed, because: {}", err.to_string());
            if let Some(err) = err.downcast_ref::<Error>() {
                if let Some(fail) = err.try_make_fail_reason() {
                    match fail {
                        TweetFailReason::Restricted => {
                            if retry_restricted {
                                remaining.lock().unwrap().push(url.to_string());
                            } else {
                                *restricted_count.lock().unwrap() += 1
                            }
                        }
                        TweetFailReason::Deleted => *deleted_count.lock().unwrap() += 1,
                        TweetFailReason::AccountSuspended => {
                            *account_suspended_count.lock().unwrap() += 1
                        }
                        TweetFailReason::AccountNotExisted => {
                            *account_not_existed_count.lock().unwrap() += 1
                        }
                    }
                    // insert fail into twdb
                    if let TweetFailReason::Restricted = fail {
                        if !retry_restricted {
                            db.insert_fail(url, fail);
                        }
                    } else {
                        db.insert_fail(url, fail);
                    }
                } else {
                    remaining.lock().unwrap().push(url.to_string());
                }
            }
        }
    };

    let progress_count = Arc::new(Mutex::new(0));
    let total = succeed.len();
    info!("Try parse and move succeed items to TweetDB.");
    succeed.iter().for_each(|url| {
        let mut progress_count = progress_count.lock().unwrap();
        *progress_count += 1;
        info!("[{}/{}] Processing {}", progress_count, total, url);
        drop(progress_count);
        processor(url.as_str(), true);
    });

    info!("Total: {}", progress_count.lock().unwrap());
    status_printer();

    let mut retries = 0;

    if let Some(logged_in_fetcher) = logged_in_fetcher {
        while !remaining.lock().unwrap().is_empty() && retries < 5 {
            let mut remaining = remaining.lock().unwrap();
            info!("Remaining tweets: {}", remaining.len());
            info!("Using logged in fetcher.");
            if retries != 0 {
                info!("Retry times: {}", retries);
            }
            retries += 1;
            info!("Clear old download db entries.");
            remaining.iter().for_each(|url| {
                let id = extract_twitter_url(url).unwrap().1;
                dldb.remove(id);
            });
            info!("Run fetcher");
            let total_len = remaining.len();
            let (succeed, failed) = tweet_fetcher::fetch_url_lists_to_sqlite(
                &logged_in_fetcher,
                remaining.clone(),
                &dldb,
            )?;

            info!("Logged-in succeed: {}, failed: {}, expected total: {}, actual total: {}. (Succeed is not always useful...)",
                succeed.len(),
                failed.len(),
                total_len,
                succeed.len() + failed.len()
            );

            remaining.clear();
            remaining.extend(failed.into_iter());

            info!("Try parse and move succeed items to TweetDB.");
            *progress_count.lock().unwrap() = 0;
            let total = succeed.len();
            succeed.iter().for_each(|url| {
                let mut progress_count = progress_count.lock().unwrap();
                *progress_count += 1;
                info!("[{}/{}] Processing {}", progress_count, total, url);
                drop(progress_count);
                processor(url.as_str(), false);
            });
            info!("Total: {}", progress_count.lock().unwrap());
            status_printer();
        }
    }

    // done
    tweet_without_media.lock().unwrap().iter().for_each(|url| {
        info!("No media tweet: {}", url);
    });
    Ok(())
}

use crate::tweet_parser::TweetItem;
use clap::{CommandFactory, Parser, ValueHint};
use rpassword::read_password;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "todo.txt", value_hint = ValueHint::FilePath)]
    url_list: PathBuf,
    #[clap(short = 'd', long, default_value = "dl.sqlite", value_hint = ValueHint::FilePath)]
    download_db: PathBuf,
    #[clap(short = 't', long, default_value = "tw.sqlite", value_hint = ValueHint::FilePath)]
    tweet_db: PathBuf,
    #[clap(short = 'u', long)]
    username: Option<String>,
    #[clap(short = 'p', long)]
    password: Option<String>,
    #[clap(long)]
    verification_username: Option<String>,
    #[clap(long, action)]
    no_login: bool,
    #[clap(long, action)]
    manual_login: bool,
    #[clap(long, action)]
    no_headless: bool,
}

fn main() {
    env_logger::builder()
        .format(|buf, record| {
            writeln!(
                buf,
                "[{}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .filter_module("shirotweet_fetcher", LevelFilter::Info)
        // .filter_module("headless_chrome", LevelFilter::Debug)
        .init();
    info!("ShiroTweets version {}", env!("CARGO_PKG_VERSION"));

    let args: Args = Args::parse();
    if !args.url_list.exists() || !args.url_list.is_file() {
        Args::command()
            .error(
                clap::ErrorKind::ArgumentConflict,
                format!("Url list file `{}` not exists.", args.url_list.display()),
            )
            .exit();
    }

    // run_dl_db_parser("./dl.sqlite");

    if let Err(e) = run_url_downloader(
        args.url_list,
        args.download_db,
        args.tweet_db,
        (args.username, args.password, args.verification_username),
        args.no_login,
        args.manual_login,
        args.no_headless,
    ) {
        panic!("Error happen when run url downloader: {}", e);
    }
}
