use reqwest::multipart::{Form, Part};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::fs;

pub(super) struct LarkPreparedMediaMessage {
    pub(super) msg_type: &'static str,
    pub(super) content: Value,
}

#[derive(Debug, Clone)]
pub(super) struct LarkResolvedMediaMarker {
    pub(super) kind: LarkOutgoingMediaKind,
    path: PathBuf,
    file_name: String,
}

#[derive(Debug, Clone)]
pub(super) struct LarkOutgoingMediaMarker {
    kind: LarkOutgoingMediaKind,
    target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LarkOutgoingMediaKind {
    Image,
    File { file_type: &'static str },
}

impl LarkOutgoingMediaKind {
    fn from_marker_kind(kind: &str) -> Option<Self> {
        match kind.trim().to_ascii_uppercase().as_str() {
            "IMAGE" | "PHOTO" => Some(Self::Image),
            "DOCUMENT" | "FILE" => Some(Self::File {
                file_type: "stream",
            }),
            "VIDEO" => Some(Self::File { file_type: "mp4" }),
            "AUDIO" | "VOICE" => Some(Self::File { file_type: "opus" }),
            _ => None,
        }
    }
}

pub(super) fn lark_outgoing_media_from_marker(
    kind: String,
    target: String,
) -> Option<LarkOutgoingMediaMarker> {
    Some(LarkOutgoingMediaMarker {
        kind: LarkOutgoingMediaKind::from_marker_kind(&kind)?,
        target,
    })
}

pub(super) fn resolve_lark_media_marker(
    marker: &LarkOutgoingMediaMarker,
    workspace_dir: Option<&Path>,
) -> anyhow::Result<LarkResolvedMediaMarker> {
    let path = validate_lark_marker_target(&marker.target, workspace_dir)?;
    let metadata = std::fs::metadata(&path).map_err(|err| {
        anyhow::Error::msg(format!(
            "read Lark/Feishu marker target metadata failed: {err}"
        ))
    })?;
    if !metadata.is_file() {
        anyhow::bail!("read Lark/Feishu marker target is not a file");
    }

    if metadata.len() == 0 {
        anyhow::bail!("read Lark/Feishu marker target is empty");
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string();

    Ok(LarkResolvedMediaMarker {
        kind: marker.kind,
        path,
        file_name,
    })
}

fn validate_lark_marker_target(
    target: &str,
    workspace_dir: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Lark/Feishu marker target is empty");
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
        || lower.contains("://")
    {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"reason": "disallowed_scheme"})),
            "lark: marker target uses disallowed scheme"
        );
        anyhow::bail!("Lark/Feishu marker target uses a disallowed scheme");
    }

    let workspace = workspace_dir.ok_or_else(|| {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"reason": "no_workspace_dir"})),
            "lark: marker target has no workspace_dir"
        );
        anyhow::Error::msg("Lark/Feishu channel was started without a workspace_dir")
    })?;

    let workspace = std::fs::canonicalize(workspace).map_err(|err| {
        anyhow::Error::msg(format!(
            "canonicalize Lark/Feishu workspace_dir failed: {err}"
        ))
    })?;
    let candidate = Path::new(trimmed);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace.join(candidate)
    };

    let candidate = std::fs::canonicalize(&candidate).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            shadow_log::record!(
                WARN,
                shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                    .with_attrs(serde_json::json!({"reason": "not found"})),
                "lark: marker target not found on disk"
            );
            anyhow::Error::msg("Lark/Feishu marker target not found on disk")
        } else {
            anyhow::Error::msg(format!(
                "canonicalize Lark/Feishu marker target failed: {err}"
            ))
        }
    })?;

    if !candidate.starts_with(&workspace) {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                .with_attrs(serde_json::json!({"reason": "outside_workspace"})),
            "lark: marker target escapes workspaces"
        );
        anyhow::bail!("Lark/Feishu marker target resolves outside workspace_dir");
    }

    Ok(candidate)
}

pub(super) async fn build_lark_image_upload_form(
    marker: &LarkResolvedMediaMarker,
) -> anyhow::Result<Form> {
    let bytes = fs::read(&marker.path).await.map_err(|err| {
        anyhow::Error::msg(format!(
            "read Lark/Feishu image marker target failed: {err}"
        ))
    })?;
    Ok(Form::new().text("image_type", "message").part(
        "image",
        Part::bytes(bytes).file_name(marker.file_name.clone()),
    ))
}

