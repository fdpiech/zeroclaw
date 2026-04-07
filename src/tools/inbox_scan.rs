use super::traits::{Tool, ToolResult};
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Reserved stage subdirectory names — never treated as source types.
const RESERVED_DIRS: &[&str] = &["processing", "processed", "failed"];

/// Scan inbox subdirectories for unprocessed files.
///
/// Returns a JSON array of files sitting in root source-type directories.
/// Files already moved into `processing/`, `processed/`, or `failed/` are excluded.
/// Sidecar and error files (`.sidecar.json`, `.error.json`) are also excluded.
pub struct InboxScanTool {
    security: Arc<SecurityPolicy>,
}

impl InboxScanTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for InboxScanTool {
    fn name(&self) -> &str {
        "inbox_scan"
    }

    fn description(&self) -> &str {
        "Scan inbox directories for unprocessed files. Returns a JSON array of pending \
         files, each with path (workspace-relative), filename, source_type, size_bytes, \
         and modified_unix timestamp. Files already in processing/, processed/, or failed/ \
         are excluded. Use file_stage with action='start' to claim a file before reading it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "inbox_root": {
                    "type": "string",
                    "description": "Root inbox directory relative to workspace (default: 'inbox')"
                },
                "source_types": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Source-type subdirectories to scan, e.g. ['emails', 'transcripts']. Defaults to all non-reserved subdirs."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of files to return across all source types (default: 5)"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if self.security.is_rate_limited() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: too many actions in the last hour".into()),
            });
        }

        let inbox_root = args
            .get("inbox_root")
            .and_then(|v| v.as_str())
            .unwrap_or("inbox");

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(5);

        // Validate inbox_root is within workspace before consuming rate budget
        if !self.security.is_path_allowed(inbox_root) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("inbox_root path not allowed: {inbox_root}")),
            });
        }

        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: action budget exhausted".into()),
            });
        }

        let inbox_path = self.security.workspace_dir.join(inbox_root);

        // Determine which source_type dirs to scan
        let requested_types: Option<Vec<String>> = args
            .get("source_types")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            });

        // Enumerate source-type subdirs under inbox_root
        let mut source_dirs: Vec<String> = Vec::new();
        let mut read_dir = match tokio::fs::read_dir(&inbox_path).await {
            Ok(rd) => rd,
            Err(_) => {
                // Inbox root doesn't exist yet — return empty list, not an error
                return Ok(ToolResult {
                    success: true,
                    output: "[]".into(),
                    error: None,
                });
            }
        };

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if RESERVED_DIRS.contains(&name.as_str()) {
                continue;
            }
            if let Ok(meta) = entry.metadata().await {
                if meta.is_dir() {
                    source_dirs.push(name);
                }
            }
        }

        // Filter to requested types if specified
        if let Some(ref types) = requested_types {
            source_dirs.retain(|d| types.contains(d));
        }

        // Sort for determinism
        source_dirs.sort();

        let mut files: Vec<serde_json::Value> = Vec::new();

        'outer: for source_type in &source_dirs {
            let type_path = inbox_path.join(source_type);
            let mut type_dir = match tokio::fs::read_dir(&type_path).await {
                Ok(rd) => rd,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = type_dir.next_entry().await {
                if files.len() >= limit {
                    break 'outer;
                }

                let file_name = entry.file_name().to_string_lossy().into_owned();

                // Skip hidden files and sidecar / error files
                if file_name.starts_with('.')
                    || file_name.ends_with(".sidecar.json")
                    || file_name.ends_with(".error.json")
                {
                    continue;
                }

                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }

                let rel_path = format!("{inbox_root}/{source_type}/{file_name}");

                // Canonicalize to block symlink escapes
                let full_path = self.security.workspace_dir.join(&rel_path);
                let Ok(resolved) = tokio::fs::canonicalize(&full_path).await else {
                    continue;
                };
                if !self.security.is_resolved_path_allowed(&resolved) {
                    continue;
                }

                let modified_unix = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                files.push(json!({
                    "path": rel_path,
                    "filename": file_name,
                    "source_type": source_type,
                    "size_bytes": meta.len(),
                    "modified_unix": modified_unix,
                }));
            }
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&files).unwrap_or_else(|_| "[]".into()),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{AutonomyLevel, SecurityPolicy};

    fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn inbox_scan_name() {
        let tool = InboxScanTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "inbox_scan");
    }

    #[test]
    fn inbox_scan_schema_has_no_required_fields() {
        let tool = InboxScanTool::new(test_security(std::env::temp_dir()));
        let schema = tool.parameters_schema();
        assert!(schema.is_object());
        let required = schema.get("required").and_then(|v| v.as_array());
        assert!(
            required.map(|r| r.is_empty()).unwrap_or(true),
            "inbox_scan should have no required parameters"
        );
    }

    #[tokio::test]
    async fn inbox_scan_returns_empty_when_inbox_missing() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_missing");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = InboxScanTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result.success);
        assert_eq!(result.output.trim(), "[]");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn inbox_scan_returns_only_unprocessed_files() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_unprocessed");
        let _ = tokio::fs::remove_dir_all(&dir).await;

        // pending file
        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/emails/msg.eml"), "hello")
            .await
            .unwrap();

        // already-processing file (should NOT appear)
        tokio::fs::create_dir_all(dir.join("inbox/processing/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/processing/emails/old.eml"), "in progress")
            .await
            .unwrap();

        // already-processed file (should NOT appear)
        tokio::fs::create_dir_all(dir.join("inbox/processed/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/processed/emails/done.eml"), "done")
            .await
            .unwrap();

        let tool = InboxScanTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result.success, "scan should succeed");
        let files: Vec<serde_json::Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(files.len(), 1, "only the pending file should be returned");
        assert_eq!(files[0]["filename"].as_str().unwrap(), "msg.eml");
        assert_eq!(files[0]["source_type"].as_str().unwrap(), "emails");
        assert!(files[0]["path"]
            .as_str()
            .unwrap()
            .starts_with("inbox/emails/"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn inbox_scan_respects_limit() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_limit");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();

        for i in 0..10 {
            tokio::fs::write(
                dir.join(format!("inbox/emails/msg{i:02}.eml")),
                format!("email {i}"),
            )
            .await
            .unwrap();
        }

        let tool = InboxScanTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({"limit": 3})).await.unwrap();

        assert!(result.success);
        let files: Vec<serde_json::Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(files.len(), 3, "limit should cap results at 3");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn inbox_scan_filters_by_source_types() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_filter");
        let _ = tokio::fs::remove_dir_all(&dir).await;

        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(dir.join("inbox/transcripts"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/emails/a.eml"), "email")
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/transcripts/b.txt"), "transcript")
            .await
            .unwrap();

        let tool = InboxScanTool::new(test_security(dir.clone()));

        // Only request emails
        let result = tool
            .execute(json!({"source_types": ["emails"]}))
            .await
            .unwrap();
        let files: Vec<serde_json::Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["source_type"].as_str().unwrap(), "emails");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn inbox_scan_excludes_sidecar_files() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_sidecar");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/emails/msg.eml"), "email")
            .await
            .unwrap();
        tokio::fs::write(
            dir.join("inbox/emails/msg.eml.sidecar.json"),
            r#"{"status":"done"}"#,
        )
        .await
        .unwrap();
        tokio::fs::write(
            dir.join("inbox/emails/msg.eml.error.json"),
            r#"{"error":"oops"}"#,
        )
        .await
        .unwrap();

        let tool = InboxScanTool::new(test_security(dir.clone()));
        let result = tool.execute(json!({})).await.unwrap();
        let files: Vec<serde_json::Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(
            files.len(),
            1,
            "only the actual file, not sidecar/error files"
        );
        assert_eq!(files[0]["filename"].as_str().unwrap(), "msg.eml");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn inbox_scan_rejects_path_traversal_inbox_root() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_traversal");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = InboxScanTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"inbox_root": "../../../etc"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not allowed"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn inbox_scan_blocks_when_rate_limited() {
        let dir = std::env::temp_dir().join("zeroclaw_test_inbox_scan_rate_limited");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = InboxScanTool::new(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: dir.clone(),
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        }));
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Rate limit exceeded"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
