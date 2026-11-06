use anketa_scout::{Failure, application, config, output, progress, source, transport};
use clap::{Parser, error::ErrorKind};
use output::style::{ColorMode, Palette, Role, Terminal, Theme};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    version,
    about = "Поиск самостоятельных анкет с собственным контактом автора",
    disable_help_flag = true
)]
struct Args {
    #[arg(
        long,
        help = "Наблюдать новые анкеты через инкрементальный API; остановка: Ctrl+C"
    )]
    watch: bool,
    #[arg(long, default_value = "20s", value_parser = parse_interval, help = "Интервал только с --watch: 30s, 1m; от 1s до 24h; сам не включает наблюдение")]
    interval: std::time::Duration,
    #[arg(
        long,
        value_name = "ПУТЬ",
        help = "Явный TOML-файл; отсутствующий файл не создаётся"
    )]
    config: Option<PathBuf>,
    #[arg(
        long,
        value_enum,
        default_value = "full",
        help = "Полные анкеты или компактный обзор с превью"
    )]
    view: output::View,
    #[arg(
        long,
        value_enum,
        default_value = "auto",
        help = "Цвет: auto учитывает TTY и NO_COLOR; always имеет приоритет над NO_COLOR"
    )]
    color: ColorMode,
    #[arg(long, help = "Отключить цвет; имеет приоритет над --color и NO_COLOR")]
    no_color: bool,
    #[arg(
        long,
        value_enum,
        default_value = "dark",
        help = "Палитра для выбранного фона; автоматического определения фона нет"
    )]
    theme: Theme,
    #[arg(long, action = clap::ArgAction::Help, help = "Показать справку")]
    help: Option<bool>,
}

async fn execute(
    args: Args,
    cancellation: &CancellationToken,
    stderr: Terminal,
    diagnostic: Palette,
) -> Result<(), Failure> {
    let stdout = Terminal::detect(&console::Term::stdout());
    let options = output::Options {
        width: stdout.width,
        view: args.view,
        palette: Palette::resolve(args.color, args.no_color, args.theme, stdout),
    };
    let default = if args.config.is_none() {
        config::default_path()?
    } else {
        PathBuf::new()
    };
    let config = config::load(args.config.as_deref(), &default)?;
    let _ = writeln!(
        io::stderr().lock(),
        "{}",
        diagnostic.paint(
            Role::Metadata,
            &format!(
                "Конфигурация: {}",
                output::safe(&config.path.display().to_string())
            )
        )
    );
    let transport_session = transport::Session::new();
    let mut cursor = application::Cursor::default();
    let report = cursor
        .process_with(config.source.page(), cancellation, async {
            let transport = transport_session.as_ref().map_err(|failure| *failure)?;
            let bytes = progress::fetching(
                stderr,
                diagnostic,
                transport.fetch(&config.source, cancellation),
            )
            .await?;
            source::normalize_cancellable(&config.source, bytes, cancellation).await
        })
        .await?;
    let transport = transport_session?;
    if !render_report(report, options, cancellation, true).await? || !args.watch {
        return Ok(());
    }
    let _ = writeln!(
        io::stderr().lock(),
        "Наблюдение: интервал {} с; остановка Ctrl+C.",
        args.interval.as_secs()
    );
    let mut delay = args.interval;
    let mut retries = 0;
    loop {
        progress::waiting(stderr, diagnostic, tokio::time::sleep(delay)).await;
        let cursor_value = cursor.value();
        let mut next_cursor = cursor;
        let result = next_cursor
            .process_with(config.source.page(), cancellation, async {
                let bytes = progress::fetching(
                    stderr,
                    diagnostic,
                    transport.fetch_after(&config.source, cursor_value, cancellation),
                )
                .await?;
                source::normalize_after_cancellable(
                    &config.source,
                    bytes,
                    cursor_value,
                    cancellation,
                )
                .await
            })
            .await;
        match result {
            Ok(report) => {
                retries = 0;
                delay = args.interval;
                if !report.profiles.is_empty()
                    && !render_report(report, options, cancellation, false).await?
                {
                    return Ok(());
                }
                cursor = next_cursor;
            }
            Err(Failure::Cancelled) => return Err(Failure::Cancelled),
            Err(error) => {
                let Some(next_delay) = watch_retry_delay(error, retries, args.interval) else {
                    return Err(error);
                };
                retries += 1;
                delay = next_delay;
                let _ = writeln!(
                    io::stderr().lock(),
                    "Наблюдение: {error} Курсор сохранён; повтор через {} с.",
                    delay.as_secs()
                );
            }
        }
    }
}

