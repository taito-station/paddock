pub fn is_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_http_and_https() {
        assert!(is_http_url("http://example.com/a.pdf"));
        assert!(is_http_url("https://example.com/a.pdf"));
    }

    #[test]
    fn rejects_local_paths() {
        assert!(!is_http_url("pdfs/inbox/a.pdf"));
        assert!(!is_http_url("/abs/path/a.pdf"));
        assert!(!is_http_url("./a.pdf"));
        assert!(!is_http_url(""));
    }
}
