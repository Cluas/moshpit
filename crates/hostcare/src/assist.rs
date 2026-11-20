//! Install Assist (`InstallAssistPlan`): what a session missing tmux, mosh
//! or herdr is offered — the packages, the command for the host's package
//! manager — and when a re-check of the host has found them. It installs
//! nothing itself: the person runs the command in their shell.

use moshpit_install::{HostCapabilities, Multiplexer, PackageManager};

/// What a re-check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recheck {
    /// Everything the packages cover is there.
    Resolved,
    StillMissing,
    /// The host couldn't be asked.
    NoChannel,
}

/// The plan for one notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistPlan {
    /// herdr ships in no distro repository: it has its own installer.
    pub herdr: bool,
    /// What to install, from the connection rather than the notice: a
    /// herdr + mosh connection missing mosh must not be handed a
    /// multiplexer it didn't pick.
    pub packages: Vec<&'static str>,
    /// The host's package manager, as its last probe found it.
    pub manager: Option<PackageManager>,
}

impl AssistPlan {
    /// `notice` is the notice's packages (`["tmux"]`, `["mosh"]` or
    /// `["herdr"]`); `tmux` whether the connection's multiplexer is tmux.
    pub fn new(notice: &[String], tmux: bool, manager: Option<PackageManager>) -> Self {
        let herdr = notice.iter().any(|p| p == "herdr");
        let packages = if herdr {
            vec!["herdr"]
        } else if tmux {
            vec!["tmux", "mosh"]
        } else {
            vec!["mosh"]
        };
        AssistPlan {
            herdr,
            packages,
            manager,
        }
    }

    pub fn mosh_only(&self) -> bool {
        self.packages == ["mosh"]
    }

    /// The command for the host's package manager, if it has a known one
    /// (herdr's own installer otherwise).
    pub fn command(&self) -> Option<String> {
        if self.herdr {
            Multiplexer::Herdr.install_command(self.manager)
        } else {
            self.manager.map(|m| m.install_command(&self.packages))
        }
    }

    /// herdr's installer drops the binary in `~/.local/bin` and touches no
    /// rc file: say Moshpit looks there.
    pub fn shows_note(&self) -> bool {
        self.herdr && self.manager != Some(PackageManager::Brew)
    }

    /// A shell command that prints `busy` while the host runs an install
    /// of this plan's packages (the command Install Assist gives, or one
    /// the person typed for the same packages: `apt-get install -y tmux
    /// mosh`, `brew install tmux mosh`, …), `idle` otherwise. Reconnecting
    /// mid-install would hang the installer up with the old shell (a dpkg
    /// left half configured), so the app waits for `idle`. None for herdr,
    /// whose installer is `curl … | sh`: nothing in its arguments says so.
    ///
    /// The pattern ends at `$`: the probe's own `grep` (its pattern ends in
    /// a literal `$`) and shell (it ends in `idle`) never match it.
    pub fn installer_probe(&self) -> Option<String> {
        if self.herdr {
            return None;
        }
        let packages = self.packages.join(" ");
        Some(format!(
            "{{ ps -A -o args= 2>/dev/null || ps w 2>/dev/null; }} \
             | grep -E '(install|add|-S)( [^ ]+)* {packages}$' >/dev/null \
             && echo busy || echo idle"
        ))
    }

    /// What [`Self::installer_probe`]'s output says: an installer runs.
    pub fn installer_running(output: &str) -> bool {
        output.trim() == "busy"
    }

