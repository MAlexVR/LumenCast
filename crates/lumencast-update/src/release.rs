//! Which release is newest and which of its files installs on this system.
//! Pure logic over the GitHub Releases JSON, tested without a network.

use crate::version::Version;
use serde::Serialize;
use serde_json::Value;

/// A file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    /// The API address of the file: it also works for a private repository,
    /// with a token, and answers with a redirect to the file.
    pub url: String,
    pub size: u64,
}

/// How the running program was installed, which decides what installs the update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    Rpm,
    Deb,
    AppImage,
    /// A binary somewhere else (a build under the home folder): updated by hand.
    Unmanaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub page: String,
    pub published: String,
    pub assets: Vec<Asset>,
}

/// Longest release notes kept.
const MAX_NOTES: usize = 20 * 1024;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The releases in a `GET /releases` answer that can be offered: not drafts,
/// with a tag that is a version, and not prereleases unless asked for.
pub fn parse_releases(json: &str, include_prereleases: bool) -> Vec<Release> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| !item.get("draft").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|item| {
            let tag = text(item, "tag_name");
            let version = Version::parse(&tag)?;
            let flagged = item
                .get("prerelease")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if (flagged || version.is_prerelease()) && !include_prereleases {
                return None;
            }
            let assets = item
                .get("assets")
                .and_then(Value::as_array)
                .map(|assets| {
                    assets
                        .iter()
                        .map(|asset| Asset {
                            name: text(asset, "name"),
                            url: text(asset, "url"),
                            size: asset.get("size").and_then(Value::as_u64).unwrap_or(0),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut notes = text(item, "body");
            if notes.len() > MAX_NOTES {
                let mut end = MAX_NOTES;
                while !notes.is_char_boundary(end) {
                    end -= 1;
                }
                notes.truncate(end);
            }
            Some(Release {
                version,
                tag,
                notes,
                page: text(item, "html_url"),
                published: text(item, "published_at"),
                assets,
            })
        })
        .collect()
}

/// The newest of `releases`, when it is newer than `current`.
pub fn newest_after<'a>(releases: &'a [Release], current: &Version) -> Option<&'a Release> {
    releases
        .iter()
        .filter(|release| release.version > *current)
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// The installer of `release` for `kind`, by the names the bundler gives them
/// (`LumenCast-1.0.0-1.x86_64.rpm`, `LumenCast_1.0.0_amd64.deb`,
/// `LumenCast_1.0.0_amd64.AppImage`); the version in the name must be the release's.
pub fn installer_for(release: &Release, kind: InstallKind) -> Option<&Asset> {
    let version = release.version.to_string();
    release.assets.iter().find(|asset| {
        let name = asset.name.as_str();
        name.contains(&version)
            && match kind {
                InstallKind::Rpm => name.ends_with(".rpm") && name.contains("x86_64"),
                InstallKind::Deb => name.ends_with(".deb") && name.contains("amd64"),
                InstallKind::AppImage => name.ends_with(".AppImage") && name.contains("amd64"),
                InstallKind::Unmanaged => false,
            }
    })
}

/// A companion file of `installer` or of the release: its signature, the sums.
pub fn companion<'a>(release: &'a Release, name: &str) -> Option<&'a Asset> {
    release.assets.iter().find(|asset| asset.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"[
      {"tag_name": "v1.1.0", "draft": true, "prerelease": false, "assets": []},
      {"tag_name": "v1.0.1", "draft": false, "prerelease": false, "body": "Fixes",
       "html_url": "https://github.com/x/y/releases/tag/v1.0.1", "published_at": "2026-11-05T10:00:00Z",
       "assets": [
         {"name": "LumenCast-1.0.1-1.x86_64.rpm", "url": "https://api.github.com/a/1", "size": 10},
         {"name": "LumenCast_1.0.1_amd64.deb", "url": "https://api.github.com/a/2", "size": 11},
         {"name": "LumenCast_1.0.1_amd64.AppImage", "url": "https://api.github.com/a/3", "size": 12},
         {"name": "LumenCast_1.0.1_amd64.deb.sig", "url": "https://api.github.com/a/4", "size": 1},
         {"name": "SHA256SUMS.txt", "url": "https://api.github.com/a/5", "size": 1}]},
      {"tag_name": "v1.0.0", "draft": false, "prerelease": false, "assets": []},
      {"tag_name": "v1.2.0-rc.1", "draft": false, "prerelease": true, "assets": []},
      {"tag_name": "nightly", "draft": false, "prerelease": false, "assets": []}
    ]"#;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn drafts_tags_that_are_not_versions_and_prereleases_are_left_out_by_default() {
        let stable = parse_releases(JSON, false);
        let tags: Vec<&str> = stable.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, ["v1.0.1", "v1.0.0"]);
        let all = parse_releases(JSON, true);
        assert_eq!(all.len(), 3);
        assert!(parse_releases("not json", true).is_empty());
        assert!(parse_releases("{}", true).is_empty());
    }

    #[test]
    fn only_a_newer_release_is_offered_and_the_newest_wins() {
        let stable = parse_releases(JSON, false);
        assert_eq!(newest_after(&stable, &v("1.0.0")).unwrap().tag, "v1.0.1");
        assert!(newest_after(&stable, &v("1.0.1")).is_none());
        assert!(newest_after(&stable, &v("2.0.0")).is_none());
        let all = parse_releases(JSON, true);
        assert_eq!(newest_after(&all, &v("1.0.1")).unwrap().tag, "v1.2.0-rc.1");
    }

    #[test]
    fn each_install_kind_gets_its_own_installer_and_a_hand_made_one_gets_none() {
        let release = &parse_releases(JSON, false)[0];
        let name = |kind| installer_for(release, kind).map(|a| a.name.clone());
        assert_eq!(
            name(InstallKind::Rpm).as_deref(),
            Some("LumenCast-1.0.1-1.x86_64.rpm")
        );
        assert_eq!(
            name(InstallKind::Deb).as_deref(),
            Some("LumenCast_1.0.1_amd64.deb")
        );
        assert_eq!(
            name(InstallKind::AppImage).as_deref(),
            Some("LumenCast_1.0.1_amd64.AppImage")
        );
        assert_eq!(name(InstallKind::Unmanaged), None);
        assert!(companion(release, "SHA256SUMS.txt").is_some());
        assert!(companion(release, "LumenCast_1.0.1_amd64.deb.sig").is_some());
        assert!(companion(release, "nothing").is_none());
    }

    #[test]
    fn an_installer_of_another_version_is_never_picked() {
        let mut release = parse_releases(JSON, false).remove(0);
        release.assets[0].name = "LumenCast-0.9.0-1.x86_64.rpm".into();
        assert!(installer_for(&release, InstallKind::Rpm).is_none());
    }

    #[test]
    fn long_notes_are_cut_on_a_character_boundary() {
        let body = "é".repeat(MAX_NOTES);
        let json = format!(r#"[{{"tag_name":"v2.0.0","body":"{body}","assets":[]}}]"#);
        let release = &parse_releases(&json, false)[0];
        assert!(release.notes.len() <= MAX_NOTES);
        assert!(release.notes.chars().all(|c| c == 'é'));
    }
}
