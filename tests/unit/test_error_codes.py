import pytest
from fastapi import HTTPException

from mix_agent.api.error_codes import APIError, code_for_detail, message_for


def test_api_error_carries_an_explicit_code_and_canonical_message():
    exc = APIError(401, "auth.login_required")
    assert isinstance(exc, HTTPException)
    assert exc.status_code == 401
    assert exc.code == "auth.login_required"
    # The Japanese message is resolved from the code, so call sites never have
    # to repeat the sentence.
    assert exc.detail == "ログインしてください"
    assert code_for_detail(exc.detail) == "auth.login_required"


def test_api_error_keeps_an_explicit_detail_for_dynamic_context():
    exc = APIError(422, "validation.invalid_input", detail="validator said no")
    assert exc.code == "validation.invalid_input"
    assert exc.detail == "validator said no"


def test_unknown_codes_do_not_raise_and_are_not_translated():
    assert message_for("does.not.exist") is None
    exc = APIError(500, "does.not.exist")
    assert exc.code == "does.not.exist"
    assert exc.detail == "does.not.exist"
    assert code_for_detail("does.not.exist") is None


@pytest.mark.parametrize("detail", [None, {"code": "dict"}, ["list"]])
def test_code_for_detail_ignores_non_string_details(detail):
    assert code_for_detail(detail) is None


def _http_error(exc):
    import asyncio
    import json

    from mix_agent.main import http_error

    response = asyncio.run(http_error(None, exc))
    return json.loads(response.body)


def test_bare_http_exception_gets_a_status_fallback_code():
    # `HTTPException(404)` has no message table entry; the UI still needs a
    # translatable code instead of falling back to English "Not Found".
    body = _http_error(HTTPException(404))
    assert body["code"] == "common.http_404"
    assert body["detail"] == "Not Found"


def test_api_error_code_wins_over_the_message_table():
    body = _http_error(APIError(401, "auth.login_required"))
    assert body["code"] == "auth.login_required"
    assert body["detail"] == "ログインしてください"


def test_openapi_documents_the_error_envelope():
    from mix_agent.main import app

    schema = app.openapi()
    assert "ErrorResponse" in schema["components"]["schemas"]
    responses = schema["paths"]["/health"]["get"]["responses"]
    assert {"4XX", "5XX"} <= set(responses)
    assert responses["4XX"]["content"]["application/json"]["schema"]["$ref"] == (
        "#/components/schemas/ErrorResponse"
    )
