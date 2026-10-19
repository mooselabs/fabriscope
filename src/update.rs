//! Self-update from GitHub releases.
//!
//! Only release builds update themselves: the release workflow sets
//! `FABRISCOPE_TARGET` at build time, which names the archive to fetch. A
//! check reads the newest release tag, and when it is newer than this build
//! downloads that release's archive and `SHA256SUMS`, verifies the archive,
//! checks that the new program runs, and swaps it in place of the executable.
//! The running process carries on unchanged; the next start uses the new one.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/mooselabs/fabriscope/releases";
pub const INSTALL: &str = "curl -fsSL https://raw.githubusercontent.com/mooselabs/fabriscope/main/scripts/install.sh | bash";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

/// The release target this build updates from: set by the release workflow
/// at build time, or by `FABRISCOPE_TARGET` at run time (tests, or a source
/// build that wants to follow releases).
pub fn target() -> Option<String> {
    option_env!("FABRISCOPE_TARGET")
        .map(str::to_string)
        .or_else(|| std::env::var("FABRISCOPE_TARGET").ok())
        .filter(|t| !t.is_empty())
}

/// `fabriscope 0.1.0 (x86_64-unknown-linux-musl)`, or `(source build)`.
pub fn version_line() -> String {
    format!(
        "fabriscope {VERSION} ({})",
        target().unwrap_or_else(|| "source build".into())
    )
}

/// True when this run should check for updates in the background.
pub fn automatic(no_update_flag: bool) -> bool {
    let opted_out = std::env::var_os("FABRISCOPE_NO_UPDATE").is_some_and(|v| !v.is_empty());
    target().is_some() && !no_update_flag && !opted_out
}

fn releases_url() -> String {
    std::env::var("FABRISCOPE_RELEASES_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| RELEASES.into())
        .trim_end_matches('/')
        .to_string()
}

pub fn archive_name(target: &str) -> String {
    format!("fabriscope-{target}.tar.gz")
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Current(semver::Version),
    Updated(semver::Version),
}

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    /// Retrying will not help (no release target, or the folder is not
    /// writable), so the background check stops.
    pub permanent: bool,
}

impl Failure {
    fn retry(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            permanent: false,
        }
    }
    fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            permanent: true,
        }
    }
}

fn agent(follow_redirects: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(if follow_redirects { 10 } else { 0 })
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent(format!("fabriscope/{VERSION}"))
        .build()
        .into()
}

/// The newest release tag, from where `releases/latest` redirects. This
/// avoids the GitHub API and its rate limit.
fn latest_tag(base: &str) -> Result<String, Failure> {
    let url = format!("{base}/latest");
    let response = agent(false)
        .get(&url)
        .call()
        .map_err(|e| Failure::retry(format!("{url}: {e}")))?;
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            Failure::retry(format!(
                "{url} answered {} without a redirect to a release",
                response.status()
            ))
        })?;
    tag_from_location(location).ok_or_else(|| {
        Failure::retry(format!(
            "{url} redirected to {location}, which names no release"
        ))
    })
}

fn tag_from_location(location: &str) -> Option<String> {
    let tag = location.split_once("/tag/")?.1;
    let tag = tag.split(['?', '#', '/']).next()?;
    (!tag.is_empty()).then(|| tag.to_string())
}

/// A release tag's version, when it is a plain release (`v1.2.3`).
pub fn tag_version(tag: &str) -> Option<semver::Version> {
    let version = semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    version.pre.is_empty().then_some(version)
}

fn download(url: &str) -> Result<Vec<u8>, Failure> {
    agent(true)
        .get(url)
        .call()
        .map_err(|e| Failure::retry(format!("{url}: {e}")))?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| Failure::retry(format!("{url}: {e}")))
}

/// The SHA-256 that `SHA256SUMS` lists for `name` (`sha256sum` format, with
/// or without the binary-mode `*`).
pub fn listed_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| sum.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The `fabriscope` program from a release archive, at its top level or one
/// folder down.
pub fn extract_program(archive: &[u8]) -> Result<Vec<u8>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let is_program = entry.header().entry_type().is_file()
            && path.file_name().is_some_and(|n| n == "fabriscope")
            && path.components().count() <= 2;
        if is_program {
            let mut program = Vec::new();
            entry.read_to_end(&mut program).map_err(|e| e.to_string())?;
            return Ok(program);
        }
    }
    Err("the archive holds no fabriscope program".into())
}

/// Checks for a release newer than `current` and installs it over the
/// running executable. Blocking; the background check runs it on a thread.
pub fn check_and_install(current: &semver::Version) -> Result<Outcome, Failure> {
    let target = target().ok_or_else(|| {
        Failure::permanent(format!(
            "this fabriscope was built from source, so it cannot update itself. Install a release with:\n  {INSTALL}"
        ))
    })?;
    let base = releases_url();
    let tag = latest_tag(&base)?;
    let Some(latest) = tag_version(&tag) else {
        return Err(Failure::retry(format!(
            "the latest release, {tag}, is not a version"
        )));
    };
    if latest <= *current {
        return Ok(Outcome::Current(current.clone()));
    }
    let name = archive_name(&target);
    let archive = download(&format!("{base}/download/{tag}/{name}"))?;
    let sums = download(&format!("{base}/download/{tag}/SHA256SUMS"))?;
    let sums = String::from_utf8_lossy(&sums);
    let expected = listed_sum(&sums, &name)
        .ok_or_else(|| Failure::retry(format!("{tag} has no SHA256SUMS entry for {name}")))?;
    if sha256_hex(&archive) != expected {
        return Err(Failure::retry(format!(
            "{name} from {tag} does not match its SHA256SUMS entry; not installing it"
        )));
    }
    let program = extract_program(&archive).map_err(|e| Failure::retry(format!("{name}: {e}")))?;
    install(&program, &latest)?;
    Ok(Outcome::Updated(latest))
}

