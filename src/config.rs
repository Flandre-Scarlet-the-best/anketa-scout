use crate::{Failure, source::ThreadUrl};
use directories::BaseDirs;
use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const DEFAULT_CONFIG: &str = include_str!("../config.example.toml");
pub const MAX_CONFIG: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    sources: Vec<String>,
}

pub struct LoadedConfig {
    pub path: PathBuf,
    pub source: ThreadUrl,
}

pub fn default_path() -> Result<PathBuf, Failure> {
    BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("anketa-scout").join("config.toml"))
        .ok_or(Failure::Config(
            "Не удалось определить пользовательский каталог конфигурации; используйте --config.",
        ))
}

pub fn load(explicit: Option<&Path>, default: &Path) -> Result<LoadedConfig, Failure> {
    let path = explicit.unwrap_or(default);
    if explicit.is_none() {
        match File::open(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_default(path)?,
            Err(_) => return Err(Failure::Config("Нет доступа к пользовательскому конфигу.")),
        }
    }
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Failure::Config("Указанный файл конфигурации отсутствует; проверьте --config.")
        } else {
            Failure::Config("Не удалось открыть конфигурацию: проверьте путь и права доступа.")
        }
    })?;
    let mut content = String::new();
    file.take(MAX_CONFIG + 1)
        .read_to_string(&mut content)
        .map_err(|_| Failure::Config("Не удалось прочитать конфигурацию как UTF-8."))?;
    if content.len() as u64 > MAX_CONFIG {
        return Err(Failure::Config("Файл конфигурации превышает 64 КиБ."));
    }
    let config: Config = toml::from_str(&content).map_err(|_| {
        Failure::Config("Некорректный TOML: ожидается только sources — список URL.")
    })?;
    if config.sources.len() != 1 {
        return Err(Failure::Config(
            "В sources должен быть ровно один тред; пустой список и несколько источников пока не поддерживаются.",
        ));
    }
    Ok(LoadedConfig {
        path: path.to_path_buf(),
        source: ThreadUrl::parse(&config.sources[0])?,
    })
}

fn create_default(path: &Path) -> Result<(), Failure> {
    let error = Failure::Config(
        "Не удалось создать пользовательский config.toml; проверьте каталог и права или используйте --config.",
    );
    let parent = path.parent().ok_or(error)?;
    fs::create_dir_all(parent).map_err(|_| error)?;
    // Готовый файл появляется атомарно; конкурирующий запуск не увидит пустой TOML.
    let mut pending = tempfile::NamedTempFile::new_in(parent).map_err(|_| error)?;
    pending
        .write_all(DEFAULT_CONFIG.as_bytes())
        .map_err(|_| error)?;
    pending.as_file().sync_all().map_err(|_| error)?;
    match pending.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(failure) if failure.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(error),
    }
}
