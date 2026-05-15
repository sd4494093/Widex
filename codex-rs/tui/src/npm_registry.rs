use serde::Deserialize;
use std::collections::HashMap;

#[cfg(not(debug_assertions))]
pub(crate) const PACKAGE_URL: &str = "https://registry.npmjs.org/@wellau%2fwidex";

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct NpmPackageInfo {
    #[serde(rename = "dist-tags")]
    dist_tags: HashMap<String, String>,
    versions: HashMap<String, NpmPackageVersionInfo>,
}

#[derive(Deserialize, Debug, Clone)]
struct NpmPackageVersionInfo {
    dist: Option<NpmPackageDist>,
}

#[derive(Deserialize, Debug, Clone)]
struct NpmPackageDist {
    tarball: Option<String>,
    integrity: Option<String>,
}

pub(crate) fn latest_ready_version(package_info: &NpmPackageInfo) -> anyhow::Result<String> {
    let latest = package_info
        .dist_tags
        .get("latest")
        .ok_or_else(|| anyhow::anyhow!("npm package is missing latest dist-tag"))?;
    version_info_with_dist(package_info, latest)?;
    Ok(latest.to_string())
}

fn version_info_with_dist<'a>(
    package_info: &'a NpmPackageInfo,
    version: &str,
) -> anyhow::Result<&'a NpmPackageVersionInfo> {
    let info = package_info
        .versions
        .get(version)
        .ok_or_else(|| anyhow::anyhow!("npm package version {version} is missing"))?;
    let Some(dist) = info.dist.as_ref() else {
        anyhow::bail!("npm package version {version} is missing dist metadata");
    };
    let has_tarball = dist
        .tarball
        .as_deref()
        .is_some_and(|tarball| !tarball.is_empty());
    if !has_tarball {
        anyhow::bail!("npm package version {version} is missing dist.tarball");
    }
    let has_integrity = dist
        .integrity
        .as_ref()
        .is_some_and(|integrity| !integrity.is_empty());
    if !has_integrity {
        anyhow::bail!("npm package version {version} is missing dist.integrity");
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn version_json(version: &str) -> serde_json::Value {
        serde_json::json!({
            "dist": {
                "integrity": format!("sha512-{version}"),
                "tarball": format!("https://registry.npmjs.org/@wellau/widex/-/widex-{version}.tgz"),
            }
        })
    }

    fn package_info(version: &str, npm_latest: &str) -> NpmPackageInfo {
        let mut versions = serde_json::Map::new();
        versions.insert(version.to_string(), version_json(version));

        serde_json::from_value(serde_json::json!({
            "dist-tags": { "latest": npm_latest },
            "versions": serde_json::Value::Object(versions),
        }))
        .expect("valid npm package metadata")
    }

    #[test]
    fn latest_ready_version_returns_latest_dist_tag_with_root_dist() {
        let latest = "1.2.3";
        let package_info = package_info(latest, latest);

        assert_eq!(
            latest_ready_version(&package_info).expect("npm package is ready"),
            latest
        );
    }

    #[test]
    fn latest_ready_version_rejects_missing_latest_dist_tag() {
        let package_info: NpmPackageInfo = serde_json::from_value(serde_json::json!({
            "dist-tags": {},
            "versions": { "1.2.3": version_json("1.2.3") },
        }))
        .expect("valid npm package metadata");

        let err = latest_ready_version(&package_info).expect_err("latest dist-tag is required");
        assert!(
            err.to_string().contains("missing latest dist-tag"),
            "error should name missing latest dist-tag: {err}"
        );
    }

    #[test]
    fn latest_ready_version_rejects_missing_root_dist() {
        let package_info: NpmPackageInfo = serde_json::from_value(serde_json::json!({
            "dist-tags": { "latest": "1.2.3" },
            "versions": { "1.2.3": {} },
        }))
        .expect("valid npm package metadata");

        let err =
            latest_ready_version(&package_info).expect_err("root package must have dist metadata");
        assert!(
            err.to_string().contains("missing dist metadata"),
            "error should name missing dist metadata: {err}"
        );
    }
}
