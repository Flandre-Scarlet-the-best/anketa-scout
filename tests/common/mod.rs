#![allow(dead_code)]
use anketa_scout::{
    domain::Message,
    source::{ThreadUrl, normalize},
};
use serde_json::{Value, json};

pub fn source() -> ThreadUrl {
    ThreadUrl::parse("https://2ch.org/test/res/100.html").unwrap()
}

pub fn message(html: &str) -> Message {
    normalize(
        &source(),
        &serde_json::to_vec(&envelope(vec![post(100, "Вступление"), post(101, html)])).unwrap(),
    )
    .unwrap()
    .pop()
    .unwrap()
}

pub fn post(num: u64, comment: &str) -> Value {
    json!({"num":num,"parent":if num == 100 {0}else{100},"comment":comment,"files":[]})
}

pub fn envelope(posts: Vec<Value>) -> Value {
    json!({"board":{"id":"test"},"current_thread":100,"posts_count":posts.len(),"threads":[{"posts":posts}]})
}
