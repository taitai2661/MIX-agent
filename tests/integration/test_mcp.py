import asyncio
import importlib.util
from pathlib import Path
import sys

async def test_real_stdio_mcp_discovery_and_call(tmp_path, monkeypatch):
    root = Path(__file__).resolve().parents[2]
    spec = importlib.util.spec_from_file_location("mix_test_mcp_runner", root / "runners/mcp/main.py")
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    monkeypatch.setattr(runner, "SHARED", tmp_path)
    server = tmp_path / "fake_mcp.py"
    server.write_text('from mcp.server.fastmcp import FastMCP\nmcp=FastMCP("test")\n@mcp.tool()\ndef echo(text: str) -> str:\n    return "echo:"+text\nmcp.run(transport="stdio")\n')
    conn = {"transport": "stdio", "command": sys.executable, "args": [str(server)]}
    try:
        discovered = await runner.invoke("discover", {"connection": conn})
        assert discovered["tools"][0]["name"] == "echo"
        result = await runner.invoke("call", {"connection": conn, "tool": "echo", "arguments": {"text": "hello"}, "run_id": "test"})
        assert result["content"][0]["text"] == "echo:hello"
    finally:
        for queue, task in runner.CONNECTIONS.values():
            task.cancel()
        await asyncio.gather(*(task for queue, task in runner.CONNECTIONS.values()), return_exceptions=True)
