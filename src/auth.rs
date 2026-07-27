use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

use crate::journey::Origin;

pub const AGENT_BROWSER_STATE_FILE_PROVIDER: &str = "agent-browser-state-file";

const MAX_STATE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COOKIES: usize = 4_096;
const MAX_ORIGINS: usize = 128;
const MAX_STORAGE_ITEMS: usize = 4_096;
const MAX_NAME_BYTES: usize = 4_096;
const MAX_VALUE_BYTES: usize = 65_536;

pub struct ValidatedState {
    bytes: Zeroizing<Vec<u8>>,
}

pub struct StagedState {
    directory: tempfile::TempDir,
    path: PathBuf,
}

impl ValidatedState {
    pub fn load(path: &Path, origin: &Origin) -> Result<Self, StateError> {
        let path_metadata = fs::symlink_metadata(path).map_err(|_| StateError::Invalid)?;
        if path_metadata.file_type().is_symlink() || !path_metadata.file_type().is_file() {
            return Err(StateError::Invalid);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if path_metadata.mode() & 0o077 != 0 {
                return Err(StateError::Invalid);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if path_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(StateError::Invalid);
            }
        }
        if path_metadata.len() == 0 || path_metadata.len() > MAX_STATE_BYTES {
            return Err(StateError::Invalid);
        }

        let mut open_options = OpenOptions::new();
        open_options.read(true);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            const O_NOFOLLOW: i32 = 0x0002_0000;
            open_options.custom_flags(O_NOFOLLOW);
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            const O_NOFOLLOW: i32 = 0x0000_0100;
            open_options.custom_flags(O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            open_options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let mut file = open_options.open(path).map_err(|_| StateError::Invalid)?;
        let opened_metadata = file.metadata().map_err(|_| StateError::Invalid)?;
        if !opened_metadata.is_file()
            || opened_metadata.len() != path_metadata.len()
            || opened_metadata.len() == 0
            || opened_metadata.len() > MAX_STATE_BYTES
        {
            return Err(StateError::Invalid);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if opened_metadata.dev() != path_metadata.dev()
                || opened_metadata.ino() != path_metadata.ino()
                || opened_metadata.mode() & 0o077 != 0
            {
                return Err(StateError::Invalid);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if opened_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || opened_metadata.volume_serial_number().is_none()
                || opened_metadata.volume_serial_number() != path_metadata.volume_serial_number()
                || opened_metadata.file_index().is_none()
                || opened_metadata.file_index() != path_metadata.file_index()
            {
                return Err(StateError::Invalid);
            }
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(opened_metadata.len() as usize));
        (&mut file)
            .take(MAX_STATE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| StateError::Invalid)?;
        let after_metadata = file.metadata().map_err(|_| StateError::Invalid)?;
        if bytes.len() as u64 != opened_metadata.len()
            || after_metadata.len() != opened_metadata.len()
            || after_metadata.modified().ok() != opened_metadata.modified().ok()
        {
            return Err(StateError::Invalid);
        }

        let document: BrowserState =
            serde_json::from_slice(bytes.as_slice()).map_err(|_| StateError::Invalid)?;
        document.validate(origin)?;
        Ok(Self { bytes })
    }

    pub fn stage(&self) -> Result<StagedState, StateError> {
        let directory = tempfile::Builder::new()
            .prefix("crawlson-auth-")
            .tempdir()
            .map_err(|_| StateError::Stage)?;
        let path = directory.path().join("state.json");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|_| StateError::Stage)?;
        file.write_all(&self.bytes).map_err(|_| StateError::Stage)?;
        file.sync_all().map_err(|_| StateError::Stage)?;
        drop(file);
        Ok(StagedState { directory, path })
    }
}

impl StagedState {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn close(self) -> Result<(), StateError> {
        self.directory.close().map_err(|_| StateError::Cleanup)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    Invalid,
    Stage,
    Cleanup,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserState {
    cookies: Vec<Cookie>,
    origins: Vec<StorageOrigin>,
}

impl BrowserState {
    fn validate(&self, target: &Origin) -> Result<(), StateError> {
        if self.cookies.len() > MAX_COOKIES || self.origins.len() > MAX_ORIGINS {
            return Err(StateError::Invalid);
        }
        let mut cookie_keys = HashSet::new();
        for cookie in &self.cookies {
            if cookie.name.is_empty()
                || cookie.name.len() > MAX_NAME_BYTES
                || cookie.value.len() > MAX_VALUE_BYTES
                || cookie.domain != target.host
                || cookie.path.is_empty()
                || !cookie.path.starts_with('/')
                || cookie.path.starts_with("//")
                || cookie.path.contains('\\')
                || cookie.path.chars().any(char::is_control)
                || cookie.expires.is_some_and(|value| !value.is_finite())
                || cookie
                    .same_site
                    .as_deref()
                    .is_some_and(|value| !matches!(value, "Strict" | "Lax" | "None"))
                || !cookie_keys.insert((
                    cookie.domain.as_str(),
                    cookie.path.as_str(),
                    cookie.name.as_str(),
                ))
            {
                return Err(StateError::Invalid);
            }
            let _accepted_upstream_metadata =
                (cookie.http_only, cookie.secure, cookie.session, cookie.size);
        }
        // agent-browser 0.26 restricts traffic by hostname rather than exact
        // origin. Browser cookies are also port-agnostic, so importing even an
        // exact-host cookie could disclose it to another port on that host.
        // Keep the first provider origin-bound until the driver boundary can
        // prevent that request before navigation.
        if !self.cookies.is_empty() {
            return Err(StateError::Invalid);
        }

        let mut origins = HashSet::new();
        let mut has_storage = false;
        for entry in &self.origins {
            if entry.local_storage.len() > MAX_STORAGE_ITEMS
                || entry.session_storage.len() > MAX_STORAGE_ITEMS
                || Origin::parse(&entry.origin).ok().as_ref() != Some(target)
                || !origins.insert(entry.origin.as_str())
            {
                return Err(StateError::Invalid);
            }
            validate_storage(&entry.local_storage)?;
            validate_storage(&entry.session_storage)?;
            has_storage |= !entry.local_storage.is_empty() || !entry.session_storage.is_empty();
        }
        if self.cookies.is_empty() && !has_storage {
            return Err(StateError::Invalid);
        }
        Ok(())
    }
}

fn validate_storage(items: &[StorageItem]) -> Result<(), StateError> {
    let mut names = HashSet::new();
    for item in items {
        if item.name.is_empty()
            || item.name.len() > MAX_NAME_BYTES
            || item.value.len() > MAX_VALUE_BYTES
            || !names.insert(item.name.as_str())
        {
            return Err(StateError::Invalid);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    #[serde(default)]
    expires: Option<f64>,
    #[serde(default, rename = "httpOnly")]
    http_only: Option<bool>,
    #[serde(default)]
    secure: Option<bool>,
    #[serde(default)]
    session: Option<bool>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default, rename = "sameSite")]
    same_site: Option<String>,
}

impl Drop for Cookie {
    fn drop(&mut self) {
        self.name.zeroize();
        self.value.zeroize();
        self.domain.zeroize();
        self.path.zeroize();
        if let Some(same_site) = &mut self.same_site {
            same_site.zeroize();
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageOrigin {
    origin: String,
    #[serde(default, rename = "localStorage")]
    local_storage: Vec<StorageItem>,
    #[serde(default, rename = "sessionStorage")]
    session_storage: Vec<StorageItem>,
}

impl Drop for StorageOrigin {
    fn drop(&mut self) {
        self.origin.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageItem {
    name: String,
    value: String,
}

impl Drop for StorageItem {
    fn drop(&mut self) {
        self.name.zeroize();
        self.value.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_state_bytes() -> Vec<u8> {
        br#"{"cookies":[],"origins":[{"origin":"http://127.0.0.1:4173","localStorage":[{"name":"session","value":"fixture"}]}]}"#.to_vec()
    }

    fn write_private(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn origin() -> Origin {
        Origin::parse("http://127.0.0.1:4173").unwrap()
    }

    #[test]
    fn accepts_exact_origin_storage_and_rejects_cookies_or_cross_origin_state() {
        let cookie: BrowserState = serde_json::from_str(
            r#"{"cookies":[{"name":"session","value":"fixture","domain":"127.0.0.1","path":"/","expires":-1,"httpOnly":true,"secure":false,"session":true,"size":14,"sameSite":"Lax"}],"origins":[]}"#,
        )
        .unwrap();
        assert_eq!(cookie.validate(&origin()), Err(StateError::Invalid));

        let storage: BrowserState = serde_json::from_str(
            r#"{"cookies":[],"origins":[{"origin":"http://127.0.0.1:4173","localStorage":[{"name":"role","value":"viewer"}]}]}"#,
        )
        .unwrap();
        assert!(storage.validate(&origin()).is_ok());

        let off_origin: BrowserState = serde_json::from_str(
            r#"{"cookies":[{"name":"session","value":"fixture","domain":"other.example","path":"/"}],"origins":[]}"#,
        )
        .unwrap();
        assert_eq!(off_origin.validate(&origin()), Err(StateError::Invalid));
    }

    #[test]
    fn rejects_empty_duplicate_or_ambiguous_state() {
        let empty: BrowserState = serde_json::from_str(r#"{"cookies":[],"origins":[]}"#).unwrap();
        assert_eq!(empty.validate(&origin()), Err(StateError::Invalid));

        let duplicate: BrowserState = serde_json::from_str(
            r#"{"cookies":[],"origins":[{"origin":"http://127.0.0.1:4173","localStorage":[{"name":"session","value":"one"},{"name":"session","value":"two"}]}]}"#,
        )
        .unwrap();
        assert_eq!(duplicate.validate(&origin()), Err(StateError::Invalid));
    }

    #[test]
    fn source_file_validation_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            ValidatedState::load(&directory.path().join("missing.json"), &origin()),
            Err(StateError::Invalid)
        ));
        assert!(matches!(
            ValidatedState::load(directory.path(), &origin()),
            Err(StateError::Invalid)
        ));

        for (name, contents) in [
            ("empty.json", b"".as_slice()),
            ("bad.json", b"{}".as_slice()),
        ] {
            let path = directory.path().join(name);
            write_private(&path, contents);
            assert!(matches!(
                ValidatedState::load(&path, &origin()),
                Err(StateError::Invalid)
            ));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};

            let broad = directory.path().join("broad.json");
            write_private(&broad, &valid_state_bytes());
            fs::set_permissions(&broad, fs::Permissions::from_mode(0o640)).unwrap();
            assert!(matches!(
                ValidatedState::load(&broad, &origin()),
                Err(StateError::Invalid)
            ));

            let private = directory.path().join("private.json");
            write_private(&private, &valid_state_bytes());
            let link = directory.path().join("linked.json");
            symlink(&private, &link).unwrap();
            assert!(matches!(
                ValidatedState::load(&link, &origin()),
                Err(StateError::Invalid)
            ));
        }
    }

    #[test]
    fn staged_copy_has_a_neutral_private_name_and_is_explicitly_removed() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("secret-source-name.json");
        let bytes = valid_state_bytes();
        write_private(&source, &bytes);
        let state = ValidatedState::load(&source, &origin()).unwrap();
        let staged = state.stage().unwrap();
        let staged_path = staged.path().to_path_buf();

        assert_eq!(staged_path.file_name().unwrap(), "state.json");
        assert!(!staged_path.starts_with(directory.path()));
        assert_eq!(fs::read(&staged_path).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&staged_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        staged.close().unwrap();
        assert!(!staged_path.exists());
    }
}