const MAX_WATCH_RETRIES: u8 = 3;

fn watch_retry_delay(
    failure: Failure,
    retries: u8,
    interval: std::time::Duration,
) -> Option<std::time::Duration> {
    if !matches!(
        failure,
        Failure::Network | Failure::Timeout | Failure::RateLimit
    ) || retries >= MAX_WATCH_RETRIES
    {
        return None;
    }
    let factor = 1u32.checked_shl(u32::from(retries) + 1).unwrap_or(u32::MAX);
    Some(
        interval
            .saturating_mul(factor)
            .min(std::time::Duration::from_secs(3600))
            .max(interval),
    )
}

fn parse_interval(input: &str) -> Result<std::time::Duration, String> {
    let boundary = input.char_indices().last().map(|(i, _)| i).unwrap_or(0);
    let (number, unit) = input.split_at(boundary);
    let factor = match unit {
        "s" => 1u64,
        "m" => 60,
        "h" => 3600,
        _ => return Err("Нужна длительность: 30s, 1m или 1h".into()),
    };
    let seconds = number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(factor))
        .filter(|n| (1..=86400).contains(n))
        .ok_or_else(|| "Интервал должен быть от 1s до 24h".to_owned())?;
    Ok(std::time::Duration::from_secs(seconds))
}

async fn render_report(
    report: application::Report,
    options: output::Options,
    cancellation: &CancellationToken,
    summary: bool,
) -> Result<bool, Failure> {
    let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let check = closed.clone();
    let render_cancellation = cancellation.clone();
    let output = spawn_output_thread(move || {
        let stdout = io::stdout();
        let writer = CancellableWriter {
            inner: stdout.lock(),
            cancellation: render_cancellation.clone(),
        };
        let result = output::render_batch(&report, writer, options, summary);
        if result
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
        {
            closed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        output_result(result, &render_cancellation)
    })
    .map_err(|_| Failure::Output)?;
    output.await.map_err(|_| Failure::Output)??;
    Ok(!check.load(std::sync::atomic::Ordering::Relaxed))
}

fn spawn_output_thread(
    render: impl FnOnce() -> Result<(), Failure> + Send + 'static,
) -> io::Result<tokio::sync::oneshot::Receiver<Result<(), Failure>>> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("anketa-scout-output".into())
        .spawn(move || {
            let _ = sender.send(render());
        })?;
    Ok(receiver)
}

struct CancellableWriter<W> {
    inner: W,
    cancellation: CancellationToken,
}

impl<W: Write> Write for CancellableWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::other("Вывод отменён"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::other("Вывод отменён"));
        }
        self.inner.flush()
    }
}

