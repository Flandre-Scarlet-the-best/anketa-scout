mod common;
use anketa_scout::{
    domain::{ContactKind, Ownership, Rejection, classify},
    source::normalize,
};
use common::{envelope, message, post, source};

#[test]
fn full_profile_preserves_own_contact_and_original() {
    let profile = classify(message(
        "Я программист. Люблю походы и настольные игры.<br>Мой Telegram: synthetic_user",
    ))
    .unwrap();
    assert_eq!(profile.contacts.len(), 1);
    assert_eq!(profile.contacts[0].kind, ContactKind::Telegram);
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
    assert_eq!(profile.contacts[0].ownership, Ownership::ExplicitSelf);
    assert!(profile.message.original().contains("<br>"));
    assert!(!profile.message.text().contains("<br>"));
}

#[test]
fn short_profile_needs_no_demographics_or_at_sign() {
    let profile = classify(message("Люблю го. Тг: synthetic_user")).unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
    assert_eq!(profile.contacts[0].ownership, Ownership::ProfileSignature);
}

#[test]
fn natural_goal_without_search_keyword_is_sufficient() {
    let profile = classify(message(
        "С удовольствием поговорю о книгах.<br>Почта: reader@example.invalid",
    ))
    .unwrap();
    assert_eq!(profile.contacts[0].kind, ContactKind::Email);
    assert_eq!(profile.contacts[0].value, "reader@example.invalid");
}

#[test]
fn multiple_contacts_have_types_and_evidence() {
    let profile = classify(message("Увлекаюсь астрономией.<br>Мой Telegram: synthetic_user<br>Мой Discord: star_reader<br>Почта: star@example.invalid")).unwrap();
    assert_eq!(
        profile.contacts.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [
            ContactKind::Telegram,
            ContactKind::Discord,
            ContactKind::Email
        ]
    );
    assert_eq!(profile.contacts[1].value, "star_reader");
    assert_eq!(profile.contacts[2].ownership, Ownership::ProfileSignature);
}

#[test]
fn description_without_contact_is_rejected() {
    assert_eq!(
        classify(message("Люблю книги, хочу обсудить фантастику.")).unwrap_err(),
        Rejection::NoOwnedContact
    );
}

#[test]
fn contact_only_and_invitation_are_not_profiles() {
    for text in [
        "@synthetic_user",
        "Пишите сюда: Telegram: synthetic_user",
        "Вот мой Telegram: synthetic_user",
        "Почта: reader@example.invalid",
    ] {
        assert_eq!(
            classify(message(text)).unwrap_err(),
            Rejection::ContactOnly,
            "{text}"
        );
    }
}

#[test]
fn forgotten_contact_is_not_combined_with_previous_description() {
    assert!(classify(message("Люблю шахматы.")).is_err());
    assert_eq!(
        classify(message("Забыл контакт, вот @synthetic_user")).unwrap_err(),
        Rejection::ContactOnly
    );
}

#[test]
fn quoted_contacts_and_descriptions_do_not_count_as_authored() {
    for text in [
        "<blockquote>Люблю шахматы.<br>Тг: synthetic_user</blockquote>",
        "<span class='unkfunc'>&gt;Люблю шахматы. Тг: synthetic_user</span>",
        "&gt; Люблю шахматы. Тг: synthetic_user",
        "<q>Люблю шахматы</q><br>Мой Telegram: synthetic_user",
        "«Люблю шахматы. Тг: synthetic_user»",
        "\"Люблю шахматы. Тг: synthetic_user\"",
        "`Люблю шахматы. Тг: synthetic_user`",
        "<pre>Люблю шахматы. Тг: synthetic_user</pre>",
        "<code>Люблю шахматы. Тг: synthetic_user</code>",
    ] {
        assert!(classify(message(text)).is_err(), "{text}");
    }
    assert_eq!(
        classify(message(
            "Люблю шахматы.<br><blockquote>Тг: synthetic_user</blockquote>"
        ))
        .unwrap_err(),
        Rejection::NoOwnedContact
    );
}

