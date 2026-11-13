// SPDX-License-Identifier: MIT
use crate::config::Config;
use crate::util;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};
use std::error::Error;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Nawet przy agresywnej retencji zostawiamy siedem najnowszych kopii.
/// Pozwala to odtworzyć bazę po kilku kolejnych nieudanych dniach.
pub const MIN_KEEP: usize = 7;

const BACKUP_PREFIX: &str = "zsl-order-";
const BACKUP_SUFFIX: &str = ".db";
const TEMP_PREFIX: &str = ".zsl-order-backup-";
const TEMP_SUFFIX: &str = ".tmp";
const MAX_KEEP_DAYS: i64 = 3650;

#[derive(Debug)]
struct Options {
    keep_days: Option<i64>,
    verify: Option<PathBuf>,
    help: bool,
}

pub async fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let options = parse_args(args)?;
    if options.help {
        print_help();
        return Ok(());
    }
    if let Some(path) = options.verify {
        return verify(&path).await;
    }

    let config = Config::load_or_create(crate::CONF_PATH)?;
    create(&config, options.keep_days).await
}

fn parse_args(args: &[String]) -> Result<Options, Box<dyn Error>> {
    let mut keep_days = None;
    let mut verify = None;
    let mut help = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => help = true,
            "--keep-days" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or("Brak wartości po --keep-days. Użyj --help.")?;
                keep_days = Some(parse_keep_days(raw)?);
            }
            "--verify" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or("Brak ścieżki po --verify. Użyj --help.")?;
                verify = Some(PathBuf::from(raw));
            }
            other if other.starts_with("--keep-days=") => {
                let raw = &other["--keep-days=".len()..];
                keep_days = Some(parse_keep_days(raw)?);
            }
            other if other.starts_with("--verify=") => {
                verify = Some(PathBuf::from(&other["--verify=".len()..]));
            }
            other => {
                return Err(
                    format!("Nieznany argument polecenia backup: „{other}”. Użyj --help.").into(),
                );
            }
        }
        index += 1;
    }

    if verify.is_some() && keep_days.is_some() {
        return Err("Nie łącz --verify z --keep-days: weryfikacja nie tworzy kopii.".into());
    }
    if verify
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err("Ścieżka po --verify nie może być pusta.".into());
    }

    Ok(Options {
        keep_days,
        verify,
        help,
    })
}

fn parse_keep_days(raw: &str) -> Result<i64, Box<dyn Error>> {
    raw.trim()
        .parse::<i64>()
        .ok()
        .filter(|value| (0..=MAX_KEEP_DAYS).contains(value))
        .ok_or_else(|| {
            format!(
                "Nieprawidłowa wartość --keep-days „{raw}”. Podaj liczbę dni od 0 do {MAX_KEEP_DAYS}."
            )
            .into()
        })
}

async fn create(config: &Config, keep_days_override: Option<i64>) -> Result<(), Box<dyn Error>> {
    let db_path = database_path(config)?;
    if !db_path.is_file() {
        return Err(format!(
            "Nie znaleziono bazy danych „{}”. Sprawdź db_url w config.json.",
            db_path.display()
        )
        .into());
    }

    let backup_dir = PathBuf::from(&config.backup_dir);
    prepare_directory(&backup_dir)?;
    cleanup_stale_temps(&backup_dir);

    let mut source = SqliteConnectOptions::from_str(&config.sqlite_url())?
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(30))
        .connect()
        .await
        .map_err(|error| format!("Nie można otworzyć bazy „{}”: {error}", db_path.display()))?;

    let stamp = util::warsaw_now().format("%Y%m%d-%H%M%S").to_string();
    let final_path = unique_backup_path(&backup_dir, &stamp);
    let temp_path = backup_dir.join(format!(
        "{TEMP_PREFIX}{}-{}{TEMP_SUFFIX}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default()
    ));

    let vacuum = format!("VACUUM INTO '{}'", sqlite_literal(&temp_path));
    if let Err(error) = sqlx::query(&vacuum).execute(&mut source).await {
        let _ = source.close().await;
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Nie udało się utworzyć kopii przez VACUUM INTO: {error}").into());
    }
    source.close().await?;

    if let Err(error) = finalize_snapshot(&temp_path, &final_path).await {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    let size = fs::metadata(&final_path)?.len();
    println!("Kopia zapasowa: {}", final_path.display());
    println!("Rozmiar: {}", format_size(size));
    println!("Integralność: ok (PRAGMA integrity_check)");

    let keep_days = keep_days_override.unwrap_or(config.backup_keep_days);
    let removed = prune(&backup_dir, keep_days)?;
    if removed.is_empty() {
        println!(
            "Retencja: nic nie usunięto (zawsze zachowywane jest min. {MIN_KEEP} najnowszych kopii)."
        );
    } else {
        println!(
            "Retencja: usunięto {} kopii starszych niż {keep_days} dni (zachowano min. {MIN_KEEP} najnowszych).",
            removed.len()
        );
        for path in removed {
            println!("  usunięto: {}", path.display());
        }
    }

    Ok(())
}

fn database_path(config: &Config) -> Result<PathBuf, Box<dyn Error>> {
    let options = SqliteConnectOptions::from_str(&config.sqlite_url())?;
    let filename = options.get_filename();
    if filename == Path::new(":memory:") {
        return Err("Kopia zapasowa nie obsługuje bazy w pamięci (:memory:).".into());
    }
    Ok(filename.to_path_buf())
}

