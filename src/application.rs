use crate::{
    Failure,
    domain::{Message, Origin, Profile, Rejection, classify},
};
use std::future::Future;
use tokio_util::sync::CancellationToken;
use url::Url;

pub struct Report {
    pub source: String,
    pub processed: usize,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Copy, Default)]
pub struct Cursor(u64);

impl Cursor {
    pub fn value(&self) -> u64 {
        self.0
    }

    pub async fn process(
        &mut self,
        source: String,
        cancellation: &CancellationToken,
        messages: Vec<Message>,
    ) -> Result<Report, Failure> {
        self.process_with(source, cancellation, async { Ok(messages) })
            .await
    }

    pub async fn process_with(
        &mut self,
        source: String,
        cancellation: &CancellationToken,
        receive: impl Future<Output = Result<Vec<Message>, Failure>>,
    ) -> Result<Report, Failure> {
        crate::diagnostics::started();
        let result = async {
            let messages = receive_messages(cancellation, receive).await?;
            let next = messages
                .iter()
                .map(|message| message.origin.post)
                .max()
                .unwrap_or(self.0)
                .max(self.0);
            let messages = messages
                .into_iter()
                .filter(|message| message.origin.post > self.0)
                .collect();
            let report = process_messages(source, cancellation, messages).await?;
            self.0 = next;
            Ok(report)
        }
        .await;
        if let Err(failure) = result {
            crate::diagnostics::failed(failure);
        }
        result
    }
}

pub async fn run(
    source: String,
    cancellation: &CancellationToken,
    receive: impl Future<Output = Result<Vec<Message>, Failure>>,
) -> Result<Report, Failure> {
    crate::diagnostics::started();
    let result = async {
        let messages = receive_messages(cancellation, receive).await?;
        process_messages(source, cancellation, messages).await
    }
    .await;
    if let Err(failure) = result {
        crate::diagnostics::failed(failure);
    }
    result
}

async fn receive_messages(
    cancellation: &CancellationToken,
    receive: impl Future<Output = Result<Vec<Message>, Failure>>,
) -> Result<Vec<Message>, Failure> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(Failure::Cancelled),
        messages = receive => messages,
    }
}

async fn process_messages(
    source: String,
    cancellation: &CancellationToken,
    messages: Vec<Message>,
) -> Result<Report, Failure> {
    if messages
        .iter()
        .any(|message| !origin_matches_source(&message.origin, &source))
    {
        return Err(Failure::Format);
    }
    let processed = messages.len();
    let mut profiles = Vec::new();
    let mut reasons = [0; 5];
    for message in messages {
        if cancellation.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let post = message.origin.post;
        match classify(message) {
            Ok(profile) => {
                crate::diagnostics::decision(post, Ok(profile.contacts.len()));
                for contact in &profile.contacts {
                    crate::diagnostics::contact(post, contact.kind, &contact.restoration);
                }
                profiles.push(profile);
            }
            Err(reason) => {
                let index = match reason {
                    Rejection::Introductory => 0,
                    Rejection::ForeignContact => 1,
                    Rejection::ContactOnly => 2,
                    Rejection::NoOwnedContact => 3,
                    Rejection::UnresolvedContact => 4,
                };
                reasons[index] += 1;
                crate::diagnostics::decision(post, Err(reason));
            }
        }
        tokio::task::yield_now().await;
    }
    if cancellation.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    crate::diagnostics::completed(processed, profiles.len(), reasons);
    Ok(Report {
        source,
        processed,
        profiles,
    })
}

fn origin_matches_source(origin: &Origin, source: &str) -> bool {
    let Ok(source_url) = Url::parse(source) else {
        return false;
    };
    let expected_path = format!("/{}/res/{}.html", origin.board, origin.thread);
    if origin.board.is_empty()
        || !origin
            .board
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || origin.thread == 0
        || origin.post == 0
        || source_url.scheme() != "https"
        || source_url.host_str() != Some(origin.source.as_str())
        || source_url.port().is_some()
        || !source_url.username().is_empty()
        || source_url.password().is_some()
        || source_url.query().is_some()
        || source_url.fragment().is_some()
        || source_url.as_str() != source
        || source_url.path() != expected_path
    {
        return false;
    }
    origin.link == format!("{source}#{}", origin.post)
}
