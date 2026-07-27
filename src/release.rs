use std::collections::HashSet;

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BUNDLE_MANIFEST_NAME: &str = "crawlson-bundle.json";
pub const UPDATE_MANIFEST_NAME: &str = "crawlson-update.json";
pub const UPDATE_SIGNATURE_NAME: &str = "crawlson-update.json.minisig";
pub const RELEASE_INVENTORY_NAME: &str = "crawlson-release.json";
pub const RELEASE_SIGNATURE_NAME: &str = "crawlson-release.json.minisig";
pub const MAX_RELEASE_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_BUNDLE_FILES: usize = 32;
pub const SUPPORTED_RELEASE_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleManifestV1 {
    pub schema_version: u8,
    pub version: Version,
    pub target: String,
    pub files: Vec<BundleFileV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleFileV1 {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateManifestV1 {
    pub schema_version: u8,
    pub version: Version,
    pub artifacts: Vec<UpdateArtifactV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateArtifactV1 {
    pub target: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseInventoryV1 {
    pub schema_version: u8,
    pub version: Version,
    pub bundles: Vec<ReleaseBundleV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseBundleV1 {
    pub target: String,
    pub format: BundleFormat,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub update_name: String,
    pub update_size: u64,
    pub update_sha256: String,
    pub files: Vec<BundleFileV1>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundleFormat {
    TarGz,
    Zip,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReleaseContractError {
    #[error("unsupported schema version {0}")]
    Schema(u8),
    #[error("release version must be stable SemVer without build metadata")]
    UnstableVersion,
    #[error("unsupported release target {0}")]
    UnsupportedTarget(String),
    #[error("release path is not a safe normalized relative path: {0}")]
    UnsafePath(String),
    #[error("release file {0} has an invalid size")]
    InvalidSize(String),
    #[error("release file {0} has a malformed SHA-256 digest")]
    InvalidDigest(String),
    #[error("release entry is duplicated: {0}")]
    Duplicate(String),
    #[error("release entries are not in canonical order")]
    NonCanonicalOrder,
    #[error("release artifact name does not match its target and version: {0}")]
    InvalidArtifactName(String),
    #[error("release contract is empty")]
    Empty,
}

impl BundleManifestV1 {
    pub fn validate(&self) -> Result<(), ReleaseContractError> {
        validate_header(self.schema_version, &self.version)?;
        validate_target(&self.target)?;
        validate_files(&self.files)
    }
}

impl UpdateManifestV1 {
    pub fn validate(&self) -> Result<(), ReleaseContractError> {
        validate_header(self.schema_version, &self.version)?;
        if self.artifacts.is_empty() {
            return Err(ReleaseContractError::Empty);
        }
        if self.artifacts.len() > SUPPORTED_RELEASE_TARGETS.len() {
            return Err(ReleaseContractError::Duplicate(
                "too many update targets".to_owned(),
            ));
        }
        let mut targets = HashSet::new();
        let mut names = HashSet::new();
        let mut previous: Option<&str> = None;
        for artifact in &self.artifacts {
            validate_target(&artifact.target)?;
            validate_size_digest(&artifact.name, artifact.size, &artifact.sha256)?;
            if previous.is_some_and(|value| value >= artifact.target.as_str()) {
                return Err(ReleaseContractError::NonCanonicalOrder);
            }
            previous = Some(&artifact.target);
            if !targets.insert(&artifact.target) {
                return Err(ReleaseContractError::Duplicate(artifact.target.clone()));
            }
            if !names.insert(&artifact.name) {
                return Err(ReleaseContractError::Duplicate(artifact.name.clone()));
            }
            let expected = update_artifact_name(&self.version, &artifact.target);
            if artifact.name != expected {
                return Err(ReleaseContractError::InvalidArtifactName(
                    artifact.name.clone(),
                ));
            }
        }
        Ok(())
    }
}

impl ReleaseInventoryV1 {
    pub fn validate(&self) -> Result<(), ReleaseContractError> {
        validate_header(self.schema_version, &self.version)?;
        if self.bundles.is_empty() {
            return Err(ReleaseContractError::Empty);
        }
        if self.bundles.len() > SUPPORTED_RELEASE_TARGETS.len() {
            return Err(ReleaseContractError::Duplicate(
                "too many release targets".to_owned(),
            ));
        }
        let mut targets = HashSet::new();
        let mut names = HashSet::new();
        let mut previous: Option<&str> = None;
        for bundle in &self.bundles {
            validate_target(&bundle.target)?;
            if previous.is_some_and(|value| value >= bundle.target.as_str()) {
                return Err(ReleaseContractError::NonCanonicalOrder);
            }
            previous = Some(&bundle.target);
            if !targets.insert(&bundle.target) {
                return Err(ReleaseContractError::Duplicate(bundle.target.clone()));
            }
            for name in [&bundle.name, &bundle.update_name] {
                if !names.insert(name) {
                    return Err(ReleaseContractError::Duplicate(name.clone()));
                }
            }
            validate_size_digest(&bundle.name, bundle.size, &bundle.sha256)?;
            validate_size_digest(
                &bundle.update_name,
                bundle.update_size,
                &bundle.update_sha256,
            )?;
            if bundle.name != bundle_artifact_name(&self.version, &bundle.target, bundle.format)
                || bundle.update_name != update_artifact_name(&self.version, &bundle.target)
            {
                return Err(ReleaseContractError::InvalidArtifactName(
                    bundle.name.clone(),
                ));
            }
            let expected_format = bundle_format(&bundle.target)?;
            if bundle.format != expected_format {
                return Err(ReleaseContractError::InvalidArtifactName(
                    bundle.name.clone(),
                ));
            }
            validate_files(&bundle.files)?;
            let crawlson_path = format!(
                "bin/crawlson{}",
                executable_suffix(&bundle.target).unwrap_or_default()
            );
            let crawlson = bundle
                .files
                .iter()
                .find(|file| file.path == crawlson_path)
                .ok_or(ReleaseContractError::Empty)?;
            if crawlson.size != bundle.update_size || crawlson.sha256 != bundle.update_sha256 {
                return Err(ReleaseContractError::InvalidDigest(bundle.name.clone()));
            }
        }
        Ok(())
    }
}

pub fn validate_files(files: &[BundleFileV1]) -> Result<(), ReleaseContractError> {
    if files.is_empty() {
        return Err(ReleaseContractError::Empty);
    }
    if files.len() > MAX_BUNDLE_FILES {
        return Err(ReleaseContractError::Duplicate(
            "too many bundle files".to_owned(),
        ));
    }
    let mut paths = HashSet::new();
    let mut previous: Option<&str> = None;
    for file in files {
        validate_relative_path(&file.path)?;
        validate_size_digest(&file.path, file.size, &file.sha256)?;
        if previous.is_some_and(|value| value >= file.path.as_str()) {
            return Err(ReleaseContractError::NonCanonicalOrder);
        }
        previous = Some(&file.path);
        if !paths.insert(&file.path) {
            return Err(ReleaseContractError::Duplicate(file.path.clone()));
        }
    }
    Ok(())
}

pub fn validate_target(target: &str) -> Result<(), ReleaseContractError> {
    if SUPPORTED_RELEASE_TARGETS.contains(&target) {
        Ok(())
    } else {
        Err(ReleaseContractError::UnsupportedTarget(target.to_owned()))
    }
}

pub fn update_artifact_name(version: &Version, target: &str) -> String {
    format!(
        "crawlson-update-v{version}-{target}{}",
        executable_suffix(target).unwrap_or_default()
    )
}

pub fn bundle_artifact_name(version: &Version, target: &str, format: BundleFormat) -> String {
    let suffix = match format {
        BundleFormat::TarGz => "tar.gz",
        BundleFormat::Zip => "zip",
    };
    format!("crawlson-v{version}-{target}.{suffix}")
}

pub fn bundle_root_name(version: &Version, target: &str) -> String {
    format!("crawlson-v{version}-{target}")
}

pub fn executable_suffix(target: &str) -> Result<&'static str, ReleaseContractError> {
    validate_target(target)?;
    Ok(if target.contains("windows") {
        ".exe"
    } else {
        ""
    })
}

pub fn bundle_format(target: &str) -> Result<BundleFormat, ReleaseContractError> {
    validate_target(target)?;
    Ok(if target.contains("windows") {
        BundleFormat::Zip
    } else {
        BundleFormat::TarGz
    })
}

fn validate_header(schema: u8, version: &Version) -> Result<(), ReleaseContractError> {
    if schema != 1 {
        return Err(ReleaseContractError::Schema(schema));
    }
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(ReleaseContractError::UnstableVersion);
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), ReleaseContractError> {
    let valid = !path.is_empty()
        && !path.contains('\\')
        && path.split('/').all(|component| {
            component
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        });
    if !valid {
        return Err(ReleaseContractError::UnsafePath(path.to_owned()));
    }
    Ok(())
}

fn validate_size_digest(name: &str, size: u64, digest: &str) -> Result<(), ReleaseContractError> {
    if size == 0 || size > MAX_RELEASE_FILE_BYTES {
        return Err(ReleaseContractError::InvalidSize(name.to_owned()));
    }
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ReleaseContractError::InvalidDigest(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_names_are_target_specific() {
        let version = Version::parse("0.5.0").unwrap();
        assert_eq!(
            update_artifact_name(&version, "x86_64-pc-windows-msvc"),
            "crawlson-update-v0.5.0-x86_64-pc-windows-msvc.exe"
        );
        assert_eq!(
            bundle_artifact_name(&version, "x86_64-unknown-linux-gnu", BundleFormat::TarGz),
            "crawlson-v0.5.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn bundle_contract_rejects_unsafe_or_noncanonical_files() {
        let base = BundleFileV1 {
            path: "bin/crawlson".to_owned(),
            size: 10,
            sha256: "0".repeat(64),
        };
        assert!(validate_files(std::slice::from_ref(&base)).is_ok());
        for path in [
            "../crawlson",
            "/bin/crawlson",
            "bin\\crawlson",
            "bin//crawlson",
            "bin/crawlson/",
            "bin/:crawlson",
            "bin/.hidden",
            ".",
        ] {
            let mut invalid = base.clone();
            invalid.path = path.to_owned();
            assert!(validate_files(&[invalid]).is_err(), "{path}");
        }
        let mut uppercase = base;
        uppercase.sha256 = "A".repeat(64);
        assert!(validate_files(&[uppercase]).is_err());
    }
}
