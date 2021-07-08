use std::error::Error;
use std::env;
use std::fs;
use std::ops::{Add, Sub};
use std::path::Path;
use std::process::exit;
use std::thread;
use std::time;
use notify_rust::Notification;
use reqwest::blocking as reqwest;
use ::reqwest::StatusCode;
use chrono::prelude::*;

const AW_DATE_FORMAT: &str = "%+";
const AW_QUERY_PART_1: &str = r#"{"query":["afk_events = query_bucket(find_bucket(\"aw-watcher-afk_\"));","afk_events = filter_keyvals(afk_events, \"status\", [\"not-afk\"]);","RETURN = sum_durations(afk_events);",";"],"timeperiods":[""#;
const AW_QUERY_PART_2: &str = r#""]}"#;

const EXTENSION_FILE: &str = ".time-limit-extension";
const DEFAULT_TIME: f64 = (7*60*60 + 30*60) as f64; // 7h30m
const DAY_START_OFFSET_HOURS: i64 = 4;
const RUN_INTERVAL: time::Duration = time::Duration::from_secs(60);

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();

    match args.first().map(|s| s.as_str()) {
        Some("daemon") => daemon(),
        Some("extend") => extend(&args[1..]),
        Some("status") => status(),
        _ => {
            eprintln!("Usage: `awtl daemon`, `awtl extend <time>`, or `awtl status`");
            exit(1);
        }
    }
}

fn daemon() {
    let mut limiter = TimeLimiter::new();
    let mut last_run = time::Instant::now();

    loop {
        println!("Running");

        if let Err(error) = limiter.run_next() {
            Notification::new()
                .summary("aw-time-limit error")
                .body(&format!("{}", error))
                .timeout(10000)
                .show()
                .unwrap();
            eprintln!("{}", error);
        }

        let now = time::Instant::now();
        let next_run = last_run + RUN_INTERVAL;

        if now < next_run {
            println!("Sleeping for {}ms", (next_run - now).as_millis());
            thread::sleep(next_run - now);
        }

        last_run = time::Instant::now();
    }
}

fn extend(args: &[String]) {
    let duration_text = match args.first() {
        Some(arg) => arg,
        None => {
            eprintln!("Usage: `awtl extend <time>`");
            exit(1);
        }
    };

    let multiplier: Option<f64> = match duration_text.as_bytes().last() {
        Some(b'h') => Some(3600.0),
        Some(b'm') => Some(60.0),
        Some(b's') => Some(0.0),
        Some(b'0'..=b'9') => None,
        None => {
            eprintln!("Empty time specified");
            exit(1);
        }
        _ => {
            eprintln!("Unrecognized time unit");
            exit(1);
        }
    };

    let duration_digits = if multiplier.is_some() {
        &duration_text[..duration_text.len() - 1]
    } else {
        duration_text.as_str()
    };

    let mut duration = match duration_digits.parse::<f64>() {
        Ok(duration) => duration,
        Err(error) => {
            eprintln!("Error parsing duration: {}", error);
            exit(1);
        },
    };

    if let Some(multiplier) = multiplier {
        duration *= multiplier;
    }

    let filepath = Path::new(&env::var("HOME").unwrap())
        .join(EXTENSION_FILE);

    fs::write(filepath, &format!("{} {}", get_today().format("%D"), duration)).unwrap();
}

fn status() {
    let mut limiter = TimeLimiter::new();
    let today = get_today();
    let today_naive = today.naive_local();

    limiter.load_extension(today_naive).unwrap();
    let seconds_today = limiter.query_aw(today).unwrap() as i64;

    let seconds_limit = DEFAULT_TIME as i64;
    let seconds_extended_limit = (DEFAULT_TIME + limiter.extension) as i64;

    println!("Active time: {}", seconds_to_string(seconds_today));
    println!(
        "Default limit: {} ({}%, {} left)",
        seconds_to_string(seconds_limit),
        seconds_today * 100 / seconds_limit,
        seconds_to_string(seconds_limit - seconds_today),
    );
    println!(
        "Extended limit: {} ({}%, {} left)",
        seconds_to_string(seconds_extended_limit),
        seconds_today * 100 / seconds_extended_limit,
        seconds_to_string(seconds_extended_limit - seconds_today),
    );
}

