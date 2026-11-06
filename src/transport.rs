use crate::{
    Failure,
    source::{MAX_BODY, ThreadUrl},
};
use reqwest::{Client, ClientBuilder, StatusCode, redirect::Policy};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn network(error: reqwest::Error) -> Failure {
    if error.is_timeout() {
        Failure::Timeout
    } else {
        Failure::Network
    }
}

fn client_builder() -> ClientBuilder {
    Client::builder()
        .user_agent(concat!("anketa-scout/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(40))
        .redirect(Policy::none())
        .https_only(true)
}

pub async fn fetch(
    source: &ThreadUrl,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, Failure> {
    let client = client_builder().build().map_err(network)?;
    fetch_with_client(&client, source, None, cancellation, "https://2ch.org").await
}

pub async fn fetch_after(
    source: &ThreadUrl,
    cursor: u64,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, Failure> {
    let client = client_builder().build().map_err(network)?;
    fetch_with_client(
        &client,
        source,
        Some(cursor),
        cancellation,
        "https://2ch.org",
    )
    .await
}

async fn fetch_with_client(
    client: &Client,
    source: &ThreadUrl,
    cursor: Option<u64>,
    cancellation: &CancellationToken,
    base: &str,
) -> Result<Vec<u8>, Failure> {
    let endpoint = cursor.map_or_else(|| source.endpoint(), |cursor| source.after_endpoint(cursor));
    let path = endpoint
        .strip_prefix("https://2ch.org")
        .ok_or(Failure::Format)?;
    request(client, &format!("{base}{path}"), cancellation).await
}

async fn request(
    client: &Client,
    endpoint: &str,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, Failure> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(Failure::Cancelled),
        result = request_inner(client, endpoint) => result,
    }
}

async fn request_inner(client: &Client, endpoint: &str) -> Result<Vec<u8>, Failure> {
    let mut response = client.get(endpoint).send().await.map_err(network)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::PARTIAL_CONTENT => return Err(Failure::Partial),
        StatusCode::NOT_FOUND | StatusCode::GONE => return Err(Failure::Unavailable),
        StatusCode::TOO_MANY_REQUESTS => return Err(Failure::RateLimit),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => return Err(Failure::Timeout),
        StatusCode::UNAUTHORIZED
        | StatusCode::FORBIDDEN
        | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS => return Err(Failure::Access),
        status if status.is_redirection() => return Err(Failure::Access),
        status if status.is_server_error() => return Err(Failure::Network),
        _ => return Err(Failure::Format),
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(Failure::Limit);
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    if media_type.eq_ignore_ascii_case("text/html") {
        return Err(Failure::Access);
    }
    if !media_type.eq_ignore_ascii_case("application/json") {
        return Err(Failure::Format);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if chunk.len() > MAX_BODY - bytes.len() {
            return Err(Failure::Limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn serve(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            let _ = stream.write_all(&response);
        });
        (endpoint, handle)
    }

    fn test_client() -> Client {
        client_builder()
            .https_only(false)
            .no_proxy()
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn incremental_http_paths_cursor_failures_and_recovery() {
        use crate::{
            application::Cursor,
            source::{normalize, normalize_after},
        };
        use serde_json::json;
        let source = ThreadUrl::parse("https://2ch.org/test/res/100.html").unwrap();
        let post = |num, comment| json!({"num":num,"parent":if num == 100 {0}else{100},"comment":comment,"files":[]});
        let batch = |posts| json!({"result":1,"posts":posts}).to_string();
        let initial = json!({"board":{"id":"test"},"current_thread":100,"posts_count":2,"threads":[{"posts":[post(100,"Вступление"),post(101,"Заметка. ТГ: synthetic_one")]}]}).to_string();
        let replies = vec![
            (200, initial),
            (
                200,
                batch(vec![
                    post(101, "Заметка. ТГ: synthetic_one"),
                    post(102, "Заметка. ТГ: synthetic_two"),
                    post(103, "Без контакта"),
                ]),
            ),
            (429, "{}".into()),
            (404, "{}".into()),
            (200, "обрыв".into()),
            (
                200,
                batch(vec![
                    post(103, "Без контакта"),
                    post(104, "Заметка. ТГ: synthetic_three"),
                    post(105, "Заметка. ТГ: synthetic_four"),
                ]),
            ),
            (200, batch(vec![])),
        ];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut paths = Vec::new();
            for (index, (status, body)) in replies.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = [0; 4096];
                let n = stream.read(&mut bytes).unwrap();
                paths.push(
                    String::from_utf8_lossy(&bytes[..n])
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_owned(),
                );
                let length = body.len() + if index == 4 { 100 } else { 0 };
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").unwrap();
            }
            paths
        });
        let client = test_client();
        let cancel = CancellationToken::new();
        let mut cursor = Cursor::default();
        let bytes = fetch_with_client(&client, &source, None, &cancel, &base)
            .await
            .unwrap();
        let report = cursor
            .process(source.page(), &cancel, normalize(&source, &bytes).unwrap())
            .await
            .unwrap();
        assert_eq!(report.profiles.len(), 1);
        let mut accepted = vec![101];
        for expected in [
            Ok(1),
            Err(Failure::RateLimit),
            Err(Failure::Unavailable),
            Err(Failure::Network),
            Ok(2),
            Ok(0),
        ] {
            let before = cursor.value();
            let result = async {
                let bytes =
                    fetch_with_client(&client, &source, Some(cursor.value()), &cancel, &base)
                        .await?;
                let messages = normalize_after(&source, &bytes, cursor.value())?;
                cursor.process(source.page(), &cancel, messages).await
            }
            .await;
            match (result, expected) {
                (Ok(report), Ok(n)) => {
                    assert_eq!(report.profiles.len(), n);
                    accepted.extend(report.profiles.iter().map(|p| p.message.origin.post));
                }
                (Err(error), Err(expected)) => {
                    assert_eq!(error, expected);
                    assert_eq!(cursor.value(), before);
                }
                _ => panic!("Неожиданный результат опроса"),
            }
        }
        assert_eq!(accepted, vec![101, 102, 104, 105]);
        assert_eq!(cursor.value(), 105);
        let paths = server.join().unwrap();
        assert_eq!(
            paths,
            [
                "/test/res/100.json",
                "/api/mobile/v2/after/test/100/101",
                "/api/mobile/v2/after/test/100/103",
                "/api/mobile/v2/after/test/100/103",
                "/api/mobile/v2/after/test/100/103",
                "/api/mobile/v2/after/test/100/103",
                "/api/mobile/v2/after/test/100/105"
            ]
        );
    }

    #[tokio::test]
    async fn status_errors_and_redirects_keep_their_categories() {
        for (status, failure) in [
            (206, Failure::Partial),
            (401, Failure::Access),
            (403, Failure::Access),
            (451, Failure::Access),
            (404, Failure::Unavailable),
            (410, Failure::Unavailable),
            (429, Failure::RateLimit),
            (408, Failure::Timeout),
            (504, Failure::Timeout),
            (503, Failure::Network),
            (302, Failure::Access),
            (400, Failure::Format),
        ] {
            let response = format!(
                "HTTP/1.1 {status} Test\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let (endpoint, handle) = serve(response.into_bytes());
            assert_eq!(
                request(&test_client(), &endpoint, &CancellationToken::new())
                    .await
                    .unwrap_err(),
                failure,
                "{status}"
            );
            handle.join().unwrap();
        }
    }

    #[tokio::test]
    async fn valid_json_body_and_access_challenge_are_distinct() {
        for (mime, result) in [
            ("application/json; charset=utf-8", Ok(b"{}".to_vec())),
            ("Application/JSON; charset=utf-8", Ok(b"{}".to_vec())),
            ("text/html; charset=utf-8", Err(Failure::Access)),
            ("TEXT/HTML; charset=utf-8", Err(Failure::Access)),
            ("text/plain", Err(Failure::Format)),
        ] {
            let (endpoint, handle) = serve(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").into_bytes());
            assert_eq!(
                request(&test_client(), &endpoint, &CancellationToken::new()).await,
                result
            );
            handle.join().unwrap();
        }
    }

    #[tokio::test]
    async fn declared_and_streamed_body_size_limits_are_enforced() {
        let (endpoint, handle) = serve(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY + 1
            )
            .into_bytes(),
        );
        assert_eq!(
            request(&test_client(), &endpoint, &CancellationToken::new()).await,
            Err(Failure::Limit)
        );
        handle.join().unwrap();
        for length in [MAX_BODY, MAX_BODY + 1] {
            let mut response =
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                    .to_vec();
            response.resize(response.len() + length, b' ');
            let (endpoint, handle) = serve(response);
            let result = request(&test_client(), &endpoint, &CancellationToken::new()).await;
            if length == MAX_BODY {
                assert_eq!(result.unwrap().len(), MAX_BODY);
            } else {
                assert_eq!(result, Err(Failure::Limit));
            }
            handle.join().unwrap();
        }
    }

    #[tokio::test]
    async fn truncated_http_body_is_network_failure() {
        let (endpoint, handle) = serve(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec());
        assert_eq!(
            request(&test_client(), &endpoint, &CancellationToken::new()).await,
            Err(Failure::Network)
        );
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn timeout_is_bounded_and_is_not_empty_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let client = client_builder()
            .https_only(false)
            .no_proxy()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        assert_eq!(
            request(&client, &endpoint, &CancellationToken::new()).await,
            Err(Failure::Timeout)
        );
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn production_client_rejects_plain_http_before_connection() {
        let client = client_builder().no_proxy().build().unwrap();
        assert_eq!(
            request(&client, "http://127.0.0.1:1", &CancellationToken::new()).await,
            Err(Failure::Network)
        );
    }

    #[tokio::test]
    async fn stalled_response_body_is_a_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{}").unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let client = client_builder()
            .https_only(false)
            .no_proxy()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        assert_eq!(
            request(&client, &endpoint, &CancellationToken::new()).await,
            Err(Failure::Timeout)
        );
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn cancellation_stops_in_flight_request_and_closes_socket() {
        use std::io::BufRead;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started, ready) = tokio::sync::oneshot::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut stream = std::io::BufReader::new(stream);
            loop {
                let mut line = String::new();
                assert!(stream.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            stream.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{}").unwrap();
            started.send(()).unwrap();
            let mut buffer = [0; 1];
            match stream.read(&mut buffer) {
                Ok(0) => true,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    true
                }
                _ => false,
            }
        });
        let token = CancellationToken::new();
        let request_token = token.clone();
        let task =
            tokio::spawn(async move { request(&test_client(), &endpoint, &request_token).await });
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        token.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap(),
            Err(Failure::Cancelled)
        );
        assert!(server.join().unwrap());
    }
}
