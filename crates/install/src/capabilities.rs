//! What a host can run — tmux, mosh-server, herdr, python3, and which
//! package manager could install what is missing (`HostCapabilities`). Drives
//! the session's degrade matrix and the install-assist sheet's command.

use moshpit_seal::foundation::text;
use serde::{Deserialize, Serialize};

/// Extra `PATH` entries a non-interactive exec channel misses: Homebrew on
/// both architectures, `/usr/local`, and herdr's installer's `~/.local/bin`.
/// `$HOME` expands on the host.
pub const EXTRA_PATH_DIRS: &str =
    "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin";

/// The probe whose output [`HostCapabilities::parse`] reads.
/// Each tool is probed individually because POSIX `command -v` in dash
/// (Debian/Ubuntu `/bin/sh`) only accepts a single operand and ignores the rest.
pub fn probe_command() -> String {
    format!(
        "for c in tmux mosh-server herdr python3; do PATH=\"$PATH:{EXTRA_PATH_DIRS}\" command -v \"$c\" 2>/dev/null || true; done; \
         echo \"::$(uname -s)::$(for p in apt-get dnf yum pacman apk brew; do PATH=\"$PATH:{EXTRA_PATH_DIRS}\" command -v \"$p\" 2>/dev/null && break; done || true)\""
    )
}

/// The multiplexer a connection drives on the host (`Multiplexer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Multiplexer {
    None,
    Tmux,
    Herdr,
}

impl Multiplexer {
    /// The install command install-assist offers, given the host's package
    /// manager. herdr is in no distro repository: only Homebrew has it, and
    /// its installer script needs no package manager at all.
    pub fn install_command(self, manager: Option<PackageManager>) -> Option<String> {
        match self {
            Self::None => None,
            Self::Tmux => manager.map(|m| m.install_command(&["tmux", "mosh"])),
            Self::Herdr if manager == Some(PackageManager::Brew) => {
                Some("brew install herdr".to_owned())
            }
            Self::Herdr => Some("curl -fsSL https://herdr.dev/install.sh | sh".to_owned()),
        }
    }
}

/// The host's package manager (`PackageManager`); serialized as Swift's
/// raw values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PackageManager {
    #[serde(rename = "aptGet")]
    AptGet,
    #[serde(rename = "dnf")]
    Dnf,
    #[serde(rename = "yum")]
    Yum,
    #[serde(rename = "pacman")]
    Pacman,
    #[serde(rename = "apk")]
    Apk,
    #[serde(rename = "brew")]
    Brew,
}

impl PackageManager {
    pub const ALL: [Self; 6] = [
        Self::AptGet,
        Self::Dnf,
        Self::Yum,
        Self::Pacman,
        Self::Apk,
        Self::Brew,
    ];

    /// From a `command -v` hit's last path component.
    pub fn from_binary_path(path: &str) -> Option<Self> {
        let name = last_component(path).unwrap_or(path);
        match name {
            "apt-get" => Some(Self::AptGet),
            "dnf" => Some(Self::Dnf),
            "yum" => Some(Self::Yum),
            "pacman" => Some(Self::Pacman),
            "apk" => Some(Self::Apk),
            "brew" => Some(Self::Brew),
            _ => None,
        }
    }

    /// `sudo apt-get install -y tmux mosh`; brew runs unprivileged.
    pub fn install_command(self, packages: &[&str]) -> String {
        let packages = packages.join(" ");
        match self {
            Self::AptGet => format!("sudo apt-get install -y {packages}"),
            Self::Dnf => format!("sudo dnf install -y {packages}"),
            Self::Yum => format!("sudo yum install -y {packages}"),
            Self::Pacman => format!("sudo pacman -S --noconfirm {packages}"),
            Self::Apk => format!("sudo apk add {packages}"),
            Self::Brew => format!("brew install {packages}"),
        }
    }
}

