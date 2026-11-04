pub mod application;
pub mod config;
pub mod domain;
pub mod output;
pub mod source;
pub mod transport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    Config(&'static str),
    Network,
    Timeout,
    Unavailable,
    Access,
    RateLimit,
    Format,
    Partial,
    Limit,
    Output,
}

impl Failure {
    pub fn code(self) -> u8 {
        match self {
            Self::Config(_) => 2,
            Self::Network | Self::Timeout => 3,
            Self::Unavailable | Self::Access | Self::RateLimit => 4,
            Self::Format => 5,
            Self::Partial => 6,
            Self::Limit => 7,
            Self::Output => 8,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Config(message) => message,
            Self::Network => "Сетевой сбой при получении треда.",
            Self::Timeout => "Истекло время ожидания источника.",
            Self::Unavailable => "Тред отсутствует или недоступен (HTTP 404/410).",
            Self::Access => {
                "Источник ограничил доступ или запросил проверку; получение не состоялось."
            }
            Self::RateLimit => "Источник ограничил частоту запросов (HTTP 429); повторите позже.",
            Self::Format => "Некорректный или неподдерживаемый формат ответа источника.",
            Self::Partial => "Получен неполный тред; результат не выдан как полный.",
            Self::Limit => "Превышено ограничение размера или количества входных данных.",
            Self::Output => "Не удалось записать результат в stdout.",
        })
    }
}

impl std::error::Error for Failure {}
