import pytest
from jsonschema.exceptions import ValidationError
from mix_agent.db.models import Conversation, Run, User, uid
from mix_agent.db.session import SessionLocal
from mix_agent.knowledge import service as knowledge
from mix_agent.tools import execute as tools_execute
from mix_agent.tools.registry import BUILTINS, fingerprint, permission
from sqlalchemy import select


def by_id(tool_id):
    return next(t for t in BUILTINS if t["id"] == tool_id)


def test_new_tool_ids_and_default_permissions(signed):
    assert by_id("web_fetch_pdf")["default_permission"] == "allow"
    assert by_id("knowledge_search")["default_permission"] == "allow"
    assert by_id("knowledge_add")["default_permission"] == "ask"
    assert by_id("knowledge_delete")["default_permission"] == "ask"
    assert by_id("web_clip_save")["default_permission"] == "ask"
    assert by_id("browser_extract")["default_permission"] == "ask"
    assert by_id("browser_wait")["default_permission"] == "ask"
    # New tools validate their schemas without extra dependencies.
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Test"})
        db.add(conversation)
        db.flush()
        tool = by_id("knowledge_search")
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key=uid(),
            data={"snapshot": {"mode": "chat", "tool_ids": [tool["id"]], "tools": [tool]}},
        )
        db.add(run)
        db.commit()
        assert permission(db, run, tool, {"query": "hello"}) == "allow"
        with pytest.raises(ValidationError):
            permission(db, run, tool, {"query": "hello", "top_k": 999})


def test_web_search_domain_and_freshness_helpers():
    assert tools_execute._sanitize_domains(["https://Example.COM/path", "foo", "foo"]) == ["example.com", "foo"]
    assert tools_execute._sanitize_domains([None, ""]) == []
    assert tools_execute._apply_domains("news", ["example.com"]) == "news site:example.com"
    assert tools_execute._apply_domains("news", []) == "news"
    assert tools_execute.DDGS_TIMELIMIT["week"] == "w"
    assert tools_execute.BRAVE_FRESHNESS["day"] == "pd"


def test_page_text_and_markdown_helpers():
    html = """
    <html><head><title>Example Title</title>
    <meta name="description" content="A short summary">
    <meta property="article:published_time" content="2026-01-02">
    </head><body>
    <nav>skip me</nav>
    <main><h1>Headline</h1><p>Hello <a href="https://example.com/more">more</a></p></main>
    <script>var x = 1;</script>
    </body></html>
    """
    title, description, published, headings, links, text = tools_execute._page_text(html)
    assert title == "Example Title"
    assert description == "A short summary"
    assert published == "2026-01-02"
    assert headings == ["Headline"]
    assert links == [{"text": "more", "url": "https://example.com/more"}]
    assert "Hello" in text and "skip me" not in text and "var x" not in text
    markdown = tools_execute._page_markdown(html)
    assert "# Headline" in markdown
    assert "[more](https://example.com/more)" in markdown


def test_knowledge_add_search_delete_roundtrip(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        doc = knowledge.add(db, owner, "PostgreSQLのpg_trgmで部分一致検索を行う手順のメモ", title="pg_trgm手順", source_type="text", memo="DB")
        db.commit()
        assert doc["chunks"] >= 1
        results = knowledge.search(db, owner, "pg_trgm 部分一致")
        assert results and results[0]["id"] == doc["id"]
        assert results[0]["title"] == "pg_trgm手順"
        assert "pg_trgm" in results[0]["excerpt"]
        listed = knowledge.list_documents(db, owner)
        assert any(entry["id"] == doc["id"] for entry in listed)
        deleted = knowledge.delete(db, owner, doc["id"])
        db.commit()
        assert deleted == {"id": doc["id"], "deleted": True, "chunks": doc["chunks"]}
        assert knowledge.search(db, owner, "pg_trgm") == []


def test_knowledge_long_content_is_chunked_and_searchable(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        content = ("検索対象の文章です。" * 400) + "合言葉はミックスナッツ"
        doc = knowledge.add(db, owner, content, title="長い文書")
        db.commit()
        assert doc["chunks"] > 1
        results = knowledge.search(db, owner, "ミックスナッツ")
        assert results and results[0]["id"] == doc["id"]
        knowledge.delete(db, owner, doc["id"])
        db.commit()


def test_knowledge_rejects_secrets_and_empty(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        with pytest.raises(ValueError):
            knowledge.add(db, owner, "api_key=should-not-be-stored")
        with pytest.raises(ValueError):
            knowledge.add(db, owner, "   ")
        with pytest.raises(ValueError):
            knowledge.delete(db, owner, "missing-id")


def test_pdf_text_extraction_helper():
    import io

    from pypdf import PdfWriter

    writer = PdfWriter()
    writer.add_blank_page(width=200, height=200)
    raw = io.BytesIO()
    writer.write(raw)
    text, pages = tools_execute._extract_pdf_text(raw.getvalue(), 10)
    assert pages == 1
    assert isinstance(text, str)


def test_tool_fingerprints_cover_new_definitions():
    ids = {tool["id"] for tool in BUILTINS}
    for expected in ("web_fetch_pdf", "browser_extract", "browser_wait", "knowledge_add", "knowledge_search", "knowledge_delete", "web_clip_save"):
        assert expected in ids
    assert len({fingerprint(tool) for tool in BUILTINS}) == len(BUILTINS)
