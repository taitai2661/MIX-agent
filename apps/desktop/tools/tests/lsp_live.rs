use mixd_tools::{lsp::LspTool, Tool, ToolContext};

#[tokio::test]
#[ignore = "requires rust-analyzer on PATH; run manually"]
async fn rust_analyzer_reports_a_type_error() {
    let root = std::env::temp_dir().join(format!("mixd-lsp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\nfn broken() -> i32 { \"no\" }\n",
    )
    .unwrap();

    let tool = LspTool::default();
    let ctx = ToolContext {
        run_id: uuid::Uuid::new_v4(),
        workspace_id: uuid::Uuid::new_v4(),
        root_path: Some(root.to_string_lossy().into_owned()),
        additional_paths: Vec::new(),
        approval_token: None,
    };
    let result = tool
        .invoke(
            ctx,
            serde_json::json!({ "action": "diagnostics", "path": "src/lib.rs", "wait_ms": 20000 }),
        )
        .await
        .expect("invoke");
    eprintln!("{}", serde_json::to_string_pretty(&result).unwrap());
    assert!(result.ok, "{:?}", result.error);
    let count = result.content["count"].as_u64().unwrap_or(0);
    assert!(count > 0, "expected a type error, got {result:?}");
    let _ = std::fs::remove_dir_all(&root);
}
