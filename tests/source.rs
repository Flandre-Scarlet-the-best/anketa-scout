mod common;
use anketa_scout::{
    Failure,
    source::{MAX_COMMENT, ThreadUrl, normalize, normalize_html},
};
use common::{envelope, post, source};
use serde_json::json;

#[test]
fn confirmed_json_shape_preserves_origin_date_files_and_raw_body() {
    let mut reply = post(101, "Люблю <b>го</b> &amp; шахматы.<br>Тг: synthetic_user");
    reply["timestamp"] = json!(0);
    reply["files"] =
        json!([{"path":"/test/src/100/image.png","displayname":"Схема.png","size":12}]);
    let messages = normalize(
        &source(),
        &serde_json::to_vec(&envelope(vec![post(100, "Вступление"), reply])).unwrap(),
    )
    .unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].is_introductory());
    let message = &messages[1];
    assert_eq!(message.origin.source, "2ch.org");
    assert_eq!(message.origin.board, "test");
    assert_eq!(message.origin.thread, 100);
    assert_eq!(message.origin.post, 101);
    assert_eq!(message.origin.link, "https://2ch.org/test/res/100.html#101");
    assert_eq!(message.text(), "Люблю го & шахматы.\nТг: synthetic_user");
    assert_eq!(
        message.original(),
        "Люблю <b>го</b> &amp; шахматы.<br>Тг: synthetic_user"
    );
    assert_eq!(message.date.as_deref(), Some("1970-01-01 00:00:00 UTC"));
    assert_eq!(
        message.attachments[0].link,
        "https://2ch.org/test/src/100/image.png"
    );
    assert_eq!(message.attachments[0].name.as_deref(), Some("Схема.png"));
    assert_eq!(message.attachments[0].size_kib, Some(12));
}

#[test]
fn missing_op_is_partial_even_when_count_is_zero() {
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&envelope(vec![])).unwrap()).unwrap_err(),
        Failure::Partial
    );
}

#[test]
fn missing_optional_fields_remain_absent() {
    let messages = normalize(
        &source(),
        &serde_json::to_vec(&envelope(vec![post(100, "Текст")])).unwrap(),
    )
    .unwrap();
    assert_eq!(messages[0].date, None);
    assert!(messages[0].attachments.is_empty());
}

#[test]
fn confirmed_null_files_means_no_attachments() {
    let mut input = envelope(vec![post(100, "Текст без вложений")]);
    input["threads"][0]["posts"][0]["files"] = serde_json::Value::Null;
    let messages = normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].attachments.is_empty());
    assert_eq!(messages[0].text(), "Текст без вложений");
}

#[test]
fn partial_count_or_missing_op_is_not_complete() {
    let mut input = envelope(vec![post(100, "Текст")]);
    input["posts_count"] = json!(2);
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap_err(),
        Failure::Partial
    );
    let missing_op = envelope(vec![post(101, "Текст")]);
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&missing_op).unwrap()).unwrap_err(),
        Failure::Partial
    );
}

#[test]
fn wrong_identity_duplicates_and_wrong_parent_are_rejected() {
    for (key, value) in [
        ("board", json!({"id":"other"})),
        ("current_thread", json!(999)),
    ] {
        let mut input = envelope(vec![post(100, "Текст")]);
        input[key] = value;
        assert_eq!(
            normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap_err(),
            Failure::Format
        );
    }
    let input = envelope(vec![post(100, "Текст"), post(100, "Повтор")]);
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap_err(),
        Failure::Format
    );
    let mut input = envelope(vec![post(100, "Текст"), post(101, "Ответ")]);
    input["threads"][0]["posts"][1]["parent"] = json!(999);
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap_err(),
        Failure::Format
    );
}

#[test]
fn invalid_json_and_missing_required_fields_fail_closed() {
    for bytes in [b"{".as_slice(), b"{}", b"<html>Captcha</html>"] {
        assert_eq!(normalize(&source(), bytes).unwrap_err(), Failure::Format);
    }
    let mut input = envelope(vec![post(100, "Текст")]);
    input["threads"][0]["posts"][0]
        .as_object_mut()
        .unwrap()
        .remove("files");
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap_err(),
        Failure::Format
    );
}

