use super::traits::{Tool, ToolResult};
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

/// Advance a file through the inbox processing pipeline.
///
/// Three actions:
/// - `start`: move `inbox/{source_type}/{file}` → `inbox/processing/{source_type}/{file}`
/// - `complete`: move to `inbox/processed/{source_type}/{file}`, optionally writing a `.sidecar.json`
/// - `fail`: move to `inbox/failed/{source_type}/{file}`, optionally writing a `.error.json`
pub struct FileStageTool {
    security: Arc<SecurityPolicy>,
}

impl FileStageTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// Parse a workspace-relative path into `(inbox_root, stage, source_type, filename)`.
    ///
    /// `start` paths have the shape:    `inbox/{source_type}/{filename}`     → stage = None
    /// `complete`/`fail` paths:         `inbox/{stage}/{source_type}/{filename}` → stage = Some(...)
    fn parse_path(path: &str) -> Option<(&str, Option<&str>, &str, &str)> {
        let parts: Vec<&str> = path.splitn(5, '/').collect();
        match parts.as_slice() {
            [inbox_root, source_type, filename] => Some((inbox_root, None, source_type, filename)),
            [inbox_root, stage, source_type, filename] => {
                Some((inbox_root, Some(stage), source_type, filename))
            }
            _ => None,
        }
    }

    /// Verify that each path component is a plain name with no traversal characters.
    fn is_safe_component(component: &str) -> bool {
        !component.is_empty()
            && component != ".."
            && component != "."
            && !component.contains('/')
            && !component.contains('\0')
    }

    async fn stage_start(
        &self,
        rel_path: &str,
        resolved_src: &Path,
        _args: &serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let parsed = Self::parse_path(rel_path);
        let Some((inbox_root, None, source_type, filename)) = parsed else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Path '{rel_path}' must match inbox/<source_type>/<filename> for action 'start'"
                )),
            });
        };

        if !Self::is_safe_component(inbox_root)
            || !Self::is_safe_component(source_type)
            || !Self::is_safe_component(filename)
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Path component contains disallowed characters".into()),
            });
        }

        let dst_dir = self
            .security
            .workspace_dir
            .join(inbox_root)
            .join("processing")
            .join(source_type);

        if let Err(e) = tokio::fs::create_dir_all(&dst_dir).await {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to create processing dir: {e}")),
            });
        }

        // Verify the destination dir is within workspace
        if !self.security.is_resolved_path_allowed(&dst_dir) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Destination path escapes workspace".into()),
            });
        }

        let dst_path = dst_dir.join(filename);

        if let Err(e) = tokio::fs::rename(resolved_src, &dst_path).await {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to move file to processing: {e}")),
            });
        }

        let new_rel = format!("{inbox_root}/processing/{source_type}/{filename}");
        Ok(ToolResult {
            success: true,
            output: format!("Claimed: {new_rel}"),
            error: None,
        })
    }

    async fn stage_complete(
        &self,
        rel_path: &str,
        resolved_src: &Path,
        args: &serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let parsed = Self::parse_path(rel_path);
        let Some((inbox_root, Some("processing"), source_type, filename)) = parsed else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Path '{rel_path}' must match inbox/processing/<source_type>/<filename> for action 'complete'"
                )),
            });
        };

        if !Self::is_safe_component(inbox_root)
            || !Self::is_safe_component(source_type)
            || !Self::is_safe_component(filename)
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Path component contains disallowed characters".into()),
            });
        }

        let dst_dir = self
            .security
            .workspace_dir
            .join(inbox_root)
            .join("processed")
            .join(source_type);

        if let Err(e) = tokio::fs::create_dir_all(&dst_dir).await {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to create processed dir: {e}")),
            });
        }

        if !self.security.is_resolved_path_allowed(&dst_dir) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Destination path escapes workspace".into()),
            });
        }

        let dst_path = dst_dir.join(filename);

        if let Err(e) = tokio::fs::rename(resolved_src, &dst_path).await {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to move file to processed: {e}")),
            });
        }

        // Write sidecar JSON if provided — non-fatal if it fails (file is already moved)
        if let Some(sidecar) = args.get("sidecar") {
            let sidecar_path = dst_dir.join(format!("{filename}.sidecar.json"));
            if let Ok(sidecar_bytes) = serde_json::to_vec_pretty(sidecar) {
                let _ = tokio::fs::write(&sidecar_path, &sidecar_bytes).await;
            }
        }

        let new_rel = format!("{inbox_root}/processed/{source_type}/{filename}");
        Ok(ToolResult {
            success: true,
            output: format!("Completed: {new_rel}"),
            error: None,
        })
    }

    async fn stage_fail(
        &self,
        rel_path: &str,
        resolved_src: &Path,
        args: &serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let parsed = Self::parse_path(rel_path);
        let Some((inbox_root, Some("processing"), source_type, filename)) = parsed else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Path '{rel_path}' must match inbox/processing/<source_type>/<filename> for action 'fail'"
                )),
            });
        };

        if !Self::is_safe_component(inbox_root)
            || !Self::is_safe_component(source_type)
            || !Self::is_safe_component(filename)
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Path component contains disallowed characters".into()),
            });
        }

        let dst_dir = self
            .security
            .workspace_dir
            .join(inbox_root)
            .join("failed")
            .join(source_type);

        if let Err(e) = tokio::fs::create_dir_all(&dst_dir).await {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to create failed dir: {e}")),
            });
        }

        if !self.security.is_resolved_path_allowed(&dst_dir) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Destination path escapes workspace".into()),
            });
        }

        let dst_path = dst_dir.join(filename);

        if let Err(e) = tokio::fs::rename(resolved_src, &dst_path).await {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to move file to failed: {e}")),
            });
        }

        // Write error JSON if provided — non-fatal if it fails
        if let Some(error_msg) = args.get("error").and_then(|v| v.as_str()) {
            let error_path = dst_dir.join(format!("{filename}.error.json"));
            if let Ok(error_bytes) = serde_json::to_vec_pretty(&json!({"error": error_msg})) {
                let _ = tokio::fs::write(&error_path, &error_bytes).await;
            }
        }

        let new_rel = format!("{inbox_root}/failed/{source_type}/{filename}");
        Ok(ToolResult {
            success: true,
            output: format!("Failed: {new_rel}"),
            error: None,
        })
    }
}

