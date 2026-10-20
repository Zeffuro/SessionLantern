#[path = "../src/export.rs"]
mod export;

use export::{encrypt_export, Credentials};
use std::io::{self, Write};

fn main() -> Result<(), &'static str> {
    let credentials = Credentials {
        session_id: "fixture-session-value".into(),
        target_idc: "fixture-region-value".into(),
        ms_token: "fixture-token-value".into(),
        verify_fp: "fixture-fingerprint-value".into(),
    };
    let encrypted = encrypt_export(&credentials, "  synthetic café 🔑 fixture  ")
        .map_err(|_| "Synthetic export failed.")?;
    io::stdout()
        .write_all(&encrypted)
        .map_err(|_| "Synthetic export output failed.")
}
