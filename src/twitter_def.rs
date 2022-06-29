use lazy_static::lazy_static;
use regex::Regex;

pub const LOGIN_URL: &'static str = "https://twitter.com/i/flow/login";
pub const LOGIN_USERNAME_SELECTOR: &'static str = r#"input[autocomplete*="username"]"#;
pub const LOGIN_PASSWORD_SELECTOR: &'static str = r#"input[autocomplete*="password"]"#;
pub const LOGIN_VALIDATE_SELECTOR: &'static str = r#"input[data-testid="ocfEnterTextTextInput"]"#;
pub const LOGIN_BUTTON_SELECTOR_NEXT: &'static str =
    r#"div[role="button"][style*="background-color"]"#;
pub const LOGIN_BUTTON_SELECTOR_VERIFY: &'static str =
    r#"div[role="button"][data-testid="ocfEnterTextNextButton"]"#;
pub const LOGIN_BUTTON_SELECTOR_LOGIN: &'static str =
    r#"div[role="button"][data-testid="LoginForm_Login_Button"]"#;
lazy_static! {
    pub static ref TWEET_JSON_URL_REGEXP: Regex =
        Regex::new(r#"https://twitter.com/i/api/graphql/.*?/TweetDetail"#).unwrap();
    pub static ref TWEET_URL_EXTRACTOR: Regex =
        Regex::new(r#"https://twitter.com/(.*?)/status/(\d*)"#).unwrap();
}

pub const TEXT_TOMBSTONE_ACCOUNT_SUSPENDED: &'static str = r#"这条推文来自一个已冻结的账号"#;
pub const TEXT_TOMBSTONE_ACCOUNT_NOT_EXISTED: &'static str = r#"这条推文来自一个已不存在的账号。"#;
pub const TEXT_TOMBSTONE_AUDLT_CONTENT: &'static str =
    r#"受年龄限制的成人内容。这些内容可能不适合 18 岁以下的用户。"#;
pub const TEXT_TOMBSTONE_USER_RESTRICTED: &'static str =
    r#"该账号所有者限制了可以查看其推文的用户。"#;
pub const TWEET_ERROR_MESSAGE_DELETED: &'static str = r#"_Missing: No status found with that ID."#;
