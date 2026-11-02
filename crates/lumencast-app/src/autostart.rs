//! Starting LumenCast when the user logs in, through the freedesktop autostart
//! convention: a `.desktop` file in `$XDG_CONFIG_HOME/autostart`.

use crate::error::AppError;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "lumencast.desktop";
/// The argument the started program receives, so it can come up minimized.
pub const AUTOSTART_ARGUMENT: &str = "--autostart";

pub struct Autostart {
    directory: PathBuf,
}

impl Autostart {
    /// `$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart`. `None` without
    /// either, so nothing is written at a guessed place.
    pub fn standard(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<Self> {
        let base = match (
            xdg_config_home.filter(|v| !v.is_empty()),
            home.filter(|v| !v.is_empty()),
        ) {
            (Some(xdg), _) => PathBuf::from(xdg),
            (None, Some(home)) => Path::new(home).join(".config"),
            (None, None) => return None,
        };
        Some(Self::in_directory(base.join("autostart")))
    }

    pub fn in_directory(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn file(&self) -> PathBuf {
        self.directory.join(FILE_NAME)
    }

    pub fn is_enabled(&self) -> bool {
        self.file().is_file()
    }

    pub fn enable(&self, executable: &Path) -> Result<(), AppError> {
        if !executable.is_absolute() {
            return Err(AppError::InvalidSetting(
                "the program path for autostart must be absolute".into(),
            ));
        }
        std::fs::create_dir_all(&self.directory)
            .and_then(|()| std::fs::write(self.file(), entry(executable)))
            .map_err(|error| AppError::AutostartFailed(error.to_string()))
    }

    /// Removing what is not there is not an error.
    pub fn disable(&self) -> Result<(), AppError> {
        match std::fs::remove_file(self.file()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::AutostartFailed(error.to_string())),
        }
    }
}

/// A quoted `Exec` argument as the desktop entry specification asks: inside
/// double quotes, `"`, `` ` ``, `$` and `\` are escaped with a backslash.
fn quote(argument: &str) -> String {
    let mut out = String::from("\"");
    for ch in argument.chars() {
        if matches!(ch, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

fn entry(executable: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=LumenCast\nComment=Camera preview and effects\nExec={} {AUTOSTART_ARGUMENT}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        quote(&executable.display().to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_starts_the_program_with_the_autostart_argument() {
        let text = entry(Path::new("/opt/Lumen Cast/lumencast"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("Exec=\"/opt/Lumen Cast/lumencast\" --autostart\n"));
        assert!(text.contains("Type=Application"));
    }

    #[test]
    fn special_characters_in_the_path_are_escaped_inside_the_quotes() {
        assert_eq!(quote("/a\"b$c`d\\e"), "\"/a\\\"b\\$c\\`d\\\\e\"");
    }

    #[test]
    fn enabling_writes_the_file_and_disabling_removes_it_twice_safely() {
        let dir = tempfile::tempdir().unwrap();
        let autostart = Autostart::in_directory(dir.path().join("autostart"));
        assert!(!autostart.is_enabled());
        autostart.enable(Path::new("/usr/bin/lumencast")).unwrap();
        assert!(autostart.is_enabled());
        autostart.enable(Path::new("/usr/bin/lumencast")).unwrap();
        autostart.disable().unwrap();
        assert!(!autostart.is_enabled());
        autostart.disable().unwrap();
    }

    #[test]
    fn a_relative_program_path_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let autostart = Autostart::in_directory(dir.path().join("autostart"));
        assert!(matches!(
            autostart.enable(Path::new("lumencast")),
            Err(AppError::InvalidSetting(_))
        ));
        assert!(!dir.path().join("autostart").exists());
    }

    #[test]
    fn the_directory_follows_xdg_then_home_and_is_none_without_either() {
        let xdg = Autostart::standard(Some("/x"), Some("/h")).unwrap();
        assert_eq!(xdg.directory, Path::new("/x/autostart"));
        let home = Autostart::standard(None, Some("/h")).unwrap();
        assert_eq!(home.directory, Path::new("/h/.config/autostart"));
        assert!(Autostart::standard(Some(""), None).is_none());
    }

    #[test]
    fn a_failure_to_write_is_reported_with_its_own_code() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("autostart");
        std::fs::write(&blocker, "a file where the directory should be").unwrap();
        let error = Autostart::in_directory(blocker)
            .enable(Path::new("/usr/bin/lumencast"))
            .unwrap_err();
        assert_eq!(error.code(), "autostart_failed");
    }
}
