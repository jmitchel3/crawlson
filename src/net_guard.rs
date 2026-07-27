use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use atomic_write_file::AtomicWriteFile;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

use crate::journey::Origin;

const CONTROL_DIRECTORY: &str = "control";
const EXTENSION_DIRECTORY: &str = "exact-origin-extension";
const MANIFEST_FILE: &str = "manifest.json";
const RULES_FILE: &str = "rules.json";
const CONTENT_FILE: &str = "content.js";
const MARKER_NAME: &str = "crawlson-exact-origin-guard";
const MAX_EXTENSION_FILE_BYTES: usize = 64 * 1024;

// This is the complete ResourceType enum currently accepted by Chrome DNR.
// `fetch()` and XHR requests are both classified as `xmlhttprequest`.
const RESOURCE_TYPES: [&str; 15] = [
    "main_frame",
    "sub_frame",
    "stylesheet",
    "script",
    "image",
    "font",
    "object",
    "xmlhttprequest",
    "ping",
    "csp_report",
    "media",
    "websocket",
    "webtransport",
    "webbundle",
    "other",
];

/// A materialized Chrome extension that blocks network access outside one
/// exact HTTP(S) origin.
///
/// Call [`ExactOriginGuard::verify`] immediately before each browser command.
/// The guard deliberately does not allow `ws` or `wss`, including endpoints
/// whose host and port otherwise match the target.
#[derive(Debug, Clone)]
pub struct ExactOriginGuard {
    run_root: PathBuf,
    extension_path: PathBuf,
    marker_token: String,
    marker_selector: String,
    manifest: ExpectedFile,
    rules: ExpectedFile,
    content: ExpectedFile,
}

#[derive(Debug, Clone)]
struct ExpectedFile {
    bytes: Vec<u8>,
    sha256: String,
}

impl ExpectedFile {
    fn new(bytes: Vec<u8>) -> Self {
        let sha256 = hex_digest(&bytes);
        Self { bytes, sha256 }
    }
}

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("invalid exact-origin network guard target: {0}")]
    InvalidOrigin(String),
    #[error("unsafe exact-origin network guard path: {0}")]
    UnsafePath(String),
    #[error("could not {operation} exact-origin network guard: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize exact-origin network guard: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("exact-origin network guard integrity check failed: {0}")]
    Integrity(String),
}

impl ExactOriginGuard {
    /// Materialize a static Manifest V3 Declarative Net Request extension
    /// beneath `<run_root>/control/exact-origin-extension`.
    pub fn materialize(run_root: &Path, origin: &Origin) -> Result<Self, GuardError> {
        let target = GuardTarget::from_origin(origin)?;

        let run_root = canonical_directory(run_root, "run root")?;
        let control = ensure_direct_child_directory(&run_root, CONTROL_DIRECTORY)?;
        let extension_path = ensure_direct_child_directory(&control, EXTENSION_DIRECTORY)?;
        let marker_token = attestation_token(&control)?;
        let marker_selector = format!("meta[name=\"{MARKER_NAME}\"][content=\"{marker_token}\"]");
        let manifest = ExpectedFile::new(json_bytes(manifest_document(&target))?);
        let rules = ExpectedFile::new(json_bytes(rules_document(&target.allow_regex))?);
        let content = ExpectedFile::new(content_script(&target.canonical_origin, &marker_token)?);

        verify_directory_entries_for_materialization(&extension_path)?;
        atomic_write_bounded(&extension_path.join(MANIFEST_FILE), &manifest.bytes)?;
        atomic_write_bounded(&extension_path.join(RULES_FILE), &rules.bytes)?;
        atomic_write_bounded(&extension_path.join(CONTENT_FILE), &content.bytes)?;

        let guard = Self {
            run_root,
            extension_path,
            marker_token,
            marker_selector,
            manifest,
            rules,
            content,
        };
        guard.verify()?;
        Ok(guard)
    }

