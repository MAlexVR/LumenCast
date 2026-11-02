//! LumenCast's own update: looks for the newest release on GitHub, downloads
//! the installer that matches how the program is installed, verifies it
//! (SHA-256 against `SHA256SUMS.txt`, and a minisign signature against the key
//! built into the program, failing closed) and installs it. The program
//! re-reads the release itself when installing: nothing the window sends is
//! trusted, and only GitHub's own hosts are contacted.

pub mod http;
pub mod install;
pub mod release;
pub mod verify;
pub mod version;

use http::{Fetch, HttpError, JSON_LIMIT, TEXT_LIMIT};
use install::{InstallError, System};
use release::{companion, installer_for, newest_after, parse_releases, InstallKind, Release};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use verify::{verify_checksum, verify_signature, VerifyError};
pub use version::Version;

pub const SUMS_ASSET: &str = "SHA256SUMS.txt";
/// The repository releases are published in.
pub const UPDATE_REPO: &str = "MAlexVR/LumenCast";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("{0}")]
    Http(#[from] HttpError),
    #[error("{0}")]
    Verify(#[from] VerifyError),
    #[error("{0}")]
    Install(#[from] InstallError),
    #[error("there is no newer version")]
    NothingNewer,
    #[error("the release has no installer for this system: {0}")]
    NoInstaller(String),
    #[error("the release is missing {0}")]
    MissingFile(String),
    #[error("{0}")]
    Other(String),
}

impl UpdateError {
    /// A short stable code for the window to word.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Http(HttpError::NotFound) => "update_not_found",
            Self::Http(HttpError::Refused(_)) => "update_refused",
            Self::Http(HttpError::Cancelled) => "update_cancelled",
            Self::Http(_) => "update_network",
            Self::Verify(_) => "update_unverified",
            Self::Install(InstallError::NotAuthorized) => "update_not_authorized",
            Self::Install(InstallError::Unmanaged) => "update_unmanaged",
            Self::Install(_) => "update_install_failed",
            Self::NothingNewer => "update_nothing_newer",
            Self::NoInstaller(_) | Self::MissingFile(_) => "update_no_installer",
            Self::Other(_) => "update_failed",
        }
    }
}

/// A newer version, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub published: String,
    /// The size of the installer, 0 when there is none for this system.
    pub size: u64,
    /// Whether the program can install it by itself on this system.
    pub can_install: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current: String,
    pub install: InstallKind,
    pub available: Option<Available>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Downloading,
    Verifying,
    Installing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

pub struct Updater {
    repo: String,
    current: Version,
    public_key: String,
    fetch: Box<dyn Fetch>,
    system: Box<dyn System>,
}

impl Updater {
    pub fn new(
        repo: &str,
        current: &str,
        public_key: &str,
        fetch: Box<dyn Fetch>,
        system: Box<dyn System>,
    ) -> Result<Self, UpdateError> {
        Ok(Self {
            repo: repo.to_owned(),
            current: Version::parse(current)
                .ok_or_else(|| UpdateError::Other(format!("{current:?} is not a version")))?,
            public_key: public_key.to_owned(),
            fetch,
            system,
        })
    }

    fn releases(&self, include_prereleases: bool) -> Result<Vec<Release>, UpdateError> {
        let url = format!(
            "https://api.github.com/repos/{}/releases?per_page=15",
            self.repo
        );
        let json = self
            .fetch
            .text(&url, "application/vnd.github+json", JSON_LIMIT)?;
        Ok(parse_releases(&json, include_prereleases))
    }

    /// Whether a newer version exists. Never downloads anything but the list.
    pub fn check(&self, include_prereleases: bool) -> Result<UpdateStatus, UpdateError> {
        let kind = install::detect(self.system.as_ref());
        let releases = self.releases(include_prereleases)?;
        let available = newest_after(&releases, &self.current).map(|release| {
            let installer = installer_for(release, kind);
            Available {
                version: release.version.to_string(),
                notes: release.notes.clone(),
                page: release.page.clone(),
                published: release.published.clone(),
                size: installer.map_or(0, |asset| asset.size),
                can_install: installer.is_some()
                    && companion(release, SUMS_ASSET).is_some()
                    && installer
                        .and_then(|a| companion(release, &format!("{}.sig", a.name)))
                        .is_some(),
            }
        });
        Ok(UpdateStatus {
            current: self.current.to_string(),
            install: kind,
            available,
        })
    }