/// `text.split(separator: "/").last`, by Swift `Character`, empty runs
/// omitted.
fn last_component(text: &str) -> Option<&str> {
    text::split_characters(text, |c| c == "/").pop()
}

/// What a host can run (`HostCapabilities`); `Codable` as the app caches it
/// in `UserDefaults`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilities {
    #[serde(rename = "hasTmux")]
    pub has_tmux: bool,
    #[serde(rename = "hasMoshServer")]
    pub has_mosh_server: bool,
    /// `uname -s`, empty when unknown.
    pub os: String,
    #[serde(
        rename = "packageManager",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub package_manager: Option<PackageManager>,
    #[serde(rename = "hasHerdr")]
    pub has_herdr: bool,
    #[serde(rename = "hasPython3")]
    pub has_python3: bool,
}

impl HostCapabilities {
    /// Everything present: the optimistic default before the first probe.
    pub fn unknown() -> Self {
        Self {
            has_tmux: true,
            has_mosh_server: true,
            os: String::new(),
            package_manager: None,
            has_herdr: true,
            has_python3: true,
        }
    }

    pub fn has(&self, multiplexer: Multiplexer) -> bool {
        match multiplexer {
            Multiplexer::None => true,
            Multiplexer::Tmux => self.has_tmux,
            Multiplexer::Herdr => self.has_herdr,
        }
    }

    /// Read the probe's output (`HostCapabilities.parse`), by Swift
    /// `Character` as the app does: lines split at every newline character
    /// (CR LF included), trimmed of `CharacterSet.whitespaces`; a line
    /// starting `::` is the `::<uname>::<package manager path>` marker,
    /// every other one a tool path whose last component names it.
    pub fn parse(output: &str) -> Self {
        let mut caps = Self {
            has_tmux: false,
            has_mosh_server: false,
            os: String::new(),
            package_manager: None,
            has_herdr: false,
            has_python3: false,
        };
        for raw in text::split_characters(output, text::character_is_newline) {
            let line = text::trim_inline_spaces(raw);
            let mut characters = text::characters(line);
            if characters.next() == Some(":") && characters.next() == Some(":") {
                let fields = split_characters_keeping_empty(line, ":");
                if fields.len() > 2 {
                    caps.os = text::trim_inline_spaces(fields[2]).to_owned();
                }
                if fields.len() > 4 {
                    caps.package_manager =
                        PackageManager::from_binary_path(text::trim_inline_spaces(fields[4]));
                }
                continue;
            }
            match last_component(line).unwrap_or(line) {
                "tmux" => caps.has_tmux = true,
                "mosh-server" => caps.has_mosh_server = true,
                "herdr" => caps.has_herdr = true,
                "python3" => caps.has_python3 = true,
                _ => {}
            }
        }
        caps
    }
}