fn get_today() -> Date<Local> {
    // ActivityWatch starts days at 4:00am, so shift our times by that much too
    Local::now()
        .sub(chrono::Duration::hours(DAY_START_OFFSET_HOURS))
        .date()
}

fn seconds_to_string(mut seconds: i64) -> String {
    let negative = seconds < 0;
    if negative {
        seconds = -seconds;
    }

    let hours = seconds / 3600;
    seconds %= 3600;

    let minutes = seconds / 60;
    seconds %= 60;

    let mut output = String::new();

    if negative {
        output.push('-');
    }

    if hours > 0 {
        output += &format!("{}h", hours);
    }

    if minutes > 0 {
        output += &format!("{}m", minutes);
    }

    output += &format!("{}s", seconds);

    output
}

struct TimeLimiter {
    client: reqwest::Client,
    logged_overage_date: Option<NaiveDate>,
    extension: f64,
}

impl TimeLimiter {
    fn new() -> Self {
        TimeLimiter {
            client: reqwest::Client::new(),
            logged_overage_date: None,
            extension: 0.0,
        }
    }

    fn run_next(&mut self) -> Result<(), Box<dyn Error>> {
        let today = get_today();
        let today_naive = today.naive_local();

        self.load_extension(today_naive)?;

        let seconds = self.query_aw(today)?;

        if seconds > DEFAULT_TIME + self.extension {
            let new_overage = match self.logged_overage_date {
                None => true,
                Some(overage_date) => overage_date != today_naive,
            };

            if new_overage {
                self.logged_overage_date = Some(today_naive);

                let message;

                if self.extension > 0.0 {
                    message = "Reached extended active time limit for today";
                } else {
                    message = "Reached default active time limit for today";
                }

                Notification::new()
                    .summary("Active time limit reached")
                    .body(message)
                    .timeout(0)
                    .show()
                    .unwrap();
            }
        } else if self.logged_overage_date.is_some() {
            self.logged_overage_date = None;
        }

        Ok(())
    }

    fn query_aw(&self, date: Date<Local>) -> Result<f64, Box<dyn Error>> {
        let start_date = date
            .and_hms(0, 0, 0)
            .add(chrono::Duration::hours(DAY_START_OFFSET_HOURS));
        let end_date = start_date + chrono::Duration::days(1);

        let start = start_date.format(AW_DATE_FORMAT).to_string();
        let end = end_date.format(AW_DATE_FORMAT).to_string();

        let time_period = start + "/" + &end;
        let query = AW_QUERY_PART_1.to_owned() + &time_period + AW_QUERY_PART_2;

        let response = self.client.post("http://localhost:5600/api/0/query/")
            .body(query)
            .header("Accept", "application/json;charset=utf-8")
            .header("Content-Type", "application/json")
            .send()?;

        if response.status() != StatusCode::OK {
            return Err(format!("Bad response code: {}", response.status()).into());
        }

        let response_text = response.text()?;
        let seconds_active: f64 = response_text[1..response_text.len()-2].parse()?;

        Ok(seconds_active)
    }

    fn load_extension(&mut self, for_date: NaiveDate) -> Result<(), Box<dyn Error>> {
        let filepath = Path::new(&env::var("HOME").unwrap())
            .join(EXTENSION_FILE);

        if let Ok(ext_bytes) = fs::read(filepath) {
            let ext_text = String::from_utf8(ext_bytes)?;

            for line in ext_text.lines() {
                let (date_text, seconds_text) = match line.split_once(' ') {
                    Some(parts) => parts,
                    None => return Err("Invalid extension file".into()),
                };

                let date = NaiveDate::parse_from_str(date_text, "%D")?;
                let seconds = seconds_text.parse::<f64>()?;

                if date == for_date {
                    self.extension = seconds;
                    break;
                }
            }
        } else {
            self.extension = 0.0;
        }

        Ok(())
    }
}
