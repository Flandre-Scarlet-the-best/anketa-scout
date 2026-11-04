use crate::{
    Failure,
    domain::{Message, Profile, classify},
};
use std::future::Future;
use tokio_util::sync::CancellationToken;

pub struct Report {
    pub source: String,
    pub processed: usize,
    pub profiles: Vec<Profile>,
}

pub async fn run(
    source: String,
    cancellation: &CancellationToken,
    receive: impl Future<Output = Result<Vec<Message>, Failure>>,
) -> Result<Report, Failure> {
    let messages = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(Failure::Cancelled),
        messages = receive => messages?,
    };
    let processed = messages.len();
    let mut profiles = Vec::new();
    for message in messages {
        if cancellation.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        if let Ok(profile) = classify(message) {
            profiles.push(profile);
        }
    }
    Ok(Report {
        source,
        processed,
        profiles,
    })
}
