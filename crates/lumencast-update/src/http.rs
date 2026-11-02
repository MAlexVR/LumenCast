//! The network side: asking GitHub for releases and downloading a file. Only
//! GitHub's own hosts are ever contacted, every redirect included, and the
//! access token (for a private repository) is only sent to `api.github.com`.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
const STALL: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;
/// The most a text file (the checksums, a signature) may weigh.
const MAX_TEXT: u64 = 1024 * 1024;
const MAX_JSON: u64 = 4 * 1024 * 1024;
/// The most an installer may weigh.
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
const API_HOST: &str = "api.github.com";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("not found (a private repository needs an access token)")]
    NotFound,
    #[error("GitHub refused the request (rate limit or permission), status {0}")]
    Refused(u16),
    #[error("unexpected status {0}")]
    Status(u16),
    #[error("{0}")]
    Network(String),
    #[error("the address {0} is not one of GitHub's")]
    Host(String),
    #[error("the answer is larger than allowed")]
    TooLarge,
    #[error("cancelled")]
    Cancelled,
}

/// Where the program gets files from (faked in tests).
pub trait Fetch: Send + Sync {
    fn text(&self, url: &str, accept: &str, limit: u64) -> Result<String, HttpError>;
    fn download(
        &self,
        url: &str,
        destination: &Path,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> Result<(), HttpError>;
}

/// The hosts a release and its files come from.
pub fn allowed_host(host: &str) -> bool {
    host == API_HOST
        || host == "github.com"
        || host == "objects.githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains('@') && !host.contains(':')).then_some(host)
}

pub struct UreqFetch {
    agent: ureq::Agent,
    token: Option<String>,
}

impl UreqFetch {
    pub fn new(token: Option<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(TIMEOUT)
            .timeout_read(STALL)
            .redirects(0)
            .user_agent(concat!("LumenCast-updater/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent,
            token: token.filter(|t| !t.is_empty()),
        }
    }

    /// Follows redirects by hand, checking each address against the allow-list.
    fn open(&self, url: &str, accept: &str) -> Result<ureq::Response, HttpError> {
        let mut current = url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            let host = host_of(&current).ok_or_else(|| HttpError::Host(current.clone()))?;
            if !allowed_host(host) {
                return Err(HttpError::Host(host.to_owned()));
            }
            let mut request = self
                .agent
                .get(&current)
                .set("Accept", accept)
                .set("X-GitHub-Api-Version", "2022-11-28");
            if host == API_HOST {
                if let Some(token) = &self.token {
                    request = request.set("Authorization", &format!("Bearer {token}"));
                }
            }
            match request.call() {
                Ok(response) if (300..400).contains(&response.status()) => {
                    let Some(next) = response.header("location") else {
                        return Err(HttpError::Status(response.status()));
                    };
                    current = next.to_owned();
                }
                Ok(response) => return Ok(response),
                Err(ureq::Error::Status(404, _)) => return Err(HttpError::NotFound),
                Err(ureq::Error::Status(code @ (401 | 403 | 429), _)) => {
                    return Err(HttpError::Refused(code))
                }
                Err(ureq::Error::Status(code, _)) => return Err(HttpError::Status(code)),
                // The transport error can carry the address: say only what failed.
                Err(ureq::Error::Transport(transport)) => {
                    return Err(HttpError::Network(format!("{:?}", transport.kind())))
                }
            }
        }
        Err(HttpError::Network("too many redirects".into()))
    }
}

impl Fetch for UreqFetch {
    fn text(&self, url: &str, accept: &str, limit: u64) -> Result<String, HttpError> {
        let response = self.open(url, accept)?;
        let mut body = String::new();
        response
            .into_reader()
            .take(limit + 1)
            .read_to_string(&mut body)
            .map_err(|e| HttpError::Network(e.kind().to_string()))?;
        if body.len() as u64 > limit {
            return Err(HttpError::TooLarge);
        }
        Ok(body)
    }

    fn download(
        &self,
        url: &str,
        destination: &Path,
        progress: &dyn Fn(u64, u64),
        cancel: &AtomicBool,
    ) -> Result<(), HttpError> {
        let response = self.open(url, "application/octet-stream")?;
        let total: u64 = response
            .header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if total > MAX_FILE {
            return Err(HttpError::TooLarge);
        }
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(destination)
            .map_err(|e| HttpError::Network(e.kind().to_string()))?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(HttpError::Cancelled);
            }
            let n = reader
                .read(&mut buffer)
                .map_err(|e| HttpError::Network(e.kind().to_string()))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > MAX_FILE {
                return Err(HttpError::TooLarge);
            }
            file.write_all(&buffer[..n])
                .map_err(|e| HttpError::Network(e.kind().to_string()))?;
            progress(done, total.max(done));
        }
        file.flush()
            .map_err(|e| HttpError::Network(e.kind().to_string()))
    }
}

pub const TEXT_LIMIT: u64 = MAX_TEXT;
pub const JSON_LIMIT: u64 = MAX_JSON;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_githubs_hosts_pass_and_only_over_https() {
        for host in [
            "api.github.com",
            "github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
        ] {
            assert!(allowed_host(host), "{host}");
        }
        for host in [
            "evil.com",
            "github.com.evil.com",
            "githubusercontent.com.evil.org",
            "notgithub.com",
            "",
        ] {
            assert!(!allowed_host(host), "{host}");
        }
        assert_eq!(
            host_of("https://api.github.com/repos/x"),
            Some("api.github.com")
        );
        assert_eq!(host_of("http://api.github.com/x"), None);
        assert_eq!(host_of("https://user@api.github.com/x"), None);
        assert_eq!(host_of("https://api.github.com:8443/x"), None);
        assert_eq!(host_of("https:///x"), None);
    }

    #[test]
    fn an_address_outside_github_is_refused_before_any_request() {
        let fetch = UreqFetch::new(Some("secret".into()));
        let error = fetch
            .text("https://evil.example/x", "application/json", 10)
            .unwrap_err();
        assert_eq!(error, HttpError::Host("evil.example".into()));
        assert!(!error.to_string().contains("secret"));
    }
}
