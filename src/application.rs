use crate::{
    Failure,
    domain::{Message, Profile, classify},
};

pub struct Report {
    pub source: String,
    pub processed: usize,
    pub profiles: Vec<Profile>,
}

pub fn run(
    source: String,
    receive: impl FnOnce() -> Result<Vec<Message>, Failure>,
) -> Result<Report, Failure> {
    let messages = receive()?;
    Ok(Report {
        source,
        processed: messages.len(),
        profiles: messages
            .into_iter()
            .filter_map(|message| classify(message).ok())
            .collect(),
    })
}
