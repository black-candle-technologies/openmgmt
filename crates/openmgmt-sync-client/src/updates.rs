//! Shared, testable release.json contract. No semver ordering: only build codes.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseFile {
    pub platform: String,
    pub arch: String,
    pub name: String,
    pub sha256: String,
    pub version_code: u64,
}
#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub files: Vec<ReleaseFile>,
}
impl ReleaseManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("Invalid release.json: {e}"))?;
        if manifest.schema != 1 {
            return Err("Unsupported release.json schema".into());
        }
        for file in &manifest.files {
            if !["macos", "windows", "linux"].contains(&file.platform.as_str())
                || file.name.is_empty()
                || file.name.contains(['/', '\\'])
                || file.name.starts_with('.')
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid release file metadata".into());
            }
        }
        Ok(manifest)
    }
    pub fn newer_file(&self, platform: &str, arch: &str, current: u64) -> Option<&ReleaseFile> {
        self.files
            .iter()
            .filter(|file| {
                file.platform == platform
                    && (file.arch == arch || (platform == "macos" && file.arch == "universal"))
                    && file.version_code > current
            })
            .max_by_key(|file| file.version_code)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(code: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"schema": 1, "files": [{"platform":"macos", "arch":"universal", "name":"OpenMGMT.dmg", "sha256":"a".repeat(64), "version_code":code}]})).unwrap()
    }
    #[test]
    fn version_comparison_is_strict_and_platform_specific() {
        let m = ReleaseManifest::parse(&manifest(102)).unwrap();
        assert!(m.newer_file("macos", "aarch64", 101).is_some());
        assert!(m.newer_file("macos", "x86_64", 102).is_none());
        assert!(m.newer_file("macos", "aarch64", 103).is_none());
        assert!(m.newer_file("windows", "x86_64", 0).is_none());
    }
    #[test]
    fn invalid_manifests_fail_closed() {
        for bytes in [
            b"{}".as_slice(),
            b"{\"schema\":2,\"files\":[]}",
            b"not json",
        ] {
            assert!(ReleaseManifest::parse(bytes).is_err());
        }
        let mut value: serde_json::Value = serde_json::from_slice(&manifest(1)).unwrap();
        value["files"][0]["name"] = "../installer.exe".into();
        assert!(ReleaseManifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value["files"][0]["name"] = "installer.exe".into();
        value["files"][0]["sha256"] = "bad".into();
        assert!(ReleaseManifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