    /// Return the canonical directory that should be passed to
    /// `agent-browser --extension`.
    pub fn extension_path(&self) -> &Path {
        &self.extension_path
    }

    /// Return a CSS selector that only this materialized extension can inject.
    pub fn marker_selector(&self) -> &str {
        &self.marker_selector
    }

    /// Return the per-run token embedded in the marker.
    pub fn marker_token(&self) -> &str {
        &self.marker_token
    }

    /// Fail closed unless the extension directory and all three files are unchanged,
    /// regular, non-symlink files with the exact materialized bytes.
    pub fn verify(&self) -> Result<(), GuardError> {
        verify_existing_directory(&self.run_root, "run root")?;

        let control = self.run_root.join(CONTROL_DIRECTORY);
        verify_direct_child_directory(&self.run_root, &control, "control directory")?;
        verify_direct_child_directory(&control, &self.extension_path, "extension directory")?;
        verify_exact_directory_entries(&self.extension_path)?;

        verify_expected_file(&self.extension_path, MANIFEST_FILE, &self.manifest)?;
        verify_expected_file(&self.extension_path, RULES_FILE, &self.rules)?;
        verify_expected_file(&self.extension_path, CONTENT_FILE, &self.content)?;
        Ok(())
    }
}

fn manifest_document(target: &GuardTarget) -> Value {
    json!({
        "manifest_version": 3,
        "name": "Crawlson Exact-Origin Network Guard",
        "version": "1.0.0",
        "minimum_chrome_version": "120",
        "description": "Blocks browser network access outside the authorized Crawlson origin.",
        "permissions": ["declarativeNetRequest"],
        "declarative_net_request": {
            "rule_resources": [{
                "id": "exact_origin_rules",
                "enabled": true,
                "path": RULES_FILE
            }]
        },
        "content_scripts": [{
            "matches": [target.match_pattern],
            "include_globs": [format!("{}/*", target.canonical_origin)],
            "js": [CONTENT_FILE],
            "run_at": "document_start",
            "all_frames": false,
            "match_about_blank": false,
            "match_origin_as_fallback": false,
            "world": "ISOLATED"
        }]
    })
}

fn rules_document(allow_regex: &str) -> Value {
    json!([
        {
            "id": 1,
            "priority": 2,
            "action": { "type": "allow" },
            "condition": {
                "regexFilter": allow_regex,
                "isUrlFilterCaseSensitive": false,
                "resourceTypes": RESOURCE_TYPES
            }
        },
        {
            "id": 2,
            "priority": 1,
            "action": { "type": "block" },
            "condition": {
                "regexFilter": "^(http|https|ws|wss)://",
                "isUrlFilterCaseSensitive": false,
                "resourceTypes": RESOURCE_TYPES
            }
        }
    ])
}

#[derive(Debug, PartialEq, Eq)]
struct GuardTarget {
    allow_regex: String,
    canonical_origin: String,
    match_pattern: String,
}

impl GuardTarget {
    fn from_origin(origin: &Origin) -> Result<Self, GuardError> {
        let normalized = normalize_origin(origin)?;
        let scheme = regex_escape(&normalized.scheme);
        let host = regex_escape(&normalized.uri_host);
        let default_port = default_port(&normalized.scheme);
        let port = if normalized.port == default_port {
            format!("(:{})?", normalized.port)
        } else {
            format!(":{}", normalized.port)
        };
        let canonical_port = if normalized.port == default_port {
            String::new()
        } else {
            format!(":{}", normalized.port)
        };

        Ok(Self {
            // The authority boundary prevents a default-port origin such as
            // example.test:443 from also matching example.test:4430.
            allow_regex: format!("^{scheme}://{host}{port}([/?#]|$)"),
            canonical_origin: format!(
                "{}://{}{}",
                normalized.scheme, normalized.uri_host, canonical_port
            ),
            // Chrome match patterns intentionally omit ports. include_globs
            // and the script's location.origin check narrow this to the exact
            // effective port.
            match_pattern: format!("{}://{}/*", normalized.scheme, normalized.uri_host),
        })
    }
}

