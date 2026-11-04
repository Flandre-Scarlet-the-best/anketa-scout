use crate::{
    Failure,
    source::{MAX_BODY, ThreadUrl},
};
use reqwest::{
    StatusCode,
    blocking::{Client, ClientBuilder},
    redirect::Policy,
};
use std::{io::Read, time::Duration};

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

pub fn fetch(source: &ThreadUrl) -> Result<Vec<u8>, Failure> {
    let client = client_builder().build().map_err(network)?;
    request(&client, &source.endpoint())
}

fn request(client: &Client, endpoint: &str) -> Result<Vec<u8>, Failure> {
    let response = client.get(endpoint).send().map_err(network)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::PARTIAL_CONTENT => return Err(Failure::Partial),
        StatusCode::NOT_FOUND | StatusCode::GONE => return Err(Failure::Unavailable),
        StatusCode::TOO_MANY_REQUESTS => return Err(Failure::RateLimit),
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
    if content_type.starts_with("text/html") {
        return Err(Failure::Access);
    }
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim() == "application/json")
    {
        return Err(Failure::Format);
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut
                || error
                    .get_ref()
                    .and_then(|cause| cause.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout)
            {
                Failure::Timeout
            } else {
                Failure::Network
            }
        })?;
    if bytes.len() > MAX_BODY {
        return Err(Failure::Limit);
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

    #[test]
    fn status_errors_and_redirects_keep_their_categories() {
        for (status, failure) in [
            (206, Failure::Partial),
            (401, Failure::Access),
            (403, Failure::Access),
            (451, Failure::Access),
            (404, Failure::Unavailable),
            (410, Failure::Unavailable),
            (429, Failure::RateLimit),
            (503, Failure::Network),
            (302, Failure::Access),
            (400, Failure::Format),
        ] {
            let response = format!(
                "HTTP/1.1 {status} Test\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let (endpoint, handle) = serve(response.into_bytes());
            assert_eq!(
                request(&test_client(), &endpoint).unwrap_err(),
                failure,
                "{status}"
            );
            handle.join().unwrap();
        }
    }

    #[test]
    fn valid_json_body_and_access_challenge_are_distinct() {
        for (mime, result) in [
            ("application/json; charset=utf-8", Ok(b"{}".to_vec())),
            ("text/html; charset=utf-8", Err(Failure::Access)),
            ("text/plain", Err(Failure::Format)),
        ] {
            let (endpoint, handle) = serve(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").into_bytes());
            assert_eq!(request(&test_client(), &endpoint), result);
            handle.join().unwrap();
        }
    }

    #[test]
    fn declared_and_streamed_body_size_limits_are_enforced() {
        let (endpoint, handle) = serve(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY + 1
            )
            .into_bytes(),
        );
        assert_eq!(request(&test_client(), &endpoint), Err(Failure::Limit));
        handle.join().unwrap();
        for length in [MAX_BODY, MAX_BODY + 1] {
            let mut response =
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                    .to_vec();
            response.resize(response.len() + length, b' ');
            let (endpoint, handle) = serve(response);
            let result = request(&test_client(), &endpoint);
            if length == MAX_BODY {
                assert_eq!(result.unwrap().len(), MAX_BODY);
            } else {
                assert_eq!(result, Err(Failure::Limit));
            }
            handle.join().unwrap();
        }
    }

    #[test]
    fn truncated_http_body_is_network_failure() {
        let (endpoint, handle) = serve(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}".to_vec());
        assert_eq!(request(&test_client(), &endpoint), Err(Failure::Network));
        handle.join().unwrap();
    }

    #[test]
    fn timeout_is_bounded_and_is_not_empty_success() {
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
        assert_eq!(request(&client, &endpoint), Err(Failure::Timeout));
        handle.join().unwrap();
    }

    #[test]
    fn production_client_rejects_plain_http_before_connection() {
        let client = client_builder().no_proxy().build().unwrap();
        assert_eq!(
            request(&client, "http://127.0.0.1:1"),
            Err(Failure::Network)
        );
    }

    #[test]
    fn stalled_response_body_is_a_timeout() {
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
        assert_eq!(request(&client, &endpoint), Err(Failure::Timeout));
        handle.join().unwrap();
    }
}