/// `text.split(separator: sep, omittingEmptySubsequences: false)` by
/// `Character`.
fn split_characters_keeping_empty<'a>(text: &'a str, separator: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut at = 0;
    for character in text::characters(text) {
        if character == separator {
            out.push(&text[start..at]);
            start = at + character.len();
        }
        at += character.len();
    }
    out.push(&text[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - HostCapabilitiesTests

    #[test]
    fn linux_host_with_tmux_mosh_server_and_apt_get_parses_fully() {
        let caps = HostCapabilities::parse(
            "/usr/bin/tmux\n/usr/bin/mosh-server\n::Linux::/usr/bin/apt-get",
        );
        assert!(caps.has_tmux);
        assert!(caps.has_mosh_server);
        assert_eq!(caps.os, "Linux");
        assert_eq!(caps.package_manager, Some(PackageManager::AptGet));
    }

    #[test]
    fn host_missing_tmux() {
        let caps = HostCapabilities::parse("/usr/bin/mosh-server\n::Linux::/usr/bin/dnf");
        assert!(!caps.has_tmux);
        assert!(caps.has_mosh_server);
        assert_eq!(caps.package_manager, Some(PackageManager::Dnf));
    }

    #[test]
    fn host_missing_mosh_server() {
        let caps = HostCapabilities::parse("/usr/bin/tmux\n::Linux::/usr/bin/pacman");
        assert!(caps.has_tmux);
        assert!(!caps.has_mosh_server);
        assert_eq!(caps.package_manager, Some(PackageManager::Pacman));
    }

    #[test]
    fn bare_host_no_package_manager() {
        let caps = HostCapabilities::parse("::Linux::\n");
        assert!(!caps.has_tmux);
        assert!(!caps.has_mosh_server);
        assert_eq!(caps.os, "Linux");
        assert_eq!(caps.package_manager, None);
    }

    #[test]
    fn macos_host_with_brew() {
        let caps = HostCapabilities::parse(
            "/opt/homebrew/bin/tmux\n/opt/homebrew/bin/mosh-server\n::Darwin::/opt/homebrew/bin/brew",
        );
        assert_eq!(caps.os, "Darwin");
        assert_eq!(caps.package_manager, Some(PackageManager::Brew));
        assert!(caps.has_tmux && caps.has_mosh_server);
    }

    #[test]
    fn apk_and_yum_map_and_unknown_is_none() {
        assert_eq!(
            HostCapabilities::parse("::Linux::/sbin/apk\n").package_manager,
            Some(PackageManager::Apk)
        );
        assert_eq!(
            HostCapabilities::parse("::Linux::/usr/bin/yum\n").package_manager,
            Some(PackageManager::Yum)
        );
        assert_eq!(
            HostCapabilities::parse("::Linux::/usr/bin/zypper\n").package_manager,
            None
        );
    }

    #[test]
    fn install_commands_include_sudo_except_brew() {
        let packages = ["tmux", "mosh"];
        assert_eq!(
            PackageManager::AptGet.install_command(&packages),
            "sudo apt-get install -y tmux mosh"
        );
        assert_eq!(
            PackageManager::Dnf.install_command(&packages),
            "sudo dnf install -y tmux mosh"
        );
        assert_eq!(
            PackageManager::Yum.install_command(&packages),
            "sudo yum install -y tmux mosh"
        );
        assert_eq!(
            PackageManager::Pacman.install_command(&packages),
            "sudo pacman -S --noconfirm tmux mosh"
        );
        assert_eq!(
            PackageManager::Apk.install_command(&packages),
            "sudo apk add tmux mosh"
        );
        assert_eq!(
            PackageManager::Brew.install_command(&packages),
            "brew install tmux mosh"
        );
    }

    #[test]
    fn noisy_or_empty_output_degrades_to_all_missing() {
        let caps = HostCapabilities::parse("");
        assert!(!caps.has_tmux && !caps.has_mosh_server && caps.package_manager.is_none());
    }

    #[test]
    fn cr_lf_line_endings_parse_identically_to_lf() {
        let caps = HostCapabilities::parse(
            "/usr/bin/tmux\r\n/usr/bin/mosh-server\r\n::Linux::/usr/bin/apt-get\r\n",
        );
        assert!(caps.has_tmux && caps.has_mosh_server);
        assert_eq!(caps.package_manager, Some(PackageManager::AptGet));
    }

    #[test]
    fn unknown_is_optimistic() {
        let unknown = HostCapabilities::unknown();
        assert!(unknown.has_tmux && unknown.has_mosh_server && unknown.has_herdr);
    }

    #[test]
    fn herdr_on_path_is_detected_alongside_the_others() {
        let caps = HostCapabilities::parse(
            "/usr/bin/tmux\n/usr/bin/mosh-server\n/home/dev/.local/bin/herdr\n::Linux::/usr/bin/apt-get",
        );
        assert!(caps.has_herdr && caps.has_tmux);
        let tmux_only = HostCapabilities::parse("/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n");
        assert!(!tmux_only.has_herdr && tmux_only.has_tmux);
    }

    #[test]
    fn the_probe_asks_for_herdr_and_searches_local_bin() {
        let cmd = probe_command();
        assert!(cmd.contains("for c in tmux mosh-server herdr python3"));
        assert!(cmd.contains("command -v \"$c\""));
        assert!(EXTRA_PATH_DIRS.contains("$HOME/.local/bin"));
    }

    #[test]
    fn probe_command_finds_multiple_tools_under_posix_sh() {
        use std::os::unix::fs::PermissionsExt;
        let uniq = format!(
            "moshpit-probe-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(uniq);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["tmux", "mosh-server", "herdr", "python3", "pacman"] {
            let bin = dir.join(name);
            std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path_env = format!("{}:/bin:/usr/bin", dir.display());
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &probe_command()])
            .env("PATH", path_env)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.status.success(), "{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let caps = HostCapabilities::parse(&stdout);
        assert!(caps.has_tmux, "missing tmux: {stdout}");
        assert!(caps.has_mosh_server, "missing mosh-server: {stdout}");
        assert!(caps.has_herdr, "missing herdr: {stdout}");
        assert!(caps.has_python3, "missing python3: {stdout}");
        assert_eq!(caps.package_manager, Some(PackageManager::Pacman), "{stdout}");
    }

    #[test]
    fn has_maps_each_multiplexer_to_its_probe() {
        let herdr_only = HostCapabilities {
            has_tmux: false,
            has_mosh_server: false,
            os: "Linux".into(),
            package_manager: Some(PackageManager::AptGet),
            has_herdr: true,
            has_python3: false,
        };
        assert!(herdr_only.has(Multiplexer::Herdr));
        assert!(!herdr_only.has(Multiplexer::Tmux));
        assert!(herdr_only.has(Multiplexer::None));
    }

    #[test]
    fn capabilities_round_trip_through_the_cache_encoding() {
        let caps = HostCapabilities {
            has_tmux: false,
            has_mosh_server: true,
            os: "Darwin".into(),
            package_manager: Some(PackageManager::Brew),
            has_herdr: true,
            has_python3: false,
        };
        let text = moshpit_seal::foundation::json::encode(&caps);
        assert!(text.contains(r#""packageManager":"brew""#), "{text}");
        let back: HostCapabilities =
            moshpit_seal::foundation::json::decode(text.as_bytes()).unwrap();
        assert_eq!(back, caps);
        // Swift's raw value for apt-get is the case name.
        let apt: HostCapabilities = moshpit_seal::foundation::json::decode(
            br#"{"hasTmux":true,"hasMoshServer":true,"os":"Linux","packageManager":"aptGet","hasHerdr":false,"hasPython3":true}"#,
        )
        .unwrap();
        assert_eq!(apt.package_manager, Some(PackageManager::AptGet));
    }

    #[test]
    fn herdr_never_routes_through_a_distro_package_manager() {
        let installer = Some("curl -fsSL https://herdr.dev/install.sh | sh".to_owned());
        assert_eq!(
            Multiplexer::Herdr.install_command(Some(PackageManager::AptGet)),
            installer
        );
        assert_eq!(
            Multiplexer::Herdr.install_command(Some(PackageManager::Pacman)),
            installer
        );
        assert_eq!(Multiplexer::Herdr.install_command(None), installer);
        assert_eq!(
            Multiplexer::Herdr
                .install_command(Some(PackageManager::Brew))
                .as_deref(),
            Some("brew install herdr")
        );
    }

    #[test]
    fn tmux_installs_mosh_alongside_and_none_offers_nothing() {
        assert_eq!(
            Multiplexer::Tmux
                .install_command(Some(PackageManager::AptGet))
                .as_deref(),
            Some("sudo apt-get install -y tmux mosh")
        );
        assert_eq!(Multiplexer::Tmux.install_command(None), None);
        assert_eq!(
            Multiplexer::None.install_command(Some(PackageManager::Brew)),
            None
        );
    }
}
