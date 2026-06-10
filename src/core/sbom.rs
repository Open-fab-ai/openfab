//! SBOM — a minimal SPDX-style software bill of materials for the generated app.
//!
//! Production swap (PRD §5): Syft (SPDX/CycloneDX). v0.1 ships a small, dependency-free
//! SPDX 2.3 document listing the generated files with their checksums, so the product
//! carries a portable inventory committed in-repo. Every file the fab emits is
//! accounted for — nothing ships unlisted.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::core::timeutil;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxFile {
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "SPDXID")]
    pub spdxid: String,
    pub checksums: Vec<Checksum>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: String,
    #[serde(rename = "checksumValue")]
    pub checksum_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sbom {
    #[serde(rename = "spdxVersion")]
    pub spdx_version: String,
    #[serde(rename = "dataLicense")]
    pub data_license: String,
    #[serde(rename = "SPDXID")]
    pub spdxid: String,
    pub name: String,
    pub created: String,
    pub creators: Vec<String>,
    pub files: Vec<SpdxFile>,
}

impl Sbom {
    /// Build an SBOM from (path, sha256) pairs.
    pub fn build(name: &str, files: &[(String, String)]) -> Sbom {
        Sbom {
            spdx_version: "SPDX-2.3".to_string(),
            data_license: "CC0-1.0".to_string(),
            spdxid: "SPDXRef-DOCUMENT".to_string(),
            name: name.to_string(),
            created: timeutil::iso_now(),
            creators: vec!["Tool: openfab-0.1".to_string()],
            files: files
                .iter()
                .enumerate()
                .map(|(i, (path, sha))| SpdxFile {
                    file_name: path.clone(),
                    spdxid: format!("SPDXRef-File-{i}"),
                    checksums: vec![Checksum {
                        algorithm: "SHA256".to_string(),
                        checksum_value: sha.clone(),
                    }],
                })
                .collect(),
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize SBOM")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sbom_with_files() {
        let files = vec![
            ("app/main.py".to_string(), "aaaa".to_string()),
            ("app/test.py".to_string(), "bbbb".to_string()),
        ];
        let sbom = Sbom::build("demo-app", &files);
        assert_eq!(sbom.files.len(), 2);
        assert_eq!(sbom.spdx_version, "SPDX-2.3");
        assert!(sbom.to_json().unwrap().contains("SHA256"));
    }
}
