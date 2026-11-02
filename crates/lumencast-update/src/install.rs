//! Installing a verified update with the tool that owns the installation:
//! the package manager for a package, a file replacement for an AppImage.
//! Everything that runs goes through [`System`], so each step is tested with a
//! fake; commands are argument lists, never a shell string.

use crate::release::InstallKind;
use std::path::{Path, PathBuf};

/// What the program needs from the machine (faked in tests).
pub trait System: Send + Sync {
    /// Runs a program and returns its exit code.
    fn run(&self, program: &str, arguments: &[String]) -> Result<i32, String>;
    fn exists(&self, program: &str) -> bool;
    fn env(&self, name: &str) -> Option<String>;
    fn current_exe(&self) -> Option<PathBuf>;
    /// Replaces `target` with `source`, keeping it executable.
    fn replace_file(&self, source: &Path, target: &Path) -> Result<(), String>;
    /// Starts the program again without waiting for it.
    fn spawn_detached(&self, program: &str) -> Result<(), String>;
}

/// pkexec exit codes: the dialog was dismissed (126) or authentication failed (127).
const POLKIT_DENIED: [i32; 2] = [126, 127];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("this installation cannot be updated from here: update it the way it was installed")]
    Unmanaged,
    #[error("the system asked for permission and it was not given")]
    NotAuthorized,
    #[error("the installer failed (exit code {0})")]
    Failed(i32),
    #[error("{0}")]
    Other(String),
}

/// How the running program is installed.
pub fn detect(system: &dyn System) -> InstallKind {
    if system
        .env("APPIMAGE")
        .is_some_and(|path| !path.is_empty() && Path::new(&path).is_file())
    {
        return InstallKind::AppImage;
    }
    let Some(exe) = system.current_exe() else {
        return InstallKind::Unmanaged;
    };
    let exe = exe.to_string_lossy().into_owned();
    if system.exists("rpm") && system.run("rpm", &["-qf".into(), exe.clone()]) == Ok(0) {
        return InstallKind::Rpm;
    }
    if system.exists("dpkg") && system.run("dpkg", &["-S".into(), exe]) == Ok(0) {
        return InstallKind::Deb;
    }
    InstallKind::Unmanaged
}

/// The program to start again after an update.
pub fn relaunch_target(system: &dyn System, kind: InstallKind) -> Option<String> {
    match kind {
        InstallKind::AppImage => system.env("APPIMAGE"),
        InstallKind::Rpm | InstallKind::Deb => Some("/usr/bin/lumencast".to_owned()),
        InstallKind::Unmanaged => None,
    }
}

/// The command that installs `file` for a package, as program and arguments.
pub fn package_command(
    system: &dyn System,
    kind: InstallKind,
    file: &Path,
) -> Option<(String, Vec<String>)> {
    let path = file.to_string_lossy().into_owned();
    let elevated = |program: &str, arguments: Vec<String>| {
        let mut all = vec![program.to_owned()];
        all.extend(arguments);
        ("pkexec".to_owned(), all)
    };
    match kind {
        InstallKind::Rpm if system.exists("dnf") => {
            Some(elevated("dnf", vec!["-y".into(), "install".into(), path]))
        }
        InstallKind::Rpm => Some(elevated(
            "rpm",
            vec!["-U".into(), "--replacepkgs".into(), path],
        )),
        InstallKind::Deb if system.exists("apt-get") => Some(elevated(
            "apt-get",
            vec!["install".into(), "-y".into(), path],
        )),
        InstallKind::Deb => Some(elevated("dpkg", vec!["-i".into(), path])),
        InstallKind::AppImage | InstallKind::Unmanaged => None,
    }
}

/// Installs `file` and starts the new version.
pub fn install(system: &dyn System, kind: InstallKind, file: &Path) -> Result<(), InstallError> {
    match kind {
        InstallKind::Unmanaged => return Err(InstallError::Unmanaged),
        InstallKind::AppImage => {
            let target = system
                .env("APPIMAGE")
                .ok_or_else(|| InstallError::Other("APPIMAGE is not set".into()))?;
            system
                .replace_file(file, Path::new(&target))
                .map_err(InstallError::Other)?;
        }
        InstallKind::Rpm | InstallKind::Deb => {
            if !system.exists("pkexec") {
                return Err(InstallError::Other(
                    "pkexec (polkit) is needed to install the update".into(),
                ));
            }
            let (program, arguments) = package_command(system, kind, file)
                .ok_or_else(|| InstallError::Other("no installer for this system".into()))?;
            match system
                .run(&program, &arguments)
                .map_err(InstallError::Other)?
            {
                0 => {}
                code if POLKIT_DENIED.contains(&code) => return Err(InstallError::NotAuthorized),
                code => return Err(InstallError::Failed(code)),
            }
        }
    }
    if let Some(target) = relaunch_target(system, kind) {
        system
            .spawn_detached(&target)
            .map_err(InstallError::Other)?;
    }
    Ok(())
}

/// The real machine.
pub struct HostSystem;

