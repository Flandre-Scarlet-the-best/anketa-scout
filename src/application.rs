use crate::{
    Failure,
    domain::{Message, Profile, Rejection, classify},
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
    crate::diagnostics::started();
    let messages = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(Failure::Cancelled),
        messages = receive => messages.inspect_err(|failure| crate::diagnostics::failed(*failure))?,
    };
    let source_prefix = format!("{source}#");
    if messages
        .iter()
        .any(|message| !message.origin.link.starts_with(&source_prefix))
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
