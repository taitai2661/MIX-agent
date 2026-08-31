"""Explicit local test fixture; never used as an application fallback."""
import json
from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()

@app.get("/v1/models")
async def models():
    return {"data": [{"id": "mix-test-model", "name": "検証専用モデル（実AIではありません）",
        "supported_parameters": ["tools", "response_format"], "architecture": {"input_modalities": ["text"]}}]}

@app.post("/v1/chat/completions")
async def chat(body: dict):
    def events():
        for part in ["これはテスト用Providerからの応答です。", "\n\nストリーミングと会話履歴を確認しています。"]:
            yield "data: " + json.dumps({"id":"test-response","object":"chat.completion.chunk","created":0,"model":"mix-test-model", "choices":[{"index":0,"delta":{"content":part},"finish_reason":None}]}, ensure_ascii=False) + "\n\n"
        yield 'data: {"id":"test-response","object":"chat.completion.chunk","created":0,"model":"mix-test-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n'
        yield "data: [DONE]\n\n"
    return StreamingResponse(events(), media_type="text/event-stream")
