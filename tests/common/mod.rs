#![allow(dead_code)]
use anketa_scout::{
    domain::{Message, Origin},
    source::{ThreadUrl, normalize_html},
};
use serde_json::{Value, json};

pub fn source() -> ThreadUrl {
    ThreadUrl::parse("https://2ch.org/test/res/100.html").unwrap()
}

pub fn message(html: &str) -> Message {
    let (text, authored_text) = normalize_html(html).unwrap();
    Message {
        origin: Origin {
            source: "2ch.org".into(),
            board: "test".into(),
            thread: 100,
            post: 101,
            link: "https://2ch.org/test/res/100.html#101".into(),
        },
        original: html.into(),
        text,
        authored_text,
        date: None,
        attachments: vec![],
        introductory: false,
    }
}

pub fn post(num: u64, comment: &str) -> Value {
    json!({"num":num,"parent":if num == 100 {0}else{100},"comment":comment,"files":[]})
}

pub fn envelope(posts: Vec<Value>) -> Value {
    json!({"board":{"id":"test"},"current_thread":100,"posts_count":posts.len(),"threads":[{"posts":posts}]})
}
