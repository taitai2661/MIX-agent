from mix_agent.tools.execute import safe_upload_name


def test_safe_upload_name_strips_paths_and_control_characters():
    assert safe_upload_name("../../etc/passwd") == "passwd"
    assert safe_upload_name('..\\..\\windows\\system32\\cmd.exe') == "cmd.exe"
    assert safe_upload_name("report\r\nInjected: yes.txt") == "reportInjected: yes.txt"
    assert safe_upload_name("日本語.pdf") == "日本語.pdf"
    assert safe_upload_name("") == "attachment"


def test_safe_upload_name_keeps_the_extension_when_truncating():
    name = safe_upload_name("a" * 300 + ".markdown")
    assert len(name) == 200
    assert name.endswith(".markdown")
