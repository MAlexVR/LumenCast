//! Starting and stopping the matte server container with `podman`.
//!
//! The image holds NVIDIA's proprietary libraries and models, so it is built
//! by the user from their own Blucast image (see `sidecar/nvidia-matte`); this
//! module only runs it.

use std::process::Command;

/// The image to run; overridable with `LUMENCAST_MATTE_IMAGE`.
pub const DEFAULT_IMAGE: &str = "localhost/lumencast-nvidia-matte:dev";
pub const CONTAINER: &str = "lumencast-matte";
pub const PORT: u16 = 18475;

/// What `podman` printed and whether it succeeded.
pub struct Outcome {
    pub success: bool,
    pub output: String,
}

/// Runs `podman` with some arguments; a fake one makes the logic testable.
pub trait Podman {
    fn run(&self, args: &[&str]) -> Result<Outcome, String>;
}

pub struct SystemPodman;

impl Podman for SystemPodman {
    fn run(&self, args: &[&str]) -> Result<Outcome, String> {
        let output = Command::new("podman")
            .args(args)
            .output()
            .map_err(|error| format!("podman could not be run: {error}"))?;
        let text = if output.status.success() {
            output.stdout
        } else {
            output.stderr
        };
        Ok(Outcome {
            success: output.status.success(),
            output: String::from_utf8_lossy(&text).trim().to_owned(),
        })
    }
}

pub fn image_name() -> String {
    std::env::var("LUMENCAST_MATTE_IMAGE")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| DEFAULT_IMAGE.to_owned())
}

/// Makes sure the server container is running and says why when it cannot be.
pub fn ensure_started(podman: &dyn Podman, image: &str) -> Result<(), String> {
    let exists = podman.run(&["image", "exists", image])?;
    if !exists.success {
        return Err(format!(
            "the NVIDIA matte image {image} is not built; build it with sidecar/nvidia-matte/build.sh"
        ));
    }
    let filter = format!("name=^{CONTAINER}$");
    let running = podman.run(&["ps", "-q", "--filter", &filter])?;
    if running.success && !running.output.is_empty() {
        return Ok(());
    }
    let publish = format!("127.0.0.1:{PORT}:{PORT}");
    let started = podman.run(&[
        "run",
        "-d",
        "--rm",
        "--name",
        CONTAINER,
        "--security-opt",
        "label=disable",
        "--device",
        "nvidia.com/gpu=all",
        "-p",
        &publish,
        image,
    ])?;
    if started.success {
        Ok(())
    } else {
        Err(format!(
            "the NVIDIA matte container did not start: {}",
            started.output
        ))
    }
}

/// Stops the container; nothing running is not an error.
pub fn stop(podman: &dyn Podman) {
    let _ = podman.run(&["stop", "-t", "2", CONTAINER]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Answers by the first words of the command and records every command.
    struct Fake {
        image_exists: bool,
        running: bool,
        run_ok: bool,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl Fake {
        fn new(image_exists: bool, running: bool, run_ok: bool) -> Self {
            Self {
                image_exists,
                running,
                run_ok,
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Podman for Fake {
        fn run(&self, args: &[&str]) -> Result<Outcome, String> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|a| (*a).to_owned()).collect());
            Ok(match args[0] {
                "image" => Outcome {
                    success: self.image_exists,
                    output: String::new(),
                },
                "ps" => Outcome {
                    success: true,
                    output: if self.running {
                        "abc123".into()
                    } else {
                        String::new()
                    },
                },
                "run" => Outcome {
                    success: self.run_ok,
                    output: if self.run_ok {
                        "id".into()
                    } else {
                        "no GPU".into()
                    },
                },
                _ => Outcome {
                    success: true,
                    output: String::new(),
                },
            })
        }
    }

    #[test]
    fn a_missing_image_says_how_to_build_it_and_starts_nothing() {
        let fake = Fake::new(false, false, true);
        let error = ensure_started(&fake, "img").unwrap_err();
        assert!(error.contains("sidecar/nvidia-matte/build.sh"), "{error}");
        assert!(fake.calls.borrow().iter().all(|c| c[0] != "run"));
    }

    #[test]
    fn a_running_container_is_left_alone() {
        let fake = Fake::new(true, true, true);
        ensure_started(&fake, "img").unwrap();
        assert!(fake.calls.borrow().iter().all(|c| c[0] != "run"));
    }

    #[test]
    fn a_stopped_one_is_started_with_the_gpu_and_only_on_loopback() {
        let fake = Fake::new(true, false, true);
        ensure_started(&fake, "img").unwrap();
        let calls = fake.calls.borrow();
        let run = calls.iter().find(|c| c[0] == "run").unwrap();
        let joined = run.join(" ");
        assert!(joined.contains("--device nvidia.com/gpu=all"));
        assert!(joined.contains("-p 127.0.0.1:18475:18475"));
        assert!(joined.contains("--rm") && joined.ends_with("img"));
    }

    #[test]
    fn a_failed_start_reports_what_podman_said() {
        let fake = Fake::new(true, false, false);
        let error = ensure_started(&fake, "img").unwrap_err();
        assert!(error.contains("no GPU"), "{error}");
    }

    #[test]
    fn a_missing_podman_is_reported() {
        struct Missing;
        impl Podman for Missing {
            fn run(&self, _: &[&str]) -> Result<Outcome, String> {
                Err("podman could not be run: not found".into())
            }
        }
        assert!(ensure_started(&Missing, "img")
            .unwrap_err()
            .contains("podman"));
        stop(&Missing);
    }
}
