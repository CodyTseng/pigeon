use once_cell::sync::Lazy;
use std::env;

pub struct Config {
    pub endpoint: String,
}

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config {
    endpoint: env::var("ENDPOINT").unwrap_or_else(|_| "ws://localhost:3000".to_string()),
});