impl System for HostSystem {
    fn run(&self, program: &str, arguments: &[String]) -> Result<i32, String> {
        std::process::Command::new(program)
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.code().unwrap_or(-1))
            .map_err(|error| format!("{program} could not be run: {}", error.kind()))
    }

    fn exists(&self, program: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
        })
    }

    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn current_exe(&self) -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    fn replace_file(&self, source: &Path, target: &Path) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let staging = target.with_extension("update");
        std::fs::copy(source, &staging).map_err(|e| e.kind().to_string())?;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.kind().to_string())?;
        std::fs::rename(&staging, target).map_err(|e| e.kind().to_string())
    }

    fn spawn_detached(&self, program: &str) -> Result<(), String> {
        // After a pause, so the program that is updating has quit first: two
        // instances never run together, and the new one would hand over to the old.
        std::process::Command::new("sh")
            .args(["-c", "sleep 2; exec \"$0\"", program])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|e| format!("{program} could not be started: {}", e.kind()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    /// A machine that answers by script and records what ran.
    #[derive(Default)]
    struct Fake {
        programs: HashSet<&'static str>,
        env: HashMap<&'static str, String>,
        exe: Option<PathBuf>,
        codes: HashMap<&'static str, i32>,
        ran: Mutex<Vec<(String, Vec<String>)>>,
        replaced: Mutex<Vec<(PathBuf, PathBuf)>>,
        started: Mutex<Vec<String>>,
    }

    impl System for Fake {
        fn run(&self, program: &str, arguments: &[String]) -> Result<i32, String> {
            self.ran
                .lock()
                .unwrap()
                .push((program.to_owned(), arguments.to_vec()));
            let key = if program == "pkexec" {
                arguments[0].as_str()
            } else {
                program
            };
            Ok(self.codes.get(key).copied().unwrap_or(0))
        }
        fn exists(&self, program: &str) -> bool {
            self.programs.contains(program)
        }
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
        fn current_exe(&self) -> Option<PathBuf> {
            self.exe.clone()
        }
        fn replace_file(&self, source: &Path, target: &Path) -> Result<(), String> {
            self.replaced
                .lock()
                .unwrap()
                .push((source.into(), target.into()));
            Ok(())
        }
        fn spawn_detached(&self, program: &str) -> Result<(), String> {
            self.started.lock().unwrap().push(program.to_owned());
            Ok(())
        }
    }

    fn machine(programs: &[&'static str]) -> Fake {
        Fake {
            programs: programs.iter().copied().collect(),
            exe: Some("/usr/bin/lumencast".into()),
            ..Fake::default()
        }
    }

    #[test]
    fn the_package_that_owns_the_program_decides_how_it_updates() {
        assert_eq!(detect(&machine(&["rpm", "dpkg"])), InstallKind::Rpm);
        let mut deb = machine(&["rpm", "dpkg"]);
        deb.codes.insert("rpm", 1);
        assert_eq!(detect(&deb), InstallKind::Deb);
        let mut by_hand = machine(&["rpm", "dpkg"]);
        by_hand.codes.insert("rpm", 1);
        by_hand.codes.insert("dpkg", 1);
        assert_eq!(detect(&by_hand), InstallKind::Unmanaged);
        assert_eq!(detect(&machine(&[])), InstallKind::Unmanaged);
    }

    #[test]
    fn an_appimage_is_recognised_by_its_environment_and_replaced_in_place() {
        let dir = std::env::temp_dir().join("lumencast-update-test-appimage");
        std::fs::write(&dir, b"x").unwrap();
        let mut fake = machine(&[]);
        fake.env
            .insert("APPIMAGE", dir.to_string_lossy().into_owned());
        assert_eq!(detect(&fake), InstallKind::AppImage);
        install(&fake, InstallKind::AppImage, Path::new("/tmp/new.AppImage")).unwrap();
        assert_eq!(fake.replaced.lock().unwrap().len(), 1);
        assert_eq!(fake.started.lock().unwrap()[0], dir.to_string_lossy());
        std::fs::remove_file(&dir).unwrap();
    }

    #[test]
    fn a_package_is_installed_with_polkit_and_the_new_version_is_started() {
        let fake = machine(&["pkexec", "dnf"]);
        install(&fake, InstallKind::Rpm, Path::new("/tmp/x.rpm")).unwrap();
        let ran = fake.ran.lock().unwrap();
        assert_eq!(ran[0].0, "pkexec");
        assert_eq!(ran[0].1, ["dnf", "-y", "install", "/tmp/x.rpm"]);
        assert_eq!(fake.started.lock().unwrap()[0], "/usr/bin/lumencast");
        let without_dnf = machine(&["pkexec"]);
        install(&without_dnf, InstallKind::Rpm, Path::new("/tmp/x.rpm")).unwrap();
        assert_eq!(without_dnf.ran.lock().unwrap()[0].1[0], "rpm");
        let deb = machine(&["pkexec", "apt-get"]);
        install(&deb, InstallKind::Deb, Path::new("/tmp/x.deb")).unwrap();
        assert_eq!(
            deb.ran.lock().unwrap()[0].1,
            ["apt-get", "install", "-y", "/tmp/x.deb"]
        );
    }

    #[test]
    fn a_refused_permission_and_a_failing_installer_are_told_apart_and_nothing_restarts() {
        let mut denied = machine(&["pkexec", "dnf"]);
        denied.codes.insert("dnf", 126);
        assert_eq!(
            install(&denied, InstallKind::Rpm, Path::new("/tmp/x.rpm")),
            Err(InstallError::NotAuthorized)
        );
        let mut broken = machine(&["pkexec", "dnf"]);
        broken.codes.insert("dnf", 1);
        assert_eq!(
            install(&broken, InstallKind::Rpm, Path::new("/tmp/x.rpm")),
            Err(InstallError::Failed(1))
        );
        assert!(
            denied.started.lock().unwrap().is_empty() && broken.started.lock().unwrap().is_empty()
        );
        assert_eq!(
            install(&machine(&[]), InstallKind::Unmanaged, Path::new("/x")),
            Err(InstallError::Unmanaged)
        );
        assert!(matches!(
            install(
                &machine(&["dnf"]),
                InstallKind::Rpm,
                Path::new("/tmp/x.rpm")
            ),
            Err(InstallError::Other(_))
        ));
    }
}
