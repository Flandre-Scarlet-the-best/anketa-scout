mod common;
use anketa_scout::domain::{ContactKind, Ownership, Rejection, classify};
use common::message;

#[test]
fn full_profile_preserves_own_contact_and_original() {
    let profile = classify(message(
        "Я программист. Люблю походы и настольные игры.<br>Мой Telegram: synthetic_user",
    ))
    .unwrap();
    assert_eq!(profile.contacts.len(), 1);
    assert_eq!(profile.contacts[0].kind, ContactKind::Telegram);
    assert_eq!(profile.contacts[0].value, "Telegram: synthetic_user");
    assert_eq!(profile.contacts[0].ownership, Ownership::ExplicitSelf);
    assert!(profile.message.original.contains("<br>"));
    assert!(!profile.message.text.contains("<br>"));
}

#[test]
fn short_profile_needs_no_demographics_or_at_sign() {
    let profile = classify(message("Люблю го. Тг: synthetic_user")).unwrap();
    assert_eq!(profile.contacts[0].value, "Тг: synthetic_user");
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
    assert_eq!(profile.contacts[1].value, "Discord: star_reader");
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
            Rejection::NoDescription,
            "{text}"
        );
    }
}

#[test]
fn forgotten_contact_is_not_combined_with_previous_description() {
    assert!(classify(message("Люблю шахматы.")).is_err());
    assert_eq!(
        classify(message("Забыл контакт, вот @synthetic_user")).unwrap_err(),
        Rejection::NoDescription
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
fn templates_advertising_and_reposts_are_rejected() {
    for text in [
        "Шаблон анкеты: люблю книги. Тг: synthetic_user",
        "Люблю игры. Предлагаю услуги. Мой Тг: synthetic_user",
        "Люблю книги. Мой канал: https://t.me/synthetic_channel",
        "Цитирую: люблю книги. Мой Тг: synthetic_user",
        "Список ресурсов. Люблю книги.<br>Тг: synthetic_user",
    ] {
        assert_eq!(
            classify(message(text)).unwrap_err(),
            Rejection::TemplateOrThirdParty,
            "{text}"
        );
    }
}

#[test]
fn introductory_post_is_never_a_profile() {
    let mut input = message("Люблю шахматы. Мой Тг: synthetic_user");
    input.introductory = true;
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
fn contact_looking_like_description_does_not_supply_content() {
    assert_eq!(
        classify(message("Тг: love_books")).unwrap_err(),
        Rejection::NoDescription
    );
}

#[test]
fn formatted_contact_link_is_recognized_without_html() {
    let profile = classify(message("<p>Люблю <b>шахматы</b> &amp; го.</p><a href='https://t.me/synthetic_user'>Мой Telegram</a>")).unwrap();
    assert_eq!(profile.contacts[0].value, "https://t.me/synthetic_user");
    assert!(!profile.message.text.contains("<a"));
}

#[test]
fn labelled_telegram_url_is_not_truncated_to_https_as_username() {
    let profile = classify(message(
        "Люблю го.<br>Telegram: https://t.me/synthetic_user",
    ))
    .unwrap();
    assert_eq!(
        profile.contacts[0].value,
        "Telegram: https://t.me/synthetic_user"
    );
}

#[test]
fn resource_links_bots_and_invalid_contact_suffixes_are_rejected() {
    for text in [
        "Люблю го.<br>Telegram: https://example.invalid/user",
        "Люблю го.<br>Discord: https://discord.gg/synthetic_invite",
        "Люблю го.<br>https://t.me/joinchat/abcdef",
        "Люблю го.<br>https://t.me/synthetic_user/123",
        "Люблю го.<br>https://t.me/synthetic_bot",
        "Люблю го.<br>Discord: abcdefghijklmnopqrstuvwxyz0123456789",
    ] {
        assert!(classify(message(text)).is_err(), "{text}");
    }
}