    /// Downloads, verifies and installs the newest version, then starts it.
    pub fn install(
        &self,
        include_prereleases: bool,
        progress: &dyn Fn(Progress),
        cancel: &AtomicBool,
    ) -> Result<(), UpdateError> {
        let kind = install::detect(self.system.as_ref());
        if kind == InstallKind::Unmanaged {
            return Err(InstallError::Unmanaged.into());
        }
        let releases = self.releases(include_prereleases)?;
        let release = newest_after(&releases, &self.current).ok_or(UpdateError::NothingNewer)?;
        let installer = installer_for(release, kind)
            .ok_or_else(|| UpdateError::NoInstaller(release.version.to_string()))?;
        let sums = companion(release, SUMS_ASSET)
            .ok_or_else(|| UpdateError::MissingFile(SUMS_ASSET.to_owned()))?;
        let signature_name = format!("{}.sig", installer.name);
        let signature = companion(release, &signature_name)
            .ok_or_else(|| UpdateError::MissingFile(signature_name.clone()))?;

        let sums_text = self
            .fetch
            .text(&sums.url, "application/octet-stream", TEXT_LIMIT)?;
        let signature_text =
            self.fetch
                .text(&signature.url, "application/octet-stream", TEXT_LIMIT)?;

        let folder = scratch_folder();
        std::fs::create_dir_all(&folder).map_err(|e| UpdateError::Other(e.kind().to_string()))?;
        let result = (|| {
            let file = folder.join(&installer.name);
            self.fetch.download(
                &installer.url,
                &file,
                &|done, total| {
                    progress(Progress {
                        phase: Phase::Downloading,
                        done,
                        total,
                    })
                },
                cancel,
            )?;
            if cancel.load(Ordering::SeqCst) {
                return Err(HttpError::Cancelled.into());
            }
            progress(Progress {
                phase: Phase::Verifying,
                done: 0,
                total: 0,
            });
            verify_checksum(&file, &installer.name, &sums_text)?;
            verify_signature(&self.public_key, &signature_text, &file)?;
            progress(Progress {
                phase: Phase::Installing,
                done: 0,
                total: 0,
            });
            install::install(self.system.as_ref(), kind, &file)?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&folder);
        result
    }
}

/// A folder of its own for each installation, so two never share files.
fn scratch_folder() -> PathBuf {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "lumencast-update-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::SeqCst)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Mutex;

    const PUB: &str = include_str!("../tests/fixtures/test.pub");
    const SIG: &str = include_str!("../tests/fixtures/payload.bin.sig");
    const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/payload.bin");

    /// GitHub, from a table of address to body.
    struct Fake {
        files: HashMap<String, Vec<u8>>,
    }

    impl Fetch for Fake {
        fn text(&self, url: &str, _accept: &str, _limit: u64) -> Result<String, HttpError> {
            self.files
                .get(url)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .ok_or(HttpError::NotFound)
        }
        fn download(
            &self,
            url: &str,
            destination: &Path,
            progress: &dyn Fn(u64, u64),
            _cancel: &AtomicBool,
        ) -> Result<(), HttpError> {
            let body = self.files.get(url).ok_or(HttpError::NotFound)?;
            std::fs::write(destination, body).map_err(|e| HttpError::Network(e.to_string()))?;
            progress(body.len() as u64, body.len() as u64);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Machine {
        ran: Mutex<Vec<String>>,
        started: Mutex<Vec<String>>,
        rpm: bool,
    }

    impl System for Machine {
        fn run(&self, program: &str, arguments: &[String]) -> Result<i32, String> {
            self.ran
                .lock()
                .unwrap()
                .push(format!("{program} {}", arguments.join(" ")));
            Ok(0)
        }
        fn exists(&self, program: &str) -> bool {
            matches!(program, "pkexec" | "dnf") || (self.rpm && program == "rpm")
        }
        fn env(&self, _: &str) -> Option<String> {
            None
        }
        fn current_exe(&self) -> Option<std::path::PathBuf> {
            Some("/usr/bin/lumencast".into())
        }
        fn replace_file(&self, _: &Path, _: &Path) -> Result<(), String> {
            Ok(())
        }
        fn spawn_detached(&self, program: &str) -> Result<(), String> {
            self.started.lock().unwrap().push(program.into());
            Ok(())
        }
    }

    fn release_json(with_sig: bool) -> String {
        let signature = if with_sig {
            r#",{"name": "LumenCast-1.1.0-1.x86_64.rpm.sig", "url": "https://api.github.com/a/sig", "size": 1}"#
        } else {
            ""
        };
        format!(
            r#"[{{"tag_name": "v1.1.0", "draft": false, "prerelease": false, "body": "News",
            "assets": [
              {{"name": "LumenCast-1.1.0-1.x86_64.rpm", "url": "https://api.github.com/a/rpm", "size": {}}},
              {{"name": "SHA256SUMS.txt", "url": "https://api.github.com/a/sums", "size": 1}}{signature}]}}]"#,
            PAYLOAD.len()
        )
    }

