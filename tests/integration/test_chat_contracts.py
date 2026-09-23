from mix_agent.main import app


def test_chat_endpoints_publish_typed_openapi_responses():
    paths = app.openapi()["paths"]
    expected = {
        ("/api/v1/conversations/{key}/messages", "get"): "ConversationMessagesView",
        ("/api/v1/conversations/{key}/messages", "post"): "SendMessageView",
        ("/api/v1/conversations/{key}/tool-calls", "get"): "ToolCallHistoryView",
        ("/api/v1/runs/{key}", "get"): "RunView",
        ("/api/v1/artifacts", "post"): "ArtifactView",
        ("/api/v1/tools", "get"): "ToolView",
        ("/api/v1/permission-rules", "get"): "PermissionRuleView",
    }
    for (path, method), schema in expected.items():
        response = paths[path][method]["responses"]["200"]["content"]["application/json"]["schema"]
        reference = response["items"]["$ref"] if "items" in response else response["$ref"]
        assert reference == f"#/components/schemas/{schema}"