#[test]
fn attachment_urls_cannot_change_host_or_escape_board() {
    for path in [
        "https://evil.invalid/x",
        "//127.0.0.1/x",
        "/test/src/../secret",
        "/test/src/file%",
        "/test/src/x?secret=hidden",
        "/other/src/x",
        "/test/src/%2e%2e/secret",
        "/test/src/%2E%2E/%2e%2e/secret",
        "/test/src/.%2e/secret",
        "/test/src/%2f..%2fsecret",
        "/test/src/%2F..%2Fsecret",
        "/test/src/%5c..%5csecret",
        "/test/src/%252f%252e%252e%252fsecret",
    ] {
        let mut input = envelope(vec![post(100, "Текст")]);
        input["threads"][0]["posts"][0]["files"] = json!([{"path":path}]);
        assert_eq!(
            normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap_err(),
            Failure::Format,
            "{path}"
        );
    }
}

#[test]
fn encoded_percent_is_preserved_in_safe_attachment_filenames() {
    for path in [
        "/test/src/100/file%25report.png",
        "/test/src/100/file%25report%20final.png",
    ] {
        let mut input = envelope(vec![post(100, "Текст")]);
        input["threads"][0]["posts"][0]["files"] = json!([{"path":path}]);
        let messages = normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap();
        assert_eq!(messages[0].attachments.len(), 1, "{path}");
        assert!(messages[0].attachments[0].link.contains("%25report"));
    }
}

#[test]
fn comment_size_boundary_and_deep_markup_are_bounded() {
    assert!(normalize_html(&"a".repeat(MAX_COMMENT)).is_ok());
    assert_eq!(
        normalize_html(&"a".repeat(MAX_COMMENT + 1)).unwrap_err(),
        Failure::Limit
    );
    assert_eq!(
        normalize_html(&format!(
            "{}текст{}",
            "<div>".repeat(130),
            "</div>".repeat(130)
        ))
        .unwrap_err(),
        Failure::Limit
    );
}

#[test]
fn quotes_and_hidden_executable_markup_are_excluded_from_author_evidence() {
    let (text, authored) = normalize_html(
        "<blockquote>Чужая цитата</blockquote><script>Люблю го</script><p>Мой текст</p>",
    )
    .unwrap();
    assert_eq!(text, "Чужая цитата\nМой текст");
    assert_eq!(authored, "Мой текст");
}

#[test]
fn only_supported_public_thread_urls_are_accepted() {
    assert_eq!(
        ThreadUrl::parse("https://2ch.org/test/res/100.json").unwrap(),
        source()
    );
    for url in [
        "http://2ch.org/test/res/100.html",
        "https://2ch.org.evil.invalid/test/res/100.html",
        "https://127.0.0.1/test/res/100.html",
        "file:///x",
        "https://user:pass@2ch.org/test/res/100.html",
        "https://2ch.org/test/res/100.html?token=private",
        "https://2ch.org/test/res/100.html#101",
        "https://2ch.org/test/res/0.html",
        "https://2ch.org:8443/test/res/100.html",
        "https://2ch.org/test/res/999999999999999999999999.html",
    ] {
        assert!(
            matches!(ThreadUrl::parse(url), Err(Failure::Config(_))),
            "{url}"
        );
    }
}

#[test]
fn post_count_and_attachment_limits_accept_boundary_then_reject_overflow() {
    use anketa_scout::source::MAX_POSTS;
    let mut posts = vec![post(100, "Вступление")];
    posts.extend((101..100 + MAX_POSTS as u64).map(|id| post(id, "")));
    assert_eq!(
        normalize(
            &source(),
            &serde_json::to_vec(&envelope(posts.clone())).unwrap()
        )
        .unwrap()
        .len(),
        MAX_POSTS
    );
    posts.push(post(100 + MAX_POSTS as u64, ""));
    assert_eq!(
        normalize(&source(), &serde_json::to_vec(&envelope(posts)).unwrap()).unwrap_err(),
        Failure::Limit
    );

    let mut input = envelope(vec![post(100, "Вступление")]);
    for count in [100, 101] {
        input["threads"][0]["posts"][0]["files"] =
            json!(vec![json!({"path":"/test/src/100/file.png"}); count]);
        let result = normalize(&source(), &serde_json::to_vec(&input).unwrap());
        if count == 100 {
            assert_eq!(result.unwrap()[0].attachments.len(), 100);
        } else {
            assert_eq!(result.unwrap_err(), Failure::Limit);
        }
    }
}

#[test]
fn date_text_is_used_only_when_timestamp_is_unavailable() {
    let mut input = envelope(vec![post(100, "Текст")]);
    input["threads"][0]["posts"][0]["date"] = json!("Дата из источника");
    let messages = normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(messages[0].date.as_deref(), Some("Дата из источника"));
    input["threads"][0]["posts"][0]["timestamp"] = json!(i64::MAX);
    let messages = normalize(&source(), &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(messages[0].date.as_deref(), Some("Дата из источника"));
}