pub(super) async fn build_lark_file_upload_form(
    marker: &LarkResolvedMediaMarker,
    file_type: &'static str,
) -> anyhow::Result<Form> {
    let bytes = fs::read(&marker.path).await.map_err(|err| {
        anyhow::Error::msg(format!("read Lark/Feishu file marker target failed: {err}"))
    })?;
    Ok(Form::new()
        .text("file_type", file_type)
        .text("file_name", marker.file_name.clone())
        .part(
            "file",
            Part::bytes(bytes).file_name(marker.file_name.clone()),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ---- LarkOutgoingMediaKind::from_marker_kind (via lark_outgoing_media_from_marker) ----

    #[test]
    fn marker_kind_image() {
        let m = lark_outgoing_media_from_marker("IMAGE".into(), "x.png".into());
        assert!(m.is_some());
        assert_eq!(m.unwrap().kind, LarkOutgoingMediaKind::Image);
    }

    #[test]
    fn marker_kind_photo() {
        let m = lark_outgoing_media_from_marker("photo".into(), "x.jpg".into());
        assert_eq!(m.unwrap().kind, LarkOutgoingMediaKind::Image);
    }

    #[test]
    fn marker_kind_document() {
        let m = lark_outgoing_media_from_marker("DOCUMENT".into(), "x.pdf".into());
        assert_eq!(
            m.unwrap().kind,
            LarkOutgoingMediaKind::File {
                file_type: "stream"
            }
        );
    }

    #[test]
    fn marker_kind_video() {
        let m = lark_outgoing_media_from_marker("VIDEO".into(), "x.mp4".into());
        assert_eq!(
            m.unwrap().kind,
            LarkOutgoingMediaKind::File { file_type: "mp4" }
        );
    }

    #[test]
    fn marker_kind_audio() {
        let m = lark_outgoing_media_from_marker("AUDIO".into(), "x.opus".into());
        assert_eq!(
            m.unwrap().kind,
            LarkOutgoingMediaKind::File { file_type: "opus" }
        );
    }

    #[test]
    fn marker_kind_voice() {
        let m = lark_outgoing_media_from_marker("voice".into(), "x.opus".into());
        assert_eq!(
            m.unwrap().kind,
            LarkOutgoingMediaKind::File { file_type: "opus" }
        );
    }

    #[test]
    fn marker_kind_unknown_returns_none() {
        assert!(lark_outgoing_media_from_marker("GIF".into(), "x.gif".into()).is_none());
    }

    #[test]
    fn marker_kind_whitespace_trimmed() {
        let m = lark_outgoing_media_from_marker("  image  ".into(), "x.png".into());
        assert!(m.is_some());
    }

    // ---- resolve_lark_media_marker / validate_lark_marker_target ----

    fn make_workspace_with_file(name: &str, content: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join(name);
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(content).unwrap();
        drop(f);
        dir
    }

    #[test]
    fn resolve_valid_file() {
        let dir = make_workspace_with_file("test.txt", b"hello");
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "test.txt".into(),
        };
        let resolved = resolve_lark_media_marker(&marker, Some(dir.path())).unwrap();
        assert_eq!(resolved.kind, LarkOutgoingMediaKind::Image);
        assert_eq!(resolved.file_name, "test.txt");
    }

    #[test]
    fn resolve_empty_target_fails() {
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "   ".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(Path::new("/tmp")));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_url_scheme_fails() {
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "https://evil.com/x.png".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(Path::new("/tmp")));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_data_scheme_fails() {
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "data:image/png;base64,abc".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(Path::new("/tmp")));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_without_workspace_fails() {
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "x.png".into(),
        };
        let result = resolve_lark_media_marker(&marker, None);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_nonexistent_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "nope.png".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(dir.path()));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_directory_not_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        // Create a sub-directory inside workspace
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "subdir".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(dir.path()));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_empty_file_fails() {
        let dir = make_workspace_with_file("empty.txt", b"");
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "empty.txt".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(dir.path()));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_path_traversal_fails() {
        let dir = make_workspace_with_file("test.txt", b"hello");
        // Try to escape workspace via ../
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "../../../etc/passwd".into(),
        };
        let result = resolve_lark_media_marker(&marker, Some(dir.path()));
        // Either NotFound or outside-workspace — both are errors
        assert!(result.is_err());
    }

    // ---- build_lark_image_upload_form / build_lark_file_upload_form ----

    #[tokio::test]
    async fn build_image_form_reads_file() {
        let dir = make_workspace_with_file("img.png", b"PNGDATA");
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            target: "img.png".into(),
        };
        let resolved = resolve_lark_media_marker(&marker, Some(dir.path())).unwrap();
        let form = build_lark_image_upload_form(&resolved).await;
        assert!(form.is_ok());
    }

    #[tokio::test]
    async fn build_file_form_reads_file() {
        let dir = make_workspace_with_file("doc.pdf", b"PDFDATA");
        let marker = LarkOutgoingMediaMarker {
            kind: LarkOutgoingMediaKind::File {
                file_type: "stream",
            },
            target: "doc.pdf".into(),
        };
        let resolved = resolve_lark_media_marker(&marker, Some(dir.path())).unwrap();
        let form = build_lark_file_upload_form(&resolved, "stream").await;
        assert!(form.is_ok());
    }

    #[tokio::test]
    async fn build_image_form_missing_file_fails() {
        // Construct a resolved marker pointing to a non-existent path
        let resolved = LarkResolvedMediaMarker {
            kind: LarkOutgoingMediaKind::Image,
            path: PathBuf::from("/nonexistent/file.png"),
            file_name: "file.png".into(),
        };
        let result = build_lark_image_upload_form(&resolved).await;
        assert!(result.is_err());
    }
}