fn output_result(result: io::Result<()>, cancellation: &CancellationToken) -> Result<(), Failure> {
    if cancellation.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(_) => Err(Failure::Output),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let text = error
                .to_string()
                .replace("Usage:", "Использование:")
                .replace("Options:", "Параметры:")
                .replace("[default:", "[по умолчанию:")
                .replace("[possible values:", "[значения:")
                .replace("Print version", "Показать версию");
            return if io::stdout().write_all(text.as_bytes()).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(8)
            };
        }
        Err(_) => {
            let _ = writeln!(
                io::stderr().lock(),
                "Некорректные аргументы. Использование: anketa-scout [--config <ПУТЬ>]. Справка: --help."
            );
            return ExitCode::from(2);
        }
    };
    let cancellation = CancellationToken::new();
    let stderr = Terminal::detect(&console::Term::stderr());
    let diagnostic = Palette::resolve(args.color, args.no_color, args.theme, stderr);
    let result = tokio::select! {
        result = execute(args, &cancellation, stderr, diagnostic) => result,
        signal = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            match signal { Ok(()) => Err(Failure::Cancelled), Err(_) => Err(Failure::Signal) }
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                diagnostic.paint(Role::Error, &format!("Ошибка: {error}"))
            );
            ExitCode::from(error.code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_validation_and_watch_are_independent() {
        for (input, seconds) in [("20s", 20), ("30s", 30), ("1m", 60), ("24h", 86400)] {
            assert_eq!(parse_interval(input).unwrap().as_secs(), seconds);
        }
        for input in [
            "",
            "я",
            "0s",
            "-1s",
            "5",
            "0m",
            "25h",
            "9999999999999999999999h",
        ] {
            assert!(parse_interval(input).is_err());
        }
        let args = Args::try_parse_from(["anketa-scout", "--interval", "1m"]).unwrap();
        assert!(!args.watch);
        assert_eq!(args.interval.as_secs(), 60);
        let args = Args::try_parse_from(["anketa-scout", "--watch"]).unwrap();
        assert!(args.watch);
        assert_eq!(args.interval.as_secs(), 20);
    }

    #[test]
    fn watch_retry_policy_is_transient_only_and_has_a_finite_attempt_limit() {
        let interval = std::time::Duration::from_secs(20);
        assert_eq!(
            watch_retry_delay(Failure::Network, 0, interval),
            Some(std::time::Duration::from_secs(40))
        );
        assert_eq!(
            watch_retry_delay(Failure::RateLimit, 1, interval),
            Some(std::time::Duration::from_secs(80))
        );
        assert_eq!(
            watch_retry_delay(Failure::Timeout, 2, interval),
            Some(std::time::Duration::from_secs(160))
        );
        assert_eq!(watch_retry_delay(Failure::Network, 3, interval), None);
        for failure in [
            Failure::Unavailable,
            Failure::Access,
            Failure::Format,
            Failure::Partial,
            Failure::Limit,
            Failure::Output,
        ] {
            assert_eq!(watch_retry_delay(failure, 0, interval), None);
        }
    }

    #[test]
    fn broken_pipe_is_success_and_other_output_failures_keep_code_eight() {
        let cancellation = CancellationToken::new();
        assert!(output_result(Err(io::ErrorKind::BrokenPipe.into()), &cancellation).is_ok());
        assert_eq!(
            output_result(Err(io::ErrorKind::PermissionDenied.into()), &cancellation),
            Err(Failure::Output)
        );
        assert_eq!(Failure::Output.code(), 8);
        assert_eq!(Failure::Cancelled.code(), 130);
    }

    #[test]
    fn cancelled_writer_stops_rendering() {
        let cancellation = CancellationToken::new();
        let mut writer = CancellableWriter {
            inner: Vec::new(),
            cancellation: cancellation.clone(),
        };
        cancellation.cancel();
        let error = writer.write_all(b"result").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            output_result(Err(error), &cancellation),
            Err(Failure::Cancelled)
        );
    }

    #[test]
    fn cancellation_takes_priority_over_broken_pipe() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            output_result(Err(io::ErrorKind::BrokenPipe.into()), &cancellation),
            Err(Failure::Cancelled)
        );
    }

    #[tokio::test]
    async fn cancelled_output_does_not_wait_for_a_blocked_writer_thread() {
        let (release, blocked) = std::sync::mpsc::channel();
        let output = spawn_output_thread(move || {
            blocked.recv().unwrap();
            Ok(())
        })
        .unwrap();
        let cancellation = CancellationToken::new();
        let signal = cancellation.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            signal.cancel();
        });

        let result = tokio::select! {
            result = output => Some(result.unwrap()),
            _ = cancellation.cancelled() => None,
        };
        assert_eq!(result, None);
        release.send(()).unwrap();
    }
}