#[cfg(test)]
fn allow_regex(origin: &Origin) -> Result<String, GuardError> {
    Ok(GuardTarget::from_origin(origin)?.allow_regex)
}

fn default_port(scheme: &str) -> u16 {
    match scheme {
        "http" => 80,
        "https" => 443,
        _ => unreachable!("normalize_origin accepts only http and https"),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NormalizedOrigin {
    scheme: String,
    uri_host: String,
    port: u16,
}

fn normalize_origin(origin: &Origin) -> Result<NormalizedOrigin, GuardError> {
    if !matches!(origin.scheme.as_str(), "http" | "https") {
        return Err(GuardError::InvalidOrigin(
            "scheme must be http or https".to_owned(),
        ));
    }
    if origin.effective_port == 0 {
        return Err(GuardError::InvalidOrigin(
            "effective port must be between 1 and 65535".to_owned(),
        ));
    }
    if origin.host.is_empty() || origin.host.chars().any(char::is_control) {
        return Err(GuardError::InvalidOrigin(
            "host must be non-empty and contain no control characters".to_owned(),
        ));
    }

    let (bare_host, bracketed) =
        match (origin.host.strip_prefix('['), origin.host.strip_suffix(']')) {
            (Some(without_open), Some(_)) => (
                without_open.strip_suffix(']').ok_or_else(|| {
                    GuardError::InvalidOrigin("host has malformed IPv6 brackets".to_owned())
                })?,
                true,
            ),
            (None, None) => (origin.host.as_str(), false),
            _ => {
                return Err(GuardError::InvalidOrigin(
                    "host has malformed IPv6 brackets".to_owned(),
                ));
            }
        };

    let uri_host = if bare_host.contains(':') {
        let address = bare_host.parse::<Ipv6Addr>().map_err(|_| {
            GuardError::InvalidOrigin(
                "colon-containing host must be a valid IPv6 address".to_owned(),
            )
        })?;
        format!("[{address}]")
    } else {
        if bracketed {
            return Err(GuardError::InvalidOrigin(
                "brackets are only valid around an IPv6 address".to_owned(),
            ));
        }
        if origin.host != origin.host.to_ascii_lowercase() {
            return Err(GuardError::InvalidOrigin(
                "host must use its canonical lowercase spelling".to_owned(),
            ));
        }
        origin.host.clone()
    };

    let candidate = format!(
        "{}://{}:{}/",
        origin.scheme, uri_host, origin.effective_port
    );
    let parsed = Url::parse(&candidate).map_err(|error| {
        GuardError::InvalidOrigin(format!("host does not form a valid URL authority: {error}"))
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(GuardError::InvalidOrigin(
            "host must not contain credentials".to_owned(),
        ));
    }
    if parsed.port_or_known_default() != Some(origin.effective_port) {
        return Err(GuardError::InvalidOrigin(
            "effective port did not round-trip through the URL parser".to_owned(),
        ));
    }
    let parsed_host = parsed
        .host_str()
        .ok_or_else(|| GuardError::InvalidOrigin("host did not parse".to_owned()))?;
    if bare_ipv6_brackets(parsed_host) != bare_ipv6_brackets(&uri_host) {
        return Err(GuardError::InvalidOrigin(
            "host must use its canonical URL spelling".to_owned(),
        ));
    }

    Ok(NormalizedOrigin {
        scheme: origin.scheme.clone(),
        uri_host,
        port: origin.effective_port,
    })
}

fn bare_ipv6_brackets(value: &str) -> &str {
    value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value)
}

fn regex_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn attestation_token(control: &Path) -> Result<String, GuardError> {
    let nonce = tempfile::Builder::new()
        .prefix(".exact-origin-attestation-")
        .tempfile_in(control)
        .map_err(|source| GuardError::Io {
            operation: "create attestation nonce for",
            source,
        })?;
    let random_name = nonce
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            GuardError::UnsafePath("attestation nonce name is not valid UTF-8".to_owned())
        })?
        .to_owned();
    nonce.close().map_err(|source| GuardError::Io {
        operation: "remove attestation nonce for",
        source,
    })?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut digest = Sha256::new();
    digest.update(b"crawlson-exact-origin-attestation-v1\0");
    digest.update(random_name.as_bytes());
    digest.update(std::process::id().to_le_bytes());
    digest.update(timestamp.to_le_bytes());
    Ok(hex_digest(&digest.finalize()))
}