    /// What a re-check's probe means for this plan.
    pub fn recheck(&self, found: Option<&HostCapabilities>) -> Recheck {
        let Some(caps) = found else {
            return Recheck::NoChannel;
        };
        let resolved = if self.herdr {
            caps.has_herdr
        } else {
            (!self.packages.contains(&"tmux") || caps.has_tmux)
                && (!self.packages.contains(&"mosh") || caps.has_mosh_server)
        };
        if resolved {
            Recheck::Resolved
        } else {
            Recheck::StillMissing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(tmux: bool, mosh: bool, herdr: bool) -> HostCapabilities {
        let mut caps = HostCapabilities::parse("");
        caps.has_tmux = tmux;
        caps.has_mosh_server = mosh;
        caps.has_herdr = herdr;
        caps
    }

    #[test]
    fn the_packages_follow_the_connection_not_the_notice() {
        let notice = vec!["mosh".to_owned()];
        assert_eq!(
            AssistPlan::new(&notice, true, None).packages,
            ["tmux", "mosh"]
        );
        let plain = AssistPlan::new(&notice, false, None);
        assert_eq!(plain.packages, ["mosh"]);
        assert!(plain.mosh_only());
        let herdr = AssistPlan::new(&["herdr".to_owned()], true, None);
        assert_eq!(herdr.packages, ["herdr"]);
        assert!(herdr.herdr);
    }

    #[test]
    fn the_command_is_the_package_managers() {
        let plan = AssistPlan::new(&["tmux".to_owned()], true, Some(PackageManager::AptGet));
        assert_eq!(
            plan.command().as_deref(),
            Some("sudo apt-get install -y tmux mosh")
        );
        assert_eq!(
            AssistPlan::new(&["tmux".to_owned()], true, None).command(),
            None
        );
        let herdr = AssistPlan::new(&["herdr".to_owned()], false, None);
        assert!(herdr.command().is_some(), "herdr has its own installer");
        assert!(herdr.shows_note());
        let brew = AssistPlan::new(&["herdr".to_owned()], false, Some(PackageManager::Brew));
        assert!(!brew.shows_note());
    }

    /// Runs `plan`'s probe under `sh` with a `ps` that lists `procs`.
    fn probe(plan: &AssistPlan, procs: &[&str]) -> String {
        let dir = std::env::temp_dir().join(format!(
            "assist-probe-{}-{}",
            std::process::id(),
            procs.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let listing = dir.join("procs");
        std::fs::write(&listing, procs.join("\n") + "\n").unwrap();
        let ps = dir.join("ps");
        // The real ps would list the probe itself too: so does this one.
        std::fs::write(
            &ps,
            format!(
                "#!/bin/sh\ncat '{}'\necho \"sh -c $PROBE\"\n",
                listing.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = plan.installer_probe().unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .env(
                "PATH",
                format!("{}:{}", dir.display(), std::env::var("PATH").unwrap()),
            )
            .env("PROBE", &command)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&dir).ok();
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn the_installer_probe_sees_an_install_of_the_plans_packages() {
        let both = AssistPlan::new(&["tmux".to_owned()], true, Some(PackageManager::AptGet));
        let busy = [
            vec!["sudo apt-get install -y tmux mosh"],
            vec![
                "/usr/bin/dpkg --status-fd 23 --configure",
                "apt-get install -y tmux mosh",
            ],
            vec![
                "/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby -W1 /opt/homebrew/Library/Homebrew/brew.rb install tmux mosh",
            ],
            vec!["pacman -S --noconfirm tmux mosh"],
            vec!["sudo apk add tmux mosh"],
        ];
        for procs in busy {
            let out = probe(&both, &procs);
            assert_eq!(out.trim(), "busy", "{procs:?}");
            assert!(AssistPlan::installer_running(&out));
        }
        for procs in [
            vec!["-zsh", "tmux new -s work", "mosh-server new"],
            vec!["vim install tmux mosh.txt"],
            vec!["sudo apt-get install -y htop"],
        ] {
            let out = probe(&both, &procs);
            assert_eq!(out.trim(), "idle", "{procs:?}");
            assert!(!AssistPlan::installer_running(&out));
        }
        let mosh = AssistPlan::new(&["mosh".to_owned()], false, Some(PackageManager::Dnf));
        assert_eq!(probe(&mosh, &["sudo dnf install -y mosh"]).trim(), "busy");
        let herdr = AssistPlan::new(&["herdr".to_owned()], false, None);
        assert_eq!(herdr.installer_probe(), None);
    }

    #[test]
    fn a_recheck_resolves_only_when_everything_is_there() {
        let plan = AssistPlan::new(&["tmux".to_owned()], true, None);
        assert_eq!(plan.recheck(None), Recheck::NoChannel);
        assert_eq!(
            plan.recheck(Some(&caps(true, false, false))),
            Recheck::StillMissing
        );
        assert_eq!(
            plan.recheck(Some(&caps(true, true, false))),
            Recheck::Resolved
        );
        let mosh = AssistPlan::new(&["mosh".to_owned()], false, None);
        assert_eq!(
            mosh.recheck(Some(&caps(false, true, false))),
            Recheck::Resolved
        );
        let herdr = AssistPlan::new(&["herdr".to_owned()], false, None);
        assert_eq!(
            herdr.recheck(Some(&caps(true, true, false))),
            Recheck::StillMissing
        );
        assert_eq!(
            herdr.recheck(Some(&caps(false, false, true))),
            Recheck::Resolved
        );
    }
}