#[test]
fn independent_profile_can_follow_quote_without_borrowing_contact() {
    let profile = classify(message(
        "<blockquote>Почта: foreign@example.invalid</blockquote>Люблю го.<br>Тг: synthetic_user",
    ))
    .unwrap();
    assert_eq!(profile.contacts.len(), 1);
    assert_eq!(profile.contacts[0].kind, ContactKind::Telegram);
}

#[test]
fn unrelated_words_do_not_decide_classification() {
    for text in [
        "Редактировал шаблон анкеты. Тг: synthetic_user",
        "Предлагаю услуги. Мой Тг: synthetic_user",
        "Чужой сервис продаю. Тг: synthetic_user",
    ] {
        assert_eq!(
            classify(message(text)).unwrap().contacts[0].value,
            "@synthetic_user"
        );
    }
    assert_eq!(
        classify(message(
            "Произвольная заметка. Мой канал: https://t.me/synthetic_channel"
        ))
        .unwrap_err(),
        Rejection::ForeignContact
    );
}
#[test]
fn introductory_post_is_never_a_profile() {
    let source = source();
    let messages = normalize(
        &source,
        &serde_json::to_vec(&envelope(vec![
            post(100, "Люблю шахматы. Мой Тг: synthetic_user"),
            post(101, "Обычный пост"),
        ]))
        .unwrap(),
    )
    .unwrap();
    let input = messages.into_iter().next().unwrap();
    assert!(input.is_introductory());
    assert_eq!(classify(input).unwrap_err(), Rejection::Introductory);
}

#[test]
fn ambiguous_mentions_are_not_owned_contacts() {
    for text in [
        "Люблю книги, читаю @synthetic_user",
        "Люблю игры, смотрю https://t.me/synthetic_user",
        "Люблю почту, например reader@example.invalid",
        "Люблю книги. Его Тг: synthetic_user",
        "Люблю книги. Контакт друга: reader@example.invalid",
    ] {
        assert!(classify(message(text)).is_err(), "{text}");
    }
}