#[async_trait]
impl Tool for FileStageTool {
    fn name(&self) -> &str {
        "file_stage"
    }

    fn description(&self) -> &str {
        "Advance a file through the inbox processing pipeline. Use action='start' to claim \
         a file (moves it to processing/), 'complete' to mark it done (moves to processed/, \
         optionally writes a .sidecar.json), or 'fail' to move it to failed/ with an optional \
         error note. All paths are workspace-relative."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace-relative path to the file. For 'start': inbox/<source_type>/<filename>. For 'complete'/'fail': inbox/processing/<source_type>/<filename>."
                },
                "action": {
                    "type": "string",
                    "enum": ["start", "complete", "fail"],
                    "description": "'start': claim file for processing; 'complete': mark done; 'fail': mark failed"
                },
                "sidecar": {
                    "type": "object",
                    "description": "JSON object written as <filename>.sidecar.json in the processed directory (action='complete' only)"
                },
                "error": {
                    "type": "string",
                    "description": "Error message written to <filename>.error.json in the failed directory (action='fail' only)"
                }
            },
            "required": ["path", "action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        if self.security.is_rate_limited() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: too many actions in the last hour".into()),
            });
        }

        if !self.security.is_path_allowed(path) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Path not allowed by security policy: {path}")),
            });
        }

        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: action budget exhausted".into()),
            });
        }

        let full_path = self.security.workspace_dir.join(path);
        let resolved = match tokio::fs::canonicalize(&full_path).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to resolve path: {e}")),
                });
            }
        };

        if !self.security.is_resolved_path_allowed(&resolved) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(self.security.resolved_path_violation_message(&resolved)),
            });
        }

        match action {
            "start" => self.stage_start(path, &resolved, &args).await,
            "complete" => self.stage_complete(path, &resolved, &args).await,
            "fail" => self.stage_fail(path, &resolved, &args).await,
            other => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown action '{other}': must be 'start', 'complete', or 'fail'"
                )),
            }),
        }
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
    fn file_stage_name() {
        let tool = FileStageTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "file_stage");
    }

    #[test]
    fn file_stage_schema_requires_path_and_action() {
        let tool = FileStageTool::new(test_security(std::env::temp_dir()));
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("action")));
    }

    #[test]
    fn parse_path_start_shape() {
        let result = FileStageTool::parse_path("inbox/emails/msg.eml");
        assert_eq!(
            result,
            Some(("inbox", None, "emails", "msg.eml")),
            "3-part path should parse as start shape"
        );
    }

    #[test]
    fn parse_path_stage_shape() {
        let result = FileStageTool::parse_path("inbox/processing/emails/msg.eml");
        assert_eq!(
            result,
            Some(("inbox", Some("processing"), "emails", "msg.eml")),
            "4-part path should parse as stage shape"
        );
    }

    #[test]
    fn parse_path_rejects_short_path() {
        assert!(
            FileStageTool::parse_path("inbox/msg.eml").is_none(),
            "2-part path should not parse"
        );
    }

    #[test]
    fn is_safe_component_rejects_traversal() {
        assert!(!FileStageTool::is_safe_component(".."));
        assert!(!FileStageTool::is_safe_component("."));
        assert!(!FileStageTool::is_safe_component(""));
        assert!(!FileStageTool::is_safe_component("foo/bar"));
        assert!(FileStageTool::is_safe_component("emails"));
        assert!(FileStageTool::is_safe_component("msg.eml"));
    }

    #[tokio::test]
    async fn file_stage_start_moves_file_to_processing() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_start");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/emails/msg.eml"), "hello")
            .await
            .unwrap();

        let tool = FileStageTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "inbox/emails/msg.eml", "action": "start"}))
            .await
            .unwrap();

        assert!(result.success, "start should succeed: {:?}", result.error);
        assert!(result.output.contains("Claimed:"));
        assert!(
            dir.join("inbox/processing/emails/msg.eml").exists(),
            "file should be in processing/"
        );
        assert!(
            !dir.join("inbox/emails/msg.eml").exists(),
            "file should no longer be in inbox/"
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_stage_complete_writes_sidecar_and_moves_file() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_complete");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/processing/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/processing/emails/msg.eml"), "hello")
            .await
            .unwrap();

        let tool = FileStageTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({
                "path": "inbox/processing/emails/msg.eml",
                "action": "complete",
                "sidecar": {"action_items": ["follow up"], "decisions": []}
            }))
            .await
            .unwrap();

        assert!(
            result.success,
            "complete should succeed: {:?}",
            result.error
        );
        assert!(result.output.contains("Completed:"));
        assert!(
            dir.join("inbox/processed/emails/msg.eml").exists(),
            "file should be in processed/"
        );
        assert!(
            dir.join("inbox/processed/emails/msg.eml.sidecar.json")
                .exists(),
            "sidecar should be written"
        );

        let sidecar_content =
            tokio::fs::read_to_string(dir.join("inbox/processed/emails/msg.eml.sidecar.json"))
                .await
                .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_content).unwrap();
        assert!(sidecar["action_items"].is_array());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_stage_fail_moves_file_and_writes_error() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_fail");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/processing/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/processing/emails/bad.eml"), "corrupted")
            .await
            .unwrap();

        let tool = FileStageTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({
                "path": "inbox/processing/emails/bad.eml",
                "action": "fail",
                "error": "Could not parse email headers"
            }))
            .await
            .unwrap();

        assert!(result.success, "fail should succeed: {:?}", result.error);
        assert!(result.output.contains("Failed:"));
        assert!(
            dir.join("inbox/failed/emails/bad.eml").exists(),
            "file should be in failed/"
        );
        assert!(
            dir.join("inbox/failed/emails/bad.eml.error.json").exists(),
            "error JSON should be written"
        );

        let error_content =
            tokio::fs::read_to_string(dir.join("inbox/failed/emails/bad.eml.error.json"))
                .await
                .unwrap();
        let error_json: serde_json::Value = serde_json::from_str(&error_content).unwrap();
        assert_eq!(
            error_json["error"].as_str().unwrap(),
            "Could not parse email headers"
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_stage_start_rejects_wrong_path_shape() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_bad_start_path");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/processing/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/processing/emails/msg.eml"), "hello")
            .await
            .unwrap();

        let tool = FileStageTool::new(test_security(dir.clone()));
        // Passing a processing-stage path to 'start' should fail
        let result = tool
            .execute(json!({
                "path": "inbox/processing/emails/msg.eml",
                "action": "start"
            }))
            .await
            .unwrap();

        assert!(!result.success);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_stage_rejects_path_outside_workspace() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_escape");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileStageTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "../../../etc/passwd", "action": "start"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not allowed"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_stage_rejects_unknown_action() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_unknown_action");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/emails/msg.eml"), "hello")
            .await
            .unwrap();

        let tool = FileStageTool::new(test_security(dir.clone()));
        let result = tool
            .execute(json!({"path": "inbox/emails/msg.eml", "action": "reprocess"}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result
            .error
            .as_ref()
            .unwrap()
            .contains("Unknown action 'reprocess'"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_stage_blocks_when_rate_limited() {
        let dir = std::env::temp_dir().join("zeroclaw_test_file_stage_rate_limited");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("inbox/emails"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("inbox/emails/msg.eml"), "hello")
            .await
            .unwrap();

        let tool = FileStageTool::new(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: dir.clone(),
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        }));
        let result = tool
            .execute(json!({"path": "inbox/emails/msg.eml", "action": "start"}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Rate limit exceeded"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