fn content_script(expected_origin: &str, marker_token: &str) -> Result<Vec<u8>, GuardError> {
    let expected_origin = serde_json::to_string(expected_origin)?;
    let marker_name = serde_json::to_string(MARKER_NAME)?;
    let marker_token = serde_json::to_string(marker_token)?;
    let source = format!(
        r#"(() => {{
  "use strict";
  const expectedOrigin = {expected_origin};
  const markerName = {marker_name};
  const markerToken = {marker_token};
  if (location.origin !== expectedOrigin) {{
    return;
  }}
  const selector = `meta[name="${{markerName}}"][content="${{markerToken}}"]`;
  let installing = false;
  const install = () => {{
    if (installing || document.querySelector(selector)) {{
      return;
    }}
    const parent = document.head || document.documentElement;
    if (!parent) {{
      return;
    }}
    installing = true;
    const marker = document.createElement("meta");
    marker.name = markerName;
    marker.content = markerToken;
    marker.hidden = true;
    parent.appendChild(marker);
    installing = false;
  }};
  install();
  new MutationObserver(install).observe(document, {{ childList: true, subtree: true }});
}})();
"#
    );
    let bytes = source.into_bytes();
    if bytes.is_empty() || bytes.len() > MAX_EXTENSION_FILE_BYTES {
        return Err(GuardError::Integrity(
            "generated content script exceeds its byte limit".to_owned(),
        ));
    }
    Ok(bytes)
}

fn json_bytes(value: Value) -> Result<Vec<u8>, GuardError> {
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    if bytes.is_empty() || bytes.len() > MAX_EXTENSION_FILE_BYTES {
        return Err(GuardError::Integrity(
            "generated extension file exceeds its byte limit".to_owned(),
        ));
    }
    Ok(bytes)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, GuardError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| GuardError::Io {
        operation: "inspect",
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(GuardError::UnsafePath(format!(
            "{label} must be an existing non-symlink directory"
        )));
    }
    path.canonicalize().map_err(|source| GuardError::Io {
        operation: "canonicalize",
        source,
    })
}

fn ensure_direct_child_directory(parent: &Path, name: &str) -> Result<PathBuf, GuardError> {
    verify_existing_directory(parent, "parent directory")?;
    let child = parent.join(name);
    match fs::create_dir(&child) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(GuardError::Io {
                operation: "create directory for",
                source,
            });
        }
    }
    verify_direct_child_directory(parent, &child, name)?;
    child.canonicalize().map_err(|source| GuardError::Io {
        operation: "canonicalize directory for",
        source,
    })
}

fn verify_existing_directory(path: &Path, label: &str) -> Result<(), GuardError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| GuardError::Io {
        operation: "inspect",
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(GuardError::Integrity(format!(
            "{label} is not a non-symlink directory"
        )));
    }
    Ok(())
}

fn verify_direct_child_directory(
    parent: &Path,
    child: &Path,
    label: &str,
) -> Result<(), GuardError> {
    verify_existing_directory(parent, "parent directory")?;
    verify_existing_directory(child, label)?;

    let canonical_parent = parent.canonicalize().map_err(|source| GuardError::Io {
        operation: "canonicalize parent directory for",
        source,
    })?;
    let canonical_child = child.canonicalize().map_err(|source| GuardError::Io {
        operation: "canonicalize child directory for",
        source,
    })?;
    if canonical_child.parent() != Some(canonical_parent.as_path())
        || canonical_child.file_name() != child.file_name()
    {
        return Err(GuardError::UnsafePath(format!(
            "{label} is not the expected direct child directory"
        )));
    }
    Ok(())
}

