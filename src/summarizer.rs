#![allow(dead_code, unused)]
use crate::tweet_db::TweetDB;
use crate::tweet_fetcher::TweetDownloadDB;
use crate::utils::Error;
use crate::utils::{extract_twitter_url, read_url_list};
use anyhow::Result;
use clap::{CommandFactory, Parser, ValueHint};
use log::{info, warn, LevelFilter};
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod tweet_db;
mod tweet_fetcher;
mod tweet_parser;
mod twitter_def;
mod utils;

fn run_summarizer<P: AsRef<Path>>(url_list: P, dldb_path: P, twdb_path: P) -> Result<()> {
    let urls = read_url_list(&url_list)?;

    let list_total_count = urls.len();

    let success_count = Arc::new(Mutex::new(0));
    let account_suspended_count = Arc::new(Mutex::new(0));
    let account_not_existed_count = Arc::new(Mutex::new(0));
    let restricted_count = Arc::new(Mutex::new(0));
    let deleted_count = Arc::new(Mutex::new(0));

    let tweet_without_media = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let medias_count = Arc::new(Mutex::new(0));

    let other_failed = Arc::new(Mutex::new(Vec::<(String, String)>::new()));

    let status_printer = || {
        info!("-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-");
        info!("List Total: {}", list_total_count);
        let s = success_count.lock().unwrap();
        info!("Success: {}", s);
        let acc_sus = account_suspended_count.lock().unwrap();
        info!("Account suspended: {}", acc_sus);
        let acc_ne = account_not_existed_count.lock().unwrap();
        info!("Account not existed: {}", acc_ne);
        let del = deleted_count.lock().unwrap();
        info!("Deleted: {}", del);
        let res = restricted_count.lock().unwrap();
        info!("Restricted: {}", res);
        info!("Total: {}", *s + *acc_sus + *acc_ne + *del + *res);
        info!("===========================================================");
        info!("Medias total count: {}", medias_count.lock().unwrap());
        info!(
            "Tweets without media: {} ; their content:",
            tweet_without_media.lock().unwrap().len()
        );
        tweet_without_media
            .lock()
            .unwrap()
            .iter()
            .for_each(|(url, content)| info!("{}: {}", url, content));
        info!(
            "Failed tweets: {} ; the reason:",
            other_failed.lock().unwrap().len()
        );
        other_failed
            .lock()
            .unwrap()
            .iter()
            .for_each(|(url, content)| info!("{}: {}", url, content));
        info!("-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-*-");
    };

    let dldb = TweetDownloadDB::new(&dldb_path);
    let twdb = TweetDB::new(twdb_path.as_ref())?;

    let make_existed_url =
        |urls: Vec<String>, filter: Box<dyn Fn(&&String) -> bool + Sync + Send>, warn_msg: &str| {
            let not_in_dldb = urls
                .par_iter()
                .filter(|p| filter(p))
                .collect::<Vec<&String>>();

            if !not_in_dldb.is_empty() {
                warn!(
                    "Count of Tweets in url_list `{}` but not in {} `{}` is {}.",
                    url_list.as_ref().display(),
                    warn_msg,
                    dldb_path.as_ref().display(),
                    not_in_dldb.len()
                );
                let not_in_dldb = not_in_dldb
                    .into_par_iter()
                    .map(|v| v.clone())
                    .collect::<Vec<String>>();
                urls.into_par_iter()
                    .filter(|p| not_in_dldb.contains(&p))
                    .collect()
            } else {
                info!("Good, every tweet in url_list is inside {}.", warn_msg);
                urls
            }
        };

    let urls = make_existed_url(
        urls,
        Box::new(|p: &&String| !dldb.is_exist(extract_twitter_url(p).unwrap().1)),
        "Download DB",
    );
    let urls = make_existed_url(
        urls,
        Box::new(|p: &&String| !twdb.is_exist(extract_twitter_url(p).unwrap().1)),
        "Tweet DB",
    );

    urls.into_par_iter().for_each(|url| {
        let id = extract_twitter_url(&url).unwrap().1;
        let tweet = twdb.get_tweet(id);
        if let Ok(tweet) = tweet {
            *success_count.lock().unwrap() += 1;
            let medias = twdb.get_medias(id).unwrap();
            if medias.is_empty() {
                tweet_without_media
                    .lock()
                    .unwrap()
                    .push((url, tweet.content));
            } else {
                *medias_count.lock().unwrap() += medias.len();
            }
        } else {
            let err = tweet.unwrap_err();
            let err_str = err.to_string();
            if let Ok(err) = err.downcast::<Error>() {
                match err {
                    Error::TweetNotExists => *deleted_count.lock().unwrap() += 1,
                    Error::TweetRestricted => *restricted_count.lock().unwrap() += 1,
                    Error::TwitterAccountSuspended => *account_suspended_count.lock().unwrap() += 1,
                    Error::TwitterAccountNotExisted => {
                        *account_not_existed_count.lock().unwrap() += 1
                    }
                    _ => other_failed.lock().unwrap().push((url, err.to_string())),
                }
            } else {
                other_failed.lock().unwrap().push((url, err_str));
            }
        }
    });

    status_printer();

    Ok(())
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = "todo.txt", value_hint = ValueHint::FilePath)]
    url_list: PathBuf,
    #[clap(short = 'd', long, default_value = "dl.sqlite", value_hint = ValueHint::FilePath)]
    download_db: PathBuf,
    #[clap(short = 't', long, default_value = "tw.sqlite", value_hint = ValueHint::FilePath)]
    tweet_db: PathBuf,
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
        .filter_module("shirotweet_summarizer", LevelFilter::Info)
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
    if let Err(e) = run_summarizer(args.url_list, args.download_db, args.tweet_db) {
        panic!("Error happen when run summaryizer: {}", e);
    }
}