fn prepare_directory(path: &Path) -> Result<(), Box<dyn Error>> {
    let existed = path.exists();
    fs::create_dir_all(path)?;
    if !path.is_dir() {
        return Err(format!("„{}” nie jest katalogiem.", path.display()).into());
    }
    if !existed {
        set_mode(path, 0o700)?;
    }
    Ok(())
}

async fn finalize_snapshot(temp: &Path, final_path: &Path) -> Result<(), Box<dyn Error>> {
    set_mode(temp, 0o600)?;

    {
        let file = fs::File::open(temp)?;
        file.sync_all()?;
    }

    let problems = integrity_check(temp).await?;
    if !problems.is_empty() {
        return Err(format!(
            "Kopia nie przeszła PRAGMA integrity_check: {}",
            problems.join("; ")
        )
        .into());
    }

    fs::rename(temp, final_path)?;
    set_mode(final_path, 0o600)?;
    Ok(())
}

async fn verify(path: &Path) -> Result<(), Box<dyn Error>> {
    if !path.is_file() {
        return Err(format!("Nie znaleziono pliku kopii „{}”.", path.display()).into());
    }

    let problems = integrity_check(path).await?;
    if !problems.is_empty() {
        return Err(format!(
            "Kopia „{}” jest uszkodzona: {}",
            path.display(),
            problems.join("; ")
        )
        .into());
    }

    let size = fs::metadata(path)?.len();
    println!("OK: {}", path.display());
    println!("Rozmiar: {}", format_size(size));
    println!("Integralność: ok (PRAGMA integrity_check)");
    Ok(())
}

async fn integrity_check(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut connection = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true)
        .busy_timeout(Duration::from_secs(30))
        .connect()
        .await
        .map_err(|error| format!("Nie można otworzyć kopii „{}”: {error}", path.display()))?;

    let rows: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(&mut connection)
        .await
        .map_err(|error| format!("PRAGMA integrity_check nie powiodło się: {error}"))?;
    connection.close().await?;

    if rows.len() == 1 && rows[0].eq_ignore_ascii_case("ok") {
        Ok(Vec::new())
    } else if rows.is_empty() {
        Ok(vec!["brak wyniku integrity_check".to_string()])
    } else {
        Ok(rows)
    }
}

fn prune(dir: &Path, keep_days: i64) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut backups = list_backups(dir)?;
    backups.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.0.file_name().cmp(&left.0.file_name()))
    });

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(keep_days.max(0) as u64 * 86_400))
        .unwrap_or(UNIX_EPOCH);

    let mut removed = Vec::new();
    for (index, (path, modified)) in backups.into_iter().enumerate() {
        if index < MIN_KEEP || modified >= cutoff {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Nie można usunąć „{}”: {error}", path.display()).into());
            }
        }
    }
    Ok(removed)
}

fn list_backups(dir: &Path) -> Result<Vec<(PathBuf, SystemTime)>, Box<dyn Error>> {
    let mut backups = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(BACKUP_PREFIX) || !name.ends_with(BACKUP_SUFFIX) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        backups.push((entry.path(), modified));
    }
    Ok(backups)
}

fn cleanup_stale_temps(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(86_400))
        .unwrap_or(UNIX_EPOCH);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(TEMP_PREFIX) || !name.ends_with(TEMP_SUFFIX) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        if modified < cutoff {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn unique_backup_path(dir: &Path, stamp: &str) -> PathBuf {
    let first = dir.join(format!("{BACKUP_PREFIX}{stamp}{BACKUP_SUFFIX}"));
    if !first.exists() {
        return first;
    }
    for suffix in 1..10_000 {
        let candidate = dir.join(format!("{BACKUP_PREFIX}{stamp}-{suffix}{BACKUP_SUFFIX}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!(
        "{BACKUP_PREFIX}{stamp}-{}{BACKUP_SUFFIX}",
        std::process::id()
    ))
}

fn sqlite_literal(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn set_mode(path: &Path, mode: u32) -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const KIB: f64 = 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn print_help() {
    println!(
        "ZSL Order — kopie zapasowe bazy SQLite

Użycie:
  zsl-order backup                     utwórz kopię i zastosuj retencję
  zsl-order backup --keep-days DNI     nadpisz backup_keep_days z konfiguracji
  zsl-order backup --verify PLIK       sprawdź integralność istniejącej kopii
  zsl-order backup --help              pokaż tę pomoc

Kopia jest tworzona przez VACUUM INTO do pliku tymczasowego, sprawdzana
przez PRAGMA integrity_check, ustawiana na 0600 i atomowo przenoszona do
katalogu backup_dir. Nie kopiuj plików bazy przez cp przy działającej
aplikacji — WAL może zawierać niezapisane dane.

Konfiguracja:
  config.json: backup_dir (domyślnie backups), backup_keep_days (30)
  .env / środowisko: BACKUP_DIR, BACKUP_KEEP_DAYS

Retencja usuwa kopie starsze niż BACKUP_KEEP_DAYS, ale ZAWSZE zachowuje
co najmniej {MIN_KEEP} najnowszych. Wartość 0 oznacza „zostaw tylko
{MIN_KEEP} najnowszych”. Kopie zawierają hashe haseł i tokeny sesji —
traktuj katalog backup_dir jak dane wrażliwe."
    );
}
