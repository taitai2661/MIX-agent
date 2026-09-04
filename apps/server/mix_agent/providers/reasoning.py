"""Resolve reasoning once per run; never infer API support from a mode alone."""

import re


def reasoning_control(kind: str, model: str) -> str | None:
    if kind in ("openai", "openrouter"):
        return kind
    if kind == "anthropic":
        if re.match(r"^claude-(opus|sonnet)-4-[678](?:-|$)", model):
            return "anthropic_adaptive"
        if re.fullmatch(
            r"claude-(?:(?:opus|sonnet|haiku)-4(?:-[15])?|3-7-sonnet)(?:-(?:\d{8}|latest))?", model
        ):
            return "anthropic_budget"
    if kind == "gemini":
        if model.startswith("gemini-2.5-"):
            return "gemini_budget"
        if re.match(r"^gemini-3(?:\.\d+)?-", model):
            return "gemini_level"
    # Ollama/LM Studio/generic compatible endpoints do not share a verified
    # reasoning-control protocol. A capability override alone is insufficient.
    return None


def resolve_reasoning(kind: str, model: str, caps: dict, mode: str, settings: dict) -> dict:
    if mode == "agent":
        return {"policy": "legacy"}
    control = reasoning_control(kind, model)
    if caps.get("reasoning") is not True or control is None:
        # Thinking is also useful with ordinary inference: the run loop can
        # still make several tool-assisted steps.  Do not send unverified
        # provider-specific reasoning parameters in that case.
        return {
            "policy": "tool_assisted" if mode == "thinking" else "off",
            "request": {},
            "summary": False,
        }
    auto = mode == "chat"
    effort = "low" if auto else settings.get("reasoning_effort", "medium")
    if effort not in ("low", "medium", "high", "xhigh", "max"):
        raise ValueError("Thinkingのreasoning_effortはlow以上に設定してください。")
    request = {}
    if control == "openai":
        request = {
            "reasoning": {"summary": "auto", "effort": effort},
            "include": ["reasoning.encrypted_content"],
        }
    elif control == "openrouter":
        request = {
            "extra_body": {
                "reasoning": {"enabled": True, "effort": effort},
                "provider": {"require_parameters": True},
            }
        }
    elif control == "anthropic_adaptive":
        request = {"thinking": {"type": "adaptive"}, "output_config": {"effort": "low" if auto else "high"}}
    elif control == "anthropic_budget":
        maximum = settings.get("max_output_tokens", 4096)
        if not isinstance(maximum, int) or isinstance(maximum, bool) or maximum <= 1024:
            raise ValueError("このモデルの思考にはmax_output_tokensを1024より大きく設定してください。")
        request = {"thinking": {"type": "enabled", "budget_tokens": 1024}}
    elif control == "gemini_budget":
        request = {"thinking_config": {"include_thoughts": True, "thinking_budget": -1 if auto else 1024}}
    elif control == "gemini_level":
        request = {"thinking_config": {"include_thoughts": True, "thinking_level": "low" if auto else "high"}}
    return {
        "policy": "auto" if auto else "required",
        "control": control,
        "request": request,
        "summary": not model.startswith("claude-3-7-"),
    }


def show_summary(mode: str, settings: dict) -> bool:
    resolved = settings.get("_resolved_reasoning")
    if resolved is None or resolved["policy"] == "legacy":
        return mode != "chat"
    return resolved["summary"]


def request_options(settings: dict) -> dict | None:
    resolved = settings.get("_resolved_reasoning")
    if resolved is None or resolved["policy"] == "legacy":
        return None
    return resolved["request"]
