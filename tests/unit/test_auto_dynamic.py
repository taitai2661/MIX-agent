from types import SimpleNamespace

from mix_agent.runs import engine


class FakeDB:
    def __init__(self, provider):
        self.provider = provider
        self.commits = 0

    def get(self, kind, key):
        return self.provider if key == self.provider.id else None

    def commit(self):
        self.commits += 1


def test_auto_reevaluates_after_tool_and_records_switch(monkeypatch):
    model = SimpleNamespace(id="new", data={"provider_id": "provider", "model_id": "better", "context_window": 64000, "capabilities": {"tools": True}})
    provider = SimpleNamespace(id="provider", owner_id="owner", data={"kind": "openai"})
    run = SimpleNamespace(owner_id="owner", id="run", request_key="request-key", data={
        "requested_model_id": "auto", "steps": 1, "history": [
            {"role": "user", "content": "Investigate this task"},
            {"role": "tool", "content": "A code fix is needed"},
        ], "auto_routing": {"allowed_ids": ["old", "new"], "content": "Investigate this task", "reserved_output_tokens": 4096},
    })
    snapshot = {"model_record_id": "old", "model_id": "old-model", "mode": "agent", "model_settings": {}, "tools": []}
    selected = {}
    events = []

    def choose(*args, **kwargs):
        selected["content"] = args[3]
        selected["parts"] = args[7]
        selected["current"] = kwargs["current_model_id"]
        return model, {"profile": "coding:tools", "reason": "local"}

    monkeypatch.setattr(engine, "select_auto_model", choose)
    monkeypatch.setattr(engine, "resolve_reasoning", lambda *args: {"policy": "off"})
    monkeypatch.setattr(engine, "update", lambda row, **values: row.data.update(values))
    monkeypatch.setattr(engine, "emit", lambda *args: events.append(args[2]))
    result = engine._reevaluate_auto_model(FakeDB(provider), run, snapshot, [{"id": "tool"}])

    assert result["model_record_id"] == "new"
    assert result["context_window_info"]["context_window"] == 64000
    assert selected["current"] == "old"
    assert "code fix" in selected["content"]
    assert len(selected["parts"]) == 2
    assert run.data["auto_model_history"][-1]["to_model_record_id"] == "new"
    assert events == ["model_rerouted"]
    assert "code fix" not in str(run.data["auto_selection"])


def test_auto_does_not_reevaluate_manual_or_disabled_runs(monkeypatch):
    monkeypatch.setattr(engine, "select_auto_model", lambda *args, **kwargs: 1 / 0)
    provider = SimpleNamespace(id="provider", owner_id="owner", data={"kind": "openai"})
    snapshot = {"model_record_id": "old", "mode": "agent"}
    for requested, enabled in (("manual", True), ("auto", False)):
        run = SimpleNamespace(data={"requested_model_id": requested, "auto_routing": {"dynamic_switching": enabled}})
        assert engine._reevaluate_auto_model(FakeDB(provider), run, snapshot, []) is snapshot


def test_auto_keeps_current_model_when_it_remains_best(monkeypatch):
    model = SimpleNamespace(id="same", data={"provider_id": "provider", "model_id": "same-model", "context_window": 32000})
    provider = SimpleNamespace(id="provider", owner_id="owner", data={"kind": "openai"})
    db = FakeDB(provider)
    run = SimpleNamespace(owner_id="owner", id="run", request_key="request-key", data={
        "requested_model_id": "auto", "steps": 1,
        "history": [{"role": "user", "content": "hello"}],
        "auto_routing": {"allowed_ids": ["same"], "content": "hello"},
    })
    snapshot = {"model_record_id": "same", "model_id": "same-model", "mode": "chat", "model_settings": {}}
    monkeypatch.setattr(engine, "select_auto_model", lambda *args, **kwargs: (model, {"profile": "general"}))
    monkeypatch.setattr(engine, "resolve_reasoning", lambda *args: {"policy": "off"})
    monkeypatch.setattr(engine, "update", lambda row, **values: row.data.update(values))
    monkeypatch.setattr(engine, "emit", lambda *args: 1 / 0)
    assert engine._reevaluate_auto_model(db, run, snapshot, []) is snapshot
    assert db.commits == 1
    assert "auto_model_history" not in run.data