fn verify_directory_entries_for_materialization(path: &Path) -> Result<(), GuardError> {
    let allowed = BTreeSet::from([MANIFEST_FILE, RULES_FILE, CONTENT_FILE]);
    for entry in fs::read_dir(path).map_err(|source| GuardError::Io {
        operation: "read extension directory for",
        source,
    })? {
        let entry = entry.map_err(|source| GuardError::Io {
            operation: "read extension directory entry for",
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            GuardError::UnsafePath("extension contains a non-UTF-8 entry name".to_owned())
        })?;
        if !allowed.contains(name) {
            return Err(GuardError::UnsafePath(format!(
                "extension contains unexpected entry '{name}'"
            )));
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|source| GuardError::Io {
            operation: "inspect existing extension entry for",
            source,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(GuardError::UnsafePath(format!(
                "existing extension entry '{name}' is not a regular non-symlink file"
            )));
        }
    }
    Ok(())
}

fn verify_exact_directory_entries(path: &Path) -> Result<(), GuardError> {
    let expected = BTreeSet::from([
        MANIFEST_FILE.to_owned(),
        RULES_FILE.to_owned(),
        CONTENT_FILE.to_owned(),
    ]);
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(path).map_err(|source| GuardError::Io {
        operation: "read extension directory for",
        source,
    })? {
        let entry = entry.map_err(|source| GuardError::Io {
            operation: "read extension directory entry for",
            source,
        })?;
        let name = entry.file_name().into_string().map_err(|_| {
            GuardError::Integrity("extension contains a non-UTF-8 entry name".to_owned())
        })?;
        actual.insert(name);
    }
    if actual != expected {
        return Err(GuardError::Integrity(
            "extension directory does not contain exactly manifest.json, rules.json, and content.js"
                .to_owned(),
        ));
    }
    Ok(())
}

fn atomic_write_bounded(path: &Path, bytes: &[u8]) -> Result<(), GuardError> {
    if bytes.is_empty() || bytes.len() > MAX_EXTENSION_FILE_BYTES {
        return Err(GuardError::Integrity(
            "refused to write an empty or oversized extension file".to_owned(),
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(GuardError::UnsafePath(
            "refused to replace a non-regular extension file".to_owned(),
        ));
    }

    let mut file = AtomicWriteFile::open(path).map_err(|source| GuardError::Io {
        operation: "open atomic file for",
        source,
    })?;
    file.write_all(bytes).map_err(|source| GuardError::Io {
        operation: "write atomic file for",
        source,
    })?;
    file.commit().map_err(|source| GuardError::Io {
        operation: "commit atomic file for",
        source,
    })
}

fn verify_expected_file(
    directory: &Path,
    name: &str,
    expected: &ExpectedFile,
) -> Result<(), GuardError> {
    if expected.bytes.is_empty() || expected.bytes.len() > MAX_EXTENSION_FILE_BYTES {
        return Err(GuardError::Integrity(format!(
            "expected bytes for '{name}' are invalid"
        )));
    }
    let path = directory.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(|source| GuardError::Io {
        operation: "inspect extension file for",
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(GuardError::Integrity(format!(
            "'{name}' is not a regular non-symlink file"
        )));
    }
    if metadata.len() != expected.bytes.len() as u64 {
        return Err(GuardError::Integrity(format!(
            "'{name}' does not have the expected byte length"
        )));
    }
    let canonical = path.canonicalize().map_err(|source| GuardError::Io {
        operation: "canonicalize extension file for",
        source,
    })?;
    if canonical.parent() != Some(directory)
        || canonical.file_name() != Some(std::ffi::OsStr::new(name))
    {
        return Err(GuardError::Integrity(format!(
            "'{name}' is not contained directly in the extension directory"
        )));
    }

    let mut file = fs::File::open(&path).map_err(|source| GuardError::Io {
        operation: "open extension file for",
        source,
    })?;
    let opened_metadata = file.metadata().map_err(|source| GuardError::Io {
        operation: "inspect opened extension file for",
        source,
    })?;
    if !opened_metadata.is_file() || opened_metadata.len() != expected.bytes.len() as u64 {
        return Err(GuardError::Integrity(format!(
            "'{name}' changed while it was opened"
        )));
    }

    let mut actual = Vec::with_capacity(expected.bytes.len());
    Read::by_ref(&mut file)
        .take((MAX_EXTENSION_FILE_BYTES + 1) as u64)
        .read_to_end(&mut actual)
        .map_err(|source| GuardError::Io {
            operation: "read extension file for",
            source,
        })?;
    if actual != expected.bytes || hex_digest(&actual) != expected.sha256 {
        return Err(GuardError::Integrity(format!(
            "'{name}' bytes do not match the materialized extension"
        )));
    }

    let final_metadata = fs::symlink_metadata(&path).map_err(|source| GuardError::Io {
        operation: "reinspect extension file for",
        source,
    })?;
    if final_metadata.file_type().is_symlink()
        || !final_metadata.is_file()
        || final_metadata.len() != expected.bytes.len() as u64
    {
        return Err(GuardError::Integrity(format!(
            "'{name}' changed during verification"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened_metadata.dev() != final_metadata.dev()
            || opened_metadata.ino() != final_metadata.ino()
        {
            return Err(GuardError::Integrity(format!(
                "'{name}' was replaced during verification"
            )));
        }
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn materialized(origin: &str) -> (tempfile::TempDir, ExactOriginGuard) {
        let directory = tempfile::tempdir().unwrap();
        let guard =
            ExactOriginGuard::materialize(directory.path(), &Origin::parse(origin).unwrap())
                .unwrap();
        (directory, guard)
    }

    fn read_json(guard: &ExactOriginGuard, name: &str) -> Value {
        serde_json::from_slice(&fs::read(guard.extension_path().join(name)).unwrap()).unwrap()
    }

    #[test]
    fn materializes_attested_static_manifest_and_complete_rules() {
        let (directory, guard) = materialized("https://example.test:8443");
        let expected_path = directory
            .path()
            .canonicalize()
            .unwrap()
            .join(CONTROL_DIRECTORY)
            .join(EXTENSION_DIRECTORY);
        assert_eq!(guard.extension_path(), expected_path);
        guard.verify().unwrap();

        let manifest = read_json(&guard, MANIFEST_FILE);
        assert_eq!(manifest["manifest_version"], 3);
        assert_eq!(manifest["minimum_chrome_version"], "120");
        assert_eq!(manifest["permissions"], json!(["declarativeNetRequest"]));
        assert!(manifest.get("host_permissions").is_none());
        assert_eq!(
            manifest["declarative_net_request"]["rule_resources"][0]["path"],
            RULES_FILE
        );
        assert_eq!(
            manifest["content_scripts"][0]["matches"],
            json!(["https://example.test/*"])
        );
        assert_eq!(
            manifest["content_scripts"][0]["include_globs"],
            json!(["https://example.test:8443/*"])
        );
        assert_eq!(manifest["content_scripts"][0]["run_at"], "document_start");
        assert_eq!(manifest["content_scripts"][0]["world"], "ISOLATED");
        assert_eq!(manifest["content_scripts"][0]["js"], json!([CONTENT_FILE]));

        let rules = read_json(&guard, RULES_FILE);
        assert_eq!(rules.as_array().unwrap().len(), 2);
        assert_eq!(rules[0]["action"]["type"], "allow");
        assert_eq!(rules[0]["priority"], 2);
        assert_eq!(
            rules[0]["condition"]["regexFilter"],
            "^https://example\\.test:8443([/?#]|$)"
        );
        assert_eq!(rules[0]["condition"]["isUrlFilterCaseSensitive"], false);
        assert_eq!(rules[1]["action"]["type"], "block");
        assert_eq!(rules[1]["priority"], 1);
        assert_eq!(
            rules[1]["condition"]["regexFilter"],
            "^(http|https|ws|wss)://"
        );
        assert_eq!(
            rules[0]["condition"]["resourceTypes"],
            json!(RESOURCE_TYPES)
        );
        assert_eq!(
            rules[1]["condition"]["resourceTypes"],
            json!(RESOURCE_TYPES)
        );

        assert_eq!(guard.marker_token().len(), 64);
        assert!(
            guard
                .marker_token()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_eq!(
            guard.marker_selector(),
            format!(
                "meta[name=\"{MARKER_NAME}\"][content=\"{}\"]",
                guard.marker_token()
            )
        );
        let content = fs::read_to_string(guard.extension_path().join(CONTENT_FILE)).unwrap();
        assert!(content.contains("https://example.test:8443"));
        assert!(content.contains(guard.marker_token()));
        assert!(content.contains("MutationObserver"));
    }

    #[test]
    fn default_ports_match_canonical_omission_or_explicit_spelling_only() {
        assert_eq!(
            allow_regex(&Origin::parse("http://example.test").unwrap()).unwrap(),
            "^http://example\\.test(:80)?([/?#]|$)"
        );
        assert_eq!(
            allow_regex(&Origin::parse("https://example.test:443").unwrap()).unwrap(),
            "^https://example\\.test(:443)?([/?#]|$)"
        );
        assert_eq!(
            allow_regex(&Origin::parse("https://example.test:444").unwrap()).unwrap(),
            "^https://example\\.test:444([/?#]|$)"
        );
        let default_target =
            GuardTarget::from_origin(&Origin::parse("https://example.test:443").unwrap()).unwrap();
        assert_eq!(default_target.canonical_origin, "https://example.test");
        assert_eq!(default_target.match_pattern, "https://example.test/*");
    }

    #[test]
    fn ipv6_hosts_are_bracketed_and_regex_escaped() {
        let origin = Origin::parse("http://[::1]:4173").unwrap();
        assert_eq!(
            allow_regex(&origin).unwrap(),
            "^http://\\[::1\\]:4173([/?#]|$)"
        );
        let target = GuardTarget::from_origin(&origin).unwrap();
        assert_eq!(target.canonical_origin, "http://[::1]:4173");
        assert_eq!(target.match_pattern, "http://[::1]/*");
    }

    #[test]
    fn regex_escapes_every_re2_metacharacter() {
        assert_eq!(
            regex_escape(r"\.^$*+?()[]{}|"),
            r"\\\.\^\$\*\+\?\(\)\[\]\{\}\|"
        );
    }

    #[test]
    fn rejects_non_http_zero_port_and_noncanonical_hosts() {
        for origin in [
            Origin {
                scheme: "ws".to_owned(),
                host: "example.test".to_owned(),
                effective_port: 80,
            },
            Origin {
                scheme: "https".to_owned(),
                host: "example.test".to_owned(),
                effective_port: 0,
            },
            Origin {
                scheme: "https".to_owned(),
                host: "EXAMPLE.test".to_owned(),
                effective_port: 443,
            },
            Origin {
                scheme: "https".to_owned(),
                host: "[example.test]".to_owned(),
                effective_port: 443,
            },
            Origin {
                scheme: "https".to_owned(),
                host: "user@example.test".to_owned(),
                effective_port: 443,
            },
        ] {
            assert!(allow_regex(&origin).is_err(), "{origin:?}");
        }
    }

    #[test]
    fn rematerialization_replaces_only_expected_regular_files() {
        let (directory, first) = materialized("http://127.0.0.1:4173");
        fs::write(first.extension_path().join(MANIFEST_FILE), b"corrupt").unwrap();

        let second = ExactOriginGuard::materialize(
            directory.path(),
            &Origin::parse("http://127.0.0.1:4173").unwrap(),
        )
        .unwrap();
        second.verify().unwrap();
    }

    #[test]
    fn rematerialization_rotates_the_attestation_token() {
        let (directory, first) = materialized("http://127.0.0.1:4173");
        let first_token = first.marker_token().to_owned();

        let second = ExactOriginGuard::materialize(
            directory.path(),
            &Origin::parse("http://127.0.0.1:4173").unwrap(),
        )
        .unwrap();
        assert_ne!(first_token, second.marker_token());
        second.verify().unwrap();
        assert!(first.verify().is_err());
    }

    #[test]
    fn verification_detects_modified_bytes() {
        let (_directory, guard) = materialized("http://127.0.0.1:4173");
        let path = guard.extension_path().join(RULES_FILE);
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = if bytes[0] == b'[' { b'{' } else { b'[' };
        fs::write(path, bytes).unwrap();

        assert!(matches!(guard.verify(), Err(GuardError::Integrity(_))));
    }

    #[test]
    fn verification_detects_modified_attestation_script() {
        let (_directory, guard) = materialized("http://127.0.0.1:4173");
        let path = guard.extension_path().join(CONTENT_FILE);
        let mut bytes = fs::read(&path).unwrap();
        let final_index = bytes.len() - 1;
        bytes[final_index] ^= 1;
        fs::write(path, bytes).unwrap();

        assert!(matches!(guard.verify(), Err(GuardError::Integrity(_))));
    }

    #[test]
    fn verification_rejects_extra_extension_entries() {
        let (_directory, guard) = materialized("http://127.0.0.1:4173");
        fs::write(guard.extension_path().join("background.js"), b"").unwrap();

        assert!(matches!(guard.verify(), Err(GuardError::Integrity(_))));
    }

    #[test]
    fn materialization_rejects_preexisting_unexpected_entries() {
        let directory = tempfile::tempdir().unwrap();
        let extension = directory
            .path()
            .join(CONTROL_DIRECTORY)
            .join(EXTENSION_DIRECTORY);
        fs::create_dir_all(&extension).unwrap();
        fs::write(extension.join("unexpected.js"), b"").unwrap();

        let result = ExactOriginGuard::materialize(
            directory.path(),
            &Origin::parse("http://127.0.0.1:4173").unwrap(),
        );
        assert!(matches!(result, Err(GuardError::UnsafePath(_))));
    }

    #[cfg(unix)]
    #[test]
    fn verification_rejects_symlinked_extension_files() {
        use std::os::unix::fs::symlink;

        let (directory, guard) = materialized("http://127.0.0.1:4173");
        let manifest = guard.extension_path().join(MANIFEST_FILE);
        fs::remove_file(&manifest).unwrap();
        let outside = directory.path().join("outside.json");
        fs::write(&outside, &guard.manifest.bytes).unwrap();
        symlink(outside, manifest).unwrap();

        assert!(matches!(guard.verify(), Err(GuardError::Integrity(_))));
    }

    #[cfg(unix)]
    #[test]
    fn materialization_rejects_symlinked_control_directory() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), directory.path().join(CONTROL_DIRECTORY)).unwrap();

        let result = ExactOriginGuard::materialize(
            directory.path(),
            &Origin::parse("http://127.0.0.1:4173").unwrap(),
        );
        assert!(matches!(
            result,
            Err(GuardError::Integrity(_) | GuardError::UnsafePath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn materialization_rejects_a_symlink_run_root() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let link = parent.path().join("run");
        symlink(directory.path(), &link).unwrap();

        let result =
            ExactOriginGuard::materialize(&link, &Origin::parse("http://127.0.0.1:4173").unwrap());
        assert!(matches!(result, Err(GuardError::UnsafePath(_))));
    }
}
