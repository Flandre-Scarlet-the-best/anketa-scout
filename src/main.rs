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
    let report = application::run(config.source.page(), cancellation, async {
        let bytes = progress::fetching(
            stderr,
            diagnostic,
            transport::fetch(&config.source, cancellation),
        )
        .await?;
        source::normalize(&config.source, &bytes)
    })
    .await?;
    let render_cancellation = cancellation.clone();
    let output = spawn_output_thread(move || {
        let stdout = io::stdout();
        let writer = CancellableWriter {
            inner: stdout.lock(),
            cancellation: render_cancellation.clone(),
        };
        output_result(
            output::render_with_options(&report, writer, options),
            &render_cancellation,
        )
    })
    .map_err(|_| Failure::Output)?;
    output.await.map_err(|_| Failure::Output)?
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