/// Writes the new program beside the executable, checks that it runs and
/// reports the expected version, then swaps it in with one rename.
fn install(program: &[u8], version: &semver::Version) -> Result<(), Failure> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| Failure::permanent(format!("cannot find this program on disk: {e}")))?;
    let dir = exe.parent().unwrap_or(Path::new("."));
    let staged = dir.join(format!(".fabriscope-update-{}", std::process::id()));
    let result = stage_and_swap(program, version, &exe, &staged);
    let _ = std::fs::remove_file(&staged);
    result
}

fn stage_and_swap(
    program: &[u8],
    version: &semver::Version,
    exe: &Path,
    staged: &Path,
) -> Result<(), Failure> {
    let unwritable = |e: std::io::Error| {
        let message = format!("cannot write next to {}: {e}", exe.display());
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Failure::permanent(format!(
                "{message}. Reinstall into a folder you own (the installer uses ~/.local/bin), or update with your package manager"
            ))
        } else {
            Failure::retry(message)
        }
    };
    std::fs::write(staged, program).map_err(unwritable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
            .map_err(unwritable)?;
    }
    let output = run_fresh(staged)
        .map_err(|e| Failure::retry(format!("the downloaded program does not run: {e}")))?;
    let reported = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !reported.starts_with(&format!("fabriscope {version} ")) {
        return Err(Failure::retry(format!(
            "the downloaded program reported {:?}, not version {version}; not installing it",
            reported.trim()
        )));
    }
    self_replace::self_replace(staged).map_err(unwritable)
}

/// Runs a just-written program with `--version`. A child forked by another
/// thread while the file was open for writing can hold it briefly, which
/// shows up as "text file busy", so that error is retried.
fn run_fresh(program: &Path) -> std::io::Result<std::process::Output> {
    let mut attempts = 0;
    loop {
        match std::process::Command::new(program)
            .arg("--version")
            .output()
        {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }
}

/// Checks at start and once a day; installs a newer release and says so.
/// Stops when retrying cannot help. `installed` holds the version now on
/// disk, for `/healthz`.
pub async fn watch(installed: Arc<Mutex<Option<String>>>) {
    let mut current = semver::Version::parse(VERSION).expect("the crate version is semver");
    loop {
        let check = current.clone();
        match tokio::task::spawn_blocking(move || check_and_install(&check)).await {
            Ok(Ok(Outcome::Current(_))) => {}
            Ok(Ok(Outcome::Updated(version))) => {
                eprintln!("fabriscope: updated to v{version}; restart to use it");
                *installed.lock().unwrap() = Some(version.to_string());
                current = version;
            }
            Ok(Err(failure)) if failure.permanent => {
                eprintln!(
                    "fabriscope: automatic updates are off for this run: {}",
                    failure.message
                );
                return;
            }
            Ok(Err(failure)) => eprintln!("fabriscope: update check failed: {}", failure.message),
            Err(error) => eprintln!("fabriscope: update check failed: {error}"),
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_newer_releases_count() {
        let v = |s| semver::Version::parse(s).unwrap();
        assert_eq!(tag_version("v0.2.0"), Some(v("0.2.0")));
        assert_eq!(tag_version("0.2.0"), Some(v("0.2.0")));
        assert_eq!(tag_version("v0.2.0-rc.1"), None);
        assert_eq!(tag_version("nightly"), None);
        assert!(v("0.10.0") > v("0.9.9"));
    }

    #[test]
    fn the_tag_comes_from_the_latest_redirect() {
        assert_eq!(
            tag_from_location("https://github.com/mooselabs/fabriscope/releases/tag/v0.1.0")
                .as_deref(),
            Some("v0.1.0")
        );
        assert_eq!(
            tag_from_location("/releases/tag/v1.2.3?x=1").as_deref(),
            Some("v1.2.3")
        );
        assert_eq!(
            tag_from_location("https://github.com/mooselabs/fabriscope/releases"),
            None
        );
        assert_eq!(tag_from_location("/releases/tag/"), None);
    }

    #[test]
    fn sums_are_read_in_sha256sum_format() {
        let sums = "ABC123  fabriscope-x86_64-unknown-linux-musl.tar.gz\n\
                    def456 *fabriscope-aarch64-apple-darwin.tar.gz\n";
        assert_eq!(
            listed_sum(sums, "fabriscope-x86_64-unknown-linux-musl.tar.gz").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            listed_sum(sums, "fabriscope-aarch64-apple-darwin.tar.gz").as_deref(),
            Some("def456")
        );
        assert_eq!(listed_sum(sums, "fabriscope.tar.gz"), None);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, path, *body).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extraction_takes_only_the_program() {
        let top = archive(&[("README.md", b"readme"), ("fabriscope", b"program")]);
        assert_eq!(extract_program(&top).unwrap(), b"program");
        let nested = archive(&[("fabriscope-x/fabriscope", b"nested")]);
        assert_eq!(extract_program(&nested).unwrap(), b"nested");
        let deep = archive(&[("a/b/fabriscope", b"too deep")]);
        assert!(extract_program(&deep).is_err());
        assert!(extract_program(b"not an archive").is_err());
    }
}
