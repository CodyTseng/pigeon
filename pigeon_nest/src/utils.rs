use std::time::SystemTime;

pub fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn extract_endpoint_from_url(url: &str) -> Option<String> {
    if let Ok(parsed_url) = url::Url::parse(url) {
        let scheme = parsed_url.scheme();
        let host = parsed_url.host_str()?;
        let port = parsed_url
            .port()
            .map(|p| format!(":{}", p))
            .unwrap_or_default();
        return Some(format!("{}://{}{}", scheme, host, port));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_timestamp() {
        let timestamp = unix_timestamp();
        assert!(timestamp > 0);
    }

    #[test]
    fn test_extract_endpoint_from_url() {
        let url = "ws://localhost:3000/register";
        assert_eq!(
            extract_endpoint_from_url(url),
            Some("ws://localhost:3000".to_string())
        );
    }
}
