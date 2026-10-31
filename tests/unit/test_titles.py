"""A user-chosen title is never mistaken for the generated placeholder."""

from mix_agent.api.common import set_title_placeholder, title_is_placeholder, with_generated_title


def test_a_new_conversation_is_a_placeholder():
    value = {}
    set_title_placeholder(value, explicit=False)
    assert title_is_placeholder(value) is True


def test_an_explicit_title_is_not_a_placeholder():
    value = {}
    set_title_placeholder(value, explicit=True)
    assert title_is_placeholder(value) is False


def test_a_renamed_conversation_keeps_its_title():
    value = with_generated_title({"title": "旅行", "title_is_placeholder": False}, "新しい話")
    assert value["title"] == "旅行"
    assert title_is_placeholder(value) is False


def test_the_first_message_replaces_only_the_placeholder():
    value = with_generated_title({"title": "新しいチャット", "title_is_placeholder": True}, "旅行の計画")
    assert value["title"] == "旅行の計画"
    assert title_is_placeholder(value) is False
    # A second message must not overwrite the generated title.
    again = with_generated_title(value, "別の話")
    assert again["title"] == "旅行の計画"


def test_legacy_rows_without_the_flag_still_work():
    assert title_is_placeholder({"title": "新しいチャット"}) is True
    assert title_is_placeholder({"title": "仕事"}) is False


def test_a_whitespace_only_attachment_turn_gets_a_real_title():
    value = with_generated_title(
        {"title": "新しいチャット", "title_is_placeholder": True},
        "   ",
    )
    assert value["title"] == "添付について"
    assert title_is_placeholder(value) is False