    fn github(installer: &[u8], with_sig: bool) -> Fake {
        let hash = {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("x");
            std::fs::write(&path, installer).unwrap();
            verify::sha256_file(&path).unwrap()
        };
        let mut files = HashMap::new();
        files.insert(
            "https://api.github.com/repos/o/r/releases?per_page=15".into(),
            release_json(with_sig).into_bytes(),
        );
        files.insert("https://api.github.com/a/rpm".into(), installer.to_vec());
        files.insert(
            "https://api.github.com/a/sums".into(),
            format!("{hash}  LumenCast-1.1.0-1.x86_64.rpm\n").into_bytes(),
        );
        files.insert(
            "https://api.github.com/a/sig".into(),
            SIG.trim().as_bytes().to_vec(),
        );
        Fake { files }
    }

    fn updater(fake: Fake, machine: Machine, current: &str) -> Updater {
        Updater::new(
            "o/r",
            current,
            PUB.trim(),
            Box::new(fake),
            Box::new(machine),
        )
        .unwrap()
    }

    fn rpm() -> Machine {
        Machine {
            rpm: true,
            ..Machine::default()
        }
    }

    #[test]
    fn a_newer_release_is_reported_with_what_the_user_needs_to_decide() {
        let status = updater(github(PAYLOAD, true), rpm(), "1.0.0")
            .check(false)
            .unwrap();
        assert_eq!(status.current, "1.0.0");
        let available = status.available.unwrap();
        assert_eq!(available.version, "1.1.0");
        assert_eq!(available.notes, "News");
        assert_eq!(available.size, PAYLOAD.len() as u64);
        assert!(available.can_install);
        let same = updater(github(PAYLOAD, true), rpm(), "1.1.0")
            .check(false)
            .unwrap();
        assert!(same.available.is_none());
    }

    #[test]
    fn a_release_without_a_signature_is_shown_but_cannot_be_installed_by_the_program() {
        let status = updater(github(PAYLOAD, false), rpm(), "1.0.0")
            .check(false)
            .unwrap();
        assert!(!status.available.unwrap().can_install);
    }

    #[test]
    fn a_genuine_installer_is_verified_installed_and_the_program_restarts() {
        let steps = Mutex::new(Vec::new());
        let machine = Machine {
            rpm: true,
            ..Machine::default()
        };
        let updater = updater(github(PAYLOAD, true), machine, "1.0.0");
        updater
            .install(
                false,
                &|p| steps.lock().unwrap().push(p.phase),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(
            *steps.lock().unwrap(),
            [Phase::Downloading, Phase::Verifying, Phase::Installing]
        );
    }

    #[test]
    fn a_tampered_installer_is_refused_before_anything_runs() {
        let mut tampered = PAYLOAD.to_vec();
        tampered.push(b'!');
        // The sums file is the attacker's too: only the signature can catch it.
        let fake = github(&tampered, true);
        let machine = rpm();
        let updater = updater(fake, machine, "1.0.0");
        let error = updater
            .install(false, &|_| {}, &AtomicBool::new(false))
            .unwrap_err();
        assert!(
            matches!(error, UpdateError::Verify(VerifyError::SignatureInvalid(_))),
            "{error:?}"
        );
        assert_eq!(error.code(), "update_unverified");
    }

    #[test]
    fn a_sums_file_that_disagrees_is_refused_too() {
        let mut fake = github(PAYLOAD, true);
        fake.files.insert(
            "https://api.github.com/a/sums".into(),
            format!("{}  LumenCast-1.1.0-1.x86_64.rpm\n", "0".repeat(64)).into_bytes(),
        );
        let error = updater(fake, rpm(), "1.0.0")
            .install(false, &|_| {}, &AtomicBool::new(false))
            .unwrap_err();
        assert!(matches!(
            error,
            UpdateError::Verify(VerifyError::ChecksumMismatch(_))
        ));
    }

    #[test]
    fn without_a_signature_nothing_is_installed_and_with_nothing_newer_there_is_nothing_to_do() {
        let error = updater(github(PAYLOAD, false), rpm(), "1.0.0")
            .install(false, &|_| {}, &AtomicBool::new(false))
            .unwrap_err();
        assert!(matches!(error, UpdateError::MissingFile(_)));
        let error = updater(github(PAYLOAD, true), rpm(), "1.1.0")
            .install(false, &|_| {}, &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(error, UpdateError::NothingNewer);
    }

    #[test]
    fn a_private_repository_without_a_token_says_so() {
        let fake = Fake {
            files: HashMap::new(),
        };
        let error = updater(fake, rpm(), "1.0.0").check(false).unwrap_err();
        assert_eq!(error.code(), "update_not_found");
        assert!(Updater::new(
            "o/r",
            "nope",
            PUB,
            Box::new(Fake {
                files: HashMap::new()
            }),
            Box::new(Machine::default())
        )
        .is_err());
    }
}
