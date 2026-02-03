use std::env;
use std::process::Command;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=.git/HEAD");

    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let build_date = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .and_then(|secs| OffsetDateTime::from_unix_timestamp(secs).ok())
        .unwrap_or_else(OffsetDateTime::now_utc)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown-date".to_string());

    println!("cargo:rustc-env=GIT_SHA={}", git_sha);
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);
}
