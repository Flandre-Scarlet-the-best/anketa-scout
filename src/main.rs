use anketa_scout::{Failure, application, config, output, source, transport};
use clap::{Parser, error::ErrorKind};
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
    #[arg(long, action = clap::ArgAction::Help, help = "Показать справку")]
    help: Option<bool>,
}

async fn execute(args: Args, cancellation: &CancellationToken) -> Result<(), Failure> {
    let default = if args.config.is_none() {
        config::default_path()?
    } else {
        PathBuf::new()
    };
    let config = config::load(args.config.as_deref(), &default)?;
    eprintln!(
        "Конфигурация: {}",
        output::safe(&config.path.display().to_string())
    );
    let report = application::run(config.source.page(), cancellation, async {
        source::normalize(
            &config.source,
            &transport::fetch(&config.source, cancellation).await?,
        )
    })
    .await?;
    match output::render(&report, io::stdout().lock()) {
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
                .replace("Print version", "Показать версию");
            return if io::stdout().write_all(text.as_bytes()).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(8)
            };
        }
        Err(_) => {
            eprintln!(
                "Некорректные аргументы. Использование: anketa-scout [--config <ПУТЬ>]. Справка: --help."
            );
            return ExitCode::from(2);
        }
    };
    let cancellation = CancellationToken::new();
    let result = tokio::select! {
        result = execute(args, &cancellation) => result,
        signal = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            match signal { Ok(()) => Err(Failure::Cancelled), Err(_) => Err(Failure::Signal) }
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Ошибка: {error}");
            ExitCode::from(error.code())
        }
    }
}