#[test]
fn foreign_contact_qualifiers_require_a_word_boundary() {
    let profile = classify(message(
        "Люблю шахматы и хочу что-то лучшее — Тг: synthetic_user",
    ))
    .unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn contact_looking_like_description_does_not_supply_content() {
    assert_eq!(
        classify(message("Тг: love_books")).unwrap_err(),
        Rejection::ContactOnly
    );
}

#[test]
fn formatted_contact_link_is_recognized_without_html() {
    let profile = classify(message("<p>Люблю <b>шахматы</b> &amp; го.</p><a href='https://t.me/synthetic_user'>Мой Telegram</a>")).unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
    assert!(!profile.message.text().contains("<a"));
}

#[test]
fn labelled_telegram_url_is_not_truncated_to_https_as_username() {
    let profile = classify(message(
        "Люблю го.<br>Telegram: https://t.me/synthetic_user",
    ))
    .unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn contact_urls_are_case_insensitive_and_bound_to_the_host() {
    let telegram = classify(message(
        "Люблю го.<br>Telegram: HTTPS://T.ME/Synthetic_User",
    ))
    .unwrap();
    assert_eq!(telegram.contacts[0].kind, ContactKind::Telegram);
    assert_eq!(telegram.contacts[0].value, "@synthetic_user");

    let vk = classify(message("Люблю книги.<br>Vk.com/id12345")).unwrap();
    assert_eq!(vk.contacts[0].kind, ContactKind::Vk);
    assert_eq!(vk.contacts[0].value, "id12345");

    assert_eq!(
        classify(message("Люблю го.<br>https://great.me/something")).unwrap_err(),
        Rejection::NoOwnedContact
    );
}

#[test]
fn contact_in_parentheses_keeps_its_own_identifier() {
    let profile = classify(message("Люблю го (тг: synthetic_user)")).unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn resource_links_and_invalid_contact_suffixes_are_rejected() {
    for text in [
        "Люблю го.<br>Telegram: https://example.invalid/user",
        "Люблю го.<br>Discord: https://discord.gg/synthetic_invite",
        "Люблю го.<br>https://t.me/joinchat/abcdef",
        "Люблю го.<br>https://t.me/synthetic_user/123",
        "Люблю го.<br>Discord: abcdefghijklmnopqrstuvwxyz0123456789",
    ] {
        assert!(classify(message(text)).is_err(), "{text}");
    }
}

#[test]
fn telegram_usernames_are_not_rejected_by_a_bot_suffix_alone() {
    let profile = classify(message("Люблю го.<br>Telegram: synthetic_bot")).unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_bot");
}

#[test]
fn reserved_vk_group_prefixes_are_case_insensitive() {
    for value in ["club12345", "Club12345", "PUBLIC12345"] {
        assert_eq!(
            classify(message(&format!("Люблю книги.<br>VK: {value}"))).unwrap_err(),
            Rejection::UnresolvedContact,
            "{value}"
        );
    }
}

#[test]
fn single_quoted_contacts_and_descriptions_are_not_author_evidence() {
    for input in [
        "Люблю шахматы. 'Мой Telegram: synthetic_user'",
        "'Люблю шахматы.' Мой Telegram: synthetic_user",
        "Люблю шахматы. ‘Мой Telegram: synthetic_user’",
        "Люблю шахматы. 'Тг: quoted_one' 'Мой Тг: quoted_two'",
    ] {
        assert!(classify(message(input)).is_err(), "{input}");
    }
    let profile = classify(message(
        "I love O'Reilly books and don't mind long talks.<br>Telegram: synthetic_user",
    ))
    .unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn unmatched_single_quote_keeps_the_rest_of_an_authored_profile() {
    let profile = classify(message("Люблю '90-е. Тг: synthetic_user")).unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn bare_signature_assumes_telegram_and_keeps_evidence() {
    let profile = classify(message("Произвольная заметка.<br>@synthetic_user")).unwrap();
    assert_eq!(profile.contacts[0].kind, ContactKind::Telegram);
    assert!(profile.contacts[0].assumed_platform);
    assert!(classify(message("Произвольная заметка, читаю @synthetic_user")).is_err());
    assert_eq!(
        classify(message(
            "Произвольная заметка.<br>Telegram: @synthetic_user"
        ))
        .unwrap()
        .contacts[0]
            .kind,
        ContactKind::Telegram
    );
}
#[test]
fn hidden_http_telegram_link_remains_a_contact() {
    let profile = classify(message(
        "Люблю го.<br><a href='http://t.me/synthetic_user'>Мой Telegram</a>",
    ))
    .unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn hidden_contact_links_accept_mixed_case_hosts_and_http_vk() {
    let telegram = classify(message(
        "Люблю го.<br><a href='HTTPS://T.ME/Synthetic_User'>ссылка</a>",
    ))
    .unwrap();
    assert_eq!(telegram.contacts[0].value, "@synthetic_user");

    let vk = classify(message(
        "Люблю книги.<br><a href='HTTP://VK.COM/id12345'>профиль</a>",
    ))
    .unwrap();
    assert_eq!(vk.contacts[0].kind, ContactKind::Vk);
    assert_eq!(vk.contacts[0].value, "id12345");
}

#[test]
fn hidden_contact_links_ignore_tracking_query_and_fragment() {
    let telegram = classify(message(
        "Люблю го.<br><a href='https://t.me/synthetic_user?start=private#section'>ссылка</a>",
    ))
    .unwrap();
    assert_eq!(telegram.contacts[0].value, "@synthetic_user");

    let vk = classify(message(
        "Люблю книги.<br><a href='https://vk.com/id12345?from=private#profile'>профиль</a>",
    ))
    .unwrap();
    assert_eq!(vk.contacts[0].kind, ContactKind::Vk);
    assert_eq!(vk.contacts[0].value, "id12345");
}

#[test]
fn canonical_contacts_are_all_preserved_and_deduplicated() {
    let p = classify(message("Произвольный текст.<br>Тг: Mixed_User и https://t.me/mixed_user; VK: id12345; Почта: Reader@EXAMPLE.invalid; Discord: reader.name")).unwrap();
    assert_eq!(
        p.contacts
            .iter()
            .map(|c| c.value.as_str())
            .collect::<Vec<_>>(),
        [
            "@mixed_user",
            "id12345",
            "Reader@example.invalid",
            "reader.name"
        ]
    );
}

#[test]
fn local_restoration_operations_precede_validation() {
    for (html, expected) in [
        (
            "Заметка.<br>Тг: syn_thetic (без нижнего подчёркивания)",
            "@synthetic",
        ),
        (
            "Заметка.<br>Тг: syn0(the)tic<br>(Уберите нули и скобки)",
            "@synthetic",
        ),
        ("Заметка.<br>Тг: synthe tic (убери пробел)", "@synthetic"),
        (
            "Заметка.<br>Тг: synthetic<br>замени syn на fan",
            "@fanthetic",
        ),
    ] {
        assert_eq!(classify(message(html)).unwrap().contacts[0].value, expected);
    }
}

#[test]
fn geography_and_unsupported_instructions_do_not_invent_contacts() {
    assert!(classify(message("ДС-2. Просто текст без контакта.")).is_err());
    assert!(
        classify(message(
            "Заметка.<br>Тг: synthetic<br>убери непонятный символ"
        ))
        .is_err()
    );
}

#[test]
fn colloquial_discord_labels_are_not_geography() {
    for label in ["дс:", "дискордик", "диск"] {
        let p = classify(message(&format!("Заметка.<br>{label} synthetic_user"))).unwrap();
        assert_eq!(p.contacts[0].kind, ContactKind::Discord);
        assert_eq!(p.contacts[0].value, "synthetic_user");
    }
}

#[test]
fn snapshot_restoration_shapes_keep_unknown_platform_explicit() {
    for (html, expected) in [
        (
            "Заметка.<br>spare9 5 (убери пробел после девятки)",
            "spare95",
        ),
        (
            "Заметка.<br>@syn0(000)thetic<br>(Уберите нули и скобки)",
            "@synthetic",
        ),
        (
            "Заметка.<br><span class='spoiler'>spare_user Без нижнего подчеркивания</span>",
            "spareuser",
        ),
    ] {
        let p = classify(message(html)).unwrap();
        assert_eq!(
            p.contacts[0].kind,
            if html.contains("@") {
                ContactKind::Telegram
            } else {
                ContactKind::Unknown
            }
        );
        assert_eq!(p.contacts[0].value, expected);
    }
    let p = classify(message("Заметка.<br>synthetic_user дискорд")).unwrap();
    assert_eq!(p.contacts[0].kind, ContactKind::Discord);
}

#[test]
fn literal_instructions_do_not_truncate_invalid_identifiers() {
    let p = classify(message("Заметка.<br>Тг: synthe+tic (убери символ '+')")).unwrap();
    assert_eq!(p.contacts[0].value, "@synthetic");
    let p = classify(message("Заметка.<br>Тг: syn_thetic (замени '_' на 'x')")).unwrap();
    assert_eq!(p.contacts[0].value, "@synxthetic");
    assert!(classify(message("Заметка.<br>Тг: synthe+tic (без пробелов)")).is_err());
    assert!(classify(message("Заметка.<br>@syntheticчужой")).is_err());
    assert!(classify(message("Цитирую: заметка. Тг: synthetic_user")).is_err());
}

#[test]
fn adjacent_contacts_keep_platform_and_independent_values() {
    let p = classify(message("Заметка.<br>Тг: @synthetic_one, @synthetic_two; Почта: a@example.invalid, b@example.invalid")).unwrap();
    assert_eq!(
        p.contacts
            .iter()
            .map(|c| c.value.as_str())
            .collect::<Vec<_>>(),
        [
            "@synthetic_one",
            "@synthetic_two",
            "a@example.invalid",
            "b@example.invalid"
        ]
    );
    assert_eq!(p.contacts[1].kind, ContactKind::Telegram);
    let p = classify(message("Заметка.<br>@synthetic_one, @synthetic_two")).unwrap();
    assert_eq!(p.contacts.len(), 2);
    assert!(
        p.contacts
            .iter()
            .all(|c| c.kind == ContactKind::Telegram && c.assumed_platform)
    );
}

#[test]
fn contact_anchor_caption_does_not_supply_independent_text() {
    assert!(
        classify(message(
            "<a href='https://t.me/synthetic_user'>Написать мне</a>"
        ))
        .is_err()
    );
    let p = classify(message(
        "Заметка.<br><a href='https://vk.ru/id12345'>Написать мне</a>",
    ))
    .unwrap();
    assert_eq!(p.contacts[0].kind, ContactKind::Vk);
    assert_eq!(p.contacts[0].value, "id12345");
    assert!(p.message.text().contains("Написать мне"));
}

#[test]
fn discord_case_and_legal_trailing_dot_are_preserved_by_contract() {
    let p = classify(message("Заметка.<br>Discord: Synthetic.Name.")).unwrap();
    assert_eq!(p.contacts[0].value, "synthetic.name.");
    let p = classify(message("Заметка.<br>Discord: Synthetic#1234")).unwrap();
    assert_eq!(p.contacts[0].value, "Synthetic#1234");
}

#[test]
fn multiline_quotes_cannot_become_owned_contacts() {
    for text in [
        "«Чужая заметка.<br>Тг: synthetic_user<br>Конец цитаты»",
        "'Чужая заметка.<br>Тг: synthetic_user<br>Конец цитаты'",
        "Чужая заметка.<blockquote>Тг: synthetic_user<br>убери символ '_'</blockquote>",
    ] {
        assert!(classify(message(text)).is_err());
    }
}

#[test]
fn email_restoration_keeps_literal_case() {
    let p = classify(message(
        "Заметка.<br>Email: LoXal@example.invalid (замени X на C)",
    ))
    .unwrap();
    assert_eq!(p.contacts[0].value, "LoCal@example.invalid");
}

#[test]
fn contact_lists_and_mailto_without_independent_text_are_rejected() {
    for text in [
        "mailto:reader@example.invalid",
        "Тг: @synthetic_one и @synthetic_two",
        "Почта: a@example.invalid или b@example.invalid",
    ] {
        assert_eq!(classify(message(text)).unwrap_err(), Rejection::ContactOnly);
    }
}

#[test]
fn leading_example_and_private_message_invitation_are_not_profile_text() {
    assert_eq!(
        classify(message(
            "Вот пример анкеты: Люблю шахматы. Telegram: synthetic_user"
        ))
        .unwrap_err(),
        Rejection::ForeignContact
    );
    assert_eq!(
        classify(message("Пишите в ЛС: Telegram: synthetic_user")).unwrap_err(),
        Rejection::ContactOnly
    );
}

#[test]
fn ascii_description_can_precede_a_contact_on_the_same_line() {
    let profile = classify(message("I enjoy astronomy Telegram: synthetic_user")).unwrap();
    assert_eq!(profile.contacts[0].value, "@synthetic_user");
}

#[test]
fn invalid_and_foreign_contact_records_do_not_supply_independent_text() {
    for text in [
        "Telegram: synthetic_user, VK: Club12345",
        "Telegram: synthetic_user, Его Тг: foreign_user",
    ] {
        assert_eq!(
            classify(message(text)).unwrap_err(),
            Rejection::ContactOnly,
            "{text}"
        );
    }
}

#[test]
fn many_adjacent_contacts_are_all_preserved() {
    let mut text = String::from("Люблю го.<br>Мой Telegram: synthetic0000");
    for index in 1..1_000 {
        text.push_str(&format!(", @synthetic{index:04}"));
    }
    let profile = classify(message(&text)).unwrap();
    assert_eq!(profile.contacts.len(), 1_000);
    assert!(
        profile
            .contacts
            .iter()
            .all(|contact| contact.ownership == Ownership::ExplicitSelf)
    );
}
