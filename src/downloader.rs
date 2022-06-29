#![allow(dead_code, unused)]
use crate::tweet_db::TweetDB;
use crate::tweet_fetcher::TweetDownloadDB;
use crate::utils::Error;
use crate::utils::{extract_twitter_url, read_url_list};
use anyhow::Result;
use clap::{CommandFactory, Parser, ValueHint};
use console::{Emoji, Style};
use indicatif::{ProgressBar, ProgressStyle};
use lazy_static::lazy_static;
use log::{info, warn, LevelFilter};
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

mod tweet_db;
mod tweet_fetcher;
mod tweet_parser;
mod twitter_def;
mod utils;

use regex::Regex;
use shirodl;
use shirodl::{DownloadFailed, DownloadTask};

lazy_static! {
    static ref EXTRACTOR: Regex = Regex::new(r#".*/(.*?)(\?.*|$)"#).unwrap();
}

fn extract_fn(url: &str) -> &str {
    if let Some(m) = EXTRACTOR.captures(url) {
        m.get(1).unwrap().as_str()
    } else {
        ""
    }
}

fn is_need_orig(url: &str) -> bool {
    if let Some(m) = EXTRACTOR.captures(url) {
        if let Some(v) = m.get(2) {
            let filename = m.get(1).unwrap().as_str();
            if filename.ends_with(".mp4") {
                false
            } else {
                if v.as_str().len() > 1 {
                    false
                } else {
                    true
                }
            }
        } else {
            false
        }
    } else {
        false
    }
}

fn run_downloader<P: AsRef<Path>>(twdb: P, dest_dir: P) -> Result<()> {
    let dest_dir = dest_dir.as_ref();
    if !dest_dir.exists() {
        std::fs::create_dir_all(dest_dir);
    }
    let twdb = TweetDB::new(twdb.as_ref())?;
    let conn = twdb.get_db_conn();
    let mut stmt = conn.prepare(
        r#"SELECT t.author, m.url
                    FROM tweet AS t INNER JOIN media as m
                    WHERE t.id == m.tweet_id"#,
    )?;
    let mut tasks: Vec<DownloadTask> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap(),
                row.get::<_, String>(1).unwrap(),
            ))
        })?
        .take(10)
        .filter_map(|v| {
            let (author, url) = v.unwrap();
            let url = if is_need_orig(&url) {
                url + "?name=orig"
            } else {
                url
            };
            let filename = extract_fn(&url).to_string();
            // println!("{}/{} <== {}", author, filename, url);
            if dest_dir.join(&author).join(&filename).exists() {
                None
            } else {
                Some((url, PathBuf::from(author), Some(filename)).into())
            }
        })
        .collect();

    // test
    tasks.push(DownloadTask {
        url: "https://pbs.twimg.com/media/FR0utoaakaAUgnee.jpg?name=orig".to_string(),
        path: "test".into(),
        filename: Some("test.jpg".to_string()),
    });

    let mut unrecoverables: Vec<DownloadTask> = vec![];

    loop {
        let mut downloader = shirodl::Downloader::new();
        downloader.set_destination(dest_dir.to_path_buf());
        downloader.set_auto_rename(false);
        let bar = ProgressBar::new(tasks.len() as u64);
        while !tasks.is_empty() {
            downloader.append_task(tasks.pop().unwrap());
        }

        bar.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner}[{elapsed_precise}][{eta}] {wide_bar:.cyan/blue} [{pos}/{len} - {percent}%] {msg}",
                )?
                .progress_chars("##-"),
        );
        let (sender, receiver) = mpsc::channel();
        let retain_sender = sender.clone();
        bar.enable_steady_tick(Duration::from_millis(200));
        let display_thread = thread::spawn(move || loop {
            let msg = receiver.recv().unwrap();
            if let Some(s) = msg {
                bar.println(s);
                bar.inc(1);
            } else {
                bar.finish();
                break;
            }
        });

        let faileds = downloader
            .download(move |url, path, filename, err| {
                let msg_style = if let Some(e) = err {
                    if e.ignorable() {
                        Style::new().black().bright()
                    } else {
                        Style::new().red().bright().bold()
                    }
                } else {
                    Style::new().green()
                };
                let msg = format!(
                    "{} {}",
                    if err.is_none() {
                        Emoji::new("✔️", "[ Done ]")
                    } else {
                        Emoji::new("❌️", "[Failed]")
                    },
                    if err.is_none() {
                        url.to_string()
                    } else {
                        format!("{} [{}]", url, err.unwrap())
                    }
                );
                sender.send(Some(msg_style.apply_to(msg).to_string()));
            })
            .unwrap();

        retain_sender.send(None);
        display_thread.join().unwrap();

        let faileds: Vec<DownloadTask> = faileds
            .into_iter()
            .filter_map(|failed| {
                let err = &failed.err;
                if let shirodl::Error::ResourceNotFound = err {
                    unrecoverables.push(failed.into());
                    None
                } else {
                    Some(failed.into())
                }
            })
            .collect();

        if faileds.is_empty() {
            break;
        }
        tasks.extend(faileds.into_iter());
    }

    if !unrecoverables.is_empty() {
        let save_file = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S TweetDownloadFailures.txt")
            .to_string();
        println!(
            "There are {} item cannot be download. Saved to file {}.",
            unrecoverables.len(),
            save_file
        );
        let content: String = unrecoverables
            .into_iter()
            .map(|v| {
                format!(
                    "{} ==> {}/{}\n",
                    v.url,
                    v.path.display(),
                    v.filename.unwrap_or("".to_string())
                )
            })
            .collect();
        std::fs::write(save_file, content).unwrap();
    }
    Ok(())
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short = 't', long, default_value = "tw.sqlite", value_hint = ValueHint::FilePath)]
    tweet_db: PathBuf,
    #[clap(default_value = "TweetMedias", value_hint = ValueHint::DirPath)]
    dest_dir: PathBuf,
}

fn main() {
    println!("ShiroTweets version {}", env!("CARGO_PKG_VERSION"));

    let args: Args = Args::parse();

    if !args.tweet_db.exists() || !args.tweet_db.is_file() {
        Args::command()
            .error(
                clap::ErrorKind::ArgumentConflict,
                format!("TweetDB file `{}` not exists.", args.tweet_db.display()),
            )
            .exit();
    }
    if args.dest_dir.exists() && !args.dest_dir.is_dir() {
        Args::command()
            .error(
                clap::ErrorKind::ArgumentConflict,
                format!(
                    "Download destionation dir `{}` exists but not a dir.",
                    args.dest_dir.display()
                ),
            )
            .exit();
    }

    // run_dl_db_parser("./dl.sqlite");
    if let Err(e) = run_downloader(args.tweet_db, args.dest_dir) {
        panic!("Error happen when run downloader: {}", e);
    }
}
