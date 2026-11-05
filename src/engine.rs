use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WORKER_ARG: &str = "--qdcord-worker";

// пределы проверяются в движке, а не только в окне
pub const MAX_VARIETY_PARALLEL: usize = 64;
pub const MIN_HOURS_PARALLEL: usize = 8;
pub const MAX_HOURS_PARALLEL: usize = 256;
pub const MIN_INTERVAL_MS: u64 = 100;
pub const MAX_INTERVAL_MS: u64 = 10_000;
pub const MAX_LOG_LINES: usize = 1000;

pub const HOURS_COMPLETE_AFTER_SECONDS: u64 = 60;

#[derive(Clone)]
pub struct Game {
    pub key: String,
    pub name: String,
    pub exe: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub games: BTreeMap<String, HistoryEntry>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct HistoryEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub exe: String,
    #[serde(default)]
    pub runs: u64,
    #[serde(default)]
    pub seconds: u64,
    //true = процесс проработал нужное время. Это НЕ подтверждение Discord.
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub first_unix: u64,
    #[serde(default)]
    pub last_unix: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: String,
    pub variety_parallel: usize,
    pub variety_interval_ms: u64,
    pub hold_seconds: u64,
    pub limit: usize,
    pub hours_parallel: usize,
    pub hours_interval_ms: u64,
    pub duration_minutes: u64,
    pub skip_played: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: "variety".into(),
            variety_parallel: 8,
            variety_interval_ms: 1000,
            hold_seconds: 15,
            limit: 0,
            hours_parallel: 8,
            hours_interval_ms: 500,
            duration_minutes: 60,
            skip_played: true,
        }
    }
}

#[derive(Default)]
pub struct Status {
    pub running: bool,
    pub mode: String,
    pub phase: String,
    pub active: usize,
    pub launched: u64,
    pub completed: u64,
    pub failed: u64,
    pub process_seconds: f64,
    pub started: Option<Instant>,
    pub planned_seconds: u64,
    pub planned_games: u64,
}

#[derive(Default)]
pub struct LogBuffer {
    pub next_id: u64,
    pub lines: VecDeque<(u64, u64, String)>,
}

pub struct App {
    pub exe: PathBuf,
    pub dir: PathBuf,
    pub status: Mutex<Status>,
    pub logs: Mutex<LogBuffer>,
    pub log_file: Mutex<File>,
    pub history: Mutex<History>,
    pub settings: Mutex<Settings>,
    pub games_count: Mutex<usize>,
    pub engine: Mutex<Option<JoinHandle<()>>>,
    pub stop: AtomicBool,
    pub quit: AtomicBool,
}

impl App {
    pub fn log(&self, message: impl AsRef<str>) {
        let message = message.as_ref();
        let timestamp = unix_now();
        let line = format!("[unix:{timestamp}] {message}");

        println!("{line}");

        {
            let mut file = lock(&self.log_file);
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }

        let mut logs = lock(&self.logs);
        logs.next_id += 1;
        let id = logs.next_id;
        logs.lines.push_back((id, timestamp, message.to_string()));

        while logs.lines.len() > MAX_LOG_LINES {
            logs.lines.pop_front();
        }
    }


    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }


    pub fn set_phase(&self, phase: impl Into<String>) {
        lock(&self.status).phase = phase.into();
    }
}

pub struct TempDirectory {
    pub path: PathBuf,
}

impl TempDirectory {
    fn create(base: &Path) -> io::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        let path = base.join(format!("qdcord-tmp-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!(
                "Не удалось удалить временный каталог {}: {error}",
                self.path.display()
            );
        }
    }
}

pub struct Worker {
    pub game: Game,
    pub child: Child,
    pub started: Instant,
    pub ended: Option<Instant>,
    pub credited: u64,
    pub counted: bool,
}

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn fmt_duration(seconds: u64) -> String {
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

pub fn create_app() -> Result<Arc<App>, Box<dyn std::error::Error>> {
    let exe = env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or("Не удалось определить каталог программы")?
        .to_path_buf();

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("qdcord.log"))?;

    let (history, history_warning) = load_history(&dir);
    let settings = load_settings(&dir);

    let app = Arc::new(App {
        exe,
        dir,
        status: Mutex::new(Status {
            phase: "Ожидание".into(),
            ..Status::default()
        }),
        logs: Mutex::new(LogBuffer::default()),
        log_file: Mutex::new(log_file),
        history: Mutex::new(history),
        settings: Mutex::new(settings),
        games_count: Mutex::new(0),
        engine: Mutex::new(None),
        stop: AtomicBool::new(false),
        quit: AtomicBool::new(false),
    });

    app.log("Программа запущена.");

    if let Some(warning) = history_warning {
        app.log(warning);
    }

    match load_games(&app.dir) {
        Ok(games) => {
            *lock(&app.games_count) = games.len();
            app.log(format!("В базе найдено {} подходящих игр.", games.len()));
        }
        Err(error) => app.log(format!("ПРЕДУПРЕЖДЕНИЕ: {error}")),
    }

    //ctrl+c в консоли: останавливаем процессы и закрываем окно
    let app_for_handler = Arc::clone(&app);
    let _ = ctrlc::set_handler(move || {
        app_for_handler.stop.store(true, Ordering::SeqCst);
        app_for_handler.quit.store(true, Ordering::SeqCst);
    });

    Ok(app)
}

pub fn shutdown_app(app: &App) {
    app.stop.store(true, Ordering::SeqCst);

    let handle = lock(&app.engine).take();
    if let Some(handle) = handle {
        app.log("Ожидаем остановку процессов...");
        let _ = handle.join();
    }

    app.log("Программа завершена.");
}

pub fn safe_exe_name(raw: &str) -> Option<String> {
    let name = raw.trim().trim_start_matches('>').trim();

    if name.is_empty()
        || name.len() > 200
        || !name.to_ascii_lowercase().ends_with(".exe")
        || name.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || name.ends_with('.')
        || name.ends_with(' ')
    {
        return None;
    }

    let stem = name.split('.').next()?.to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return None;
    }

    Some(name.to_string())
}

pub fn load_games(dir: &Path) -> Result<Vec<Game>, String> {
    let path = dir.join("detectable-games.json");

    let bytes = fs::read(&path)
        .map_err(|error| format!("не удалось прочитать {}: {error}", path.display()))?;

    if bytes.len() > 64 * 1024 * 1024 {
        return Err("файл базы превышает 64 МБ".into());
    }

    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let apps: Vec<Value> = serde_json::from_slice(bytes)
        .map_err(|error| format!("неверный формат detectable-games.json: {error}"))?;

    let mut games = Vec::new();
    let mut seen_exe = HashSet::new();
    let mut seen_key = HashSet::new();

    for app in &apps {
        let Some(executables) = app.get("executables").and_then(|v| v.as_array()) else {
            continue;
        };

        for executable in executables {
            if executable.get("os").and_then(|v| v.as_str()) != Some("win32")
                || executable.get("is_launcher").and_then(|v| v.as_bool()) == Some(true)
            {
                continue;
            }

            //записи с аргументами не имитируем
            if executable
                .get("arguments")
                .and_then(|v| v.as_str())
                .is_some_and(|args| !args.trim().is_empty())
            {
                continue;
            }

            let Some(exe) = executable
                .get("name")
                .and_then(|v| v.as_str())
                .and_then(safe_exe_name)
            else {
                continue;
            };

            let key = match app.get("id").and_then(|v| v.as_str()) {
                Some(id) if !id.trim().is_empty() => id.trim().to_string(),
                _ => format!("exe:{}", exe.to_lowercase()),
            };

            if seen_key.contains(&key) || !seen_exe.insert(exe.to_lowercase()) {
                continue;
            }

            seen_key.insert(key.clone());

            let name = app
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(exe.as_str())
                .to_string();

            games.push(Game { key, name, exe });
            break; // Один процесс на игру.
        }
    }

    if games.is_empty() {
        return Err("в detectable-games.json нет подходящих игр".into());
    }

    Ok(games)
}

pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");

    {
        let mut file = File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }

    //старый файл заменяется только после успешной записи нового.
    fs::rename(&tmp, path)
}

pub fn load_history(dir: &Path) -> (History, Option<String>) {
    let path = dir.join("history.json");

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (History { version: 1, ..History::default() }, None);
        }
        Err(error) => {
            return (
                History { version: 1, ..History::default() },
                Some(format!("ПРЕДУПРЕЖДЕНИЕ: не удалось прочитать history.json: {error}")),
            );
        }
    };

    match serde_json::from_str::<History>(text.trim_start_matches('\u{feff}')) {
        Ok(history) => (history, None),
        Err(error) => {
            let backup = dir.join(format!("history.broken-{}.json", unix_now()));
            let _ = fs::rename(&path, &backup);
            (
                History { version: 1, ..History::default() },
                Some(format!(
                    "ПРЕДУПРЕЖДЕНИЕ: history.json повреждён ({error}), сохранён как {}.",
                    backup.display()
                )),
            )
        }
    }
}

pub fn save_history(app: &App) {
    let text = {
        let mut history = lock(&app.history);
        history.version = 1;
        serde_json::to_string_pretty(&*history)
    };

    match text {
        Ok(text) => {
            if let Err(error) = write_atomic(&app.dir.join("history.json"), text.as_bytes()) {
                app.log(format!("ПРЕДУПРЕЖДЕНИЕ: не удалось сохранить history.json: {error}"));
            }
        }
        Err(error) => app.log(format!("ПРЕДУПРЕЖДЕНИЕ: ошибка истории: {error}")),
    }
}

pub fn load_settings(dir: &Path) -> Settings {
    fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save_settings(app: &App) {
    let text = serde_json::to_string_pretty(&*lock(&app.settings));

    if let Ok(text) = text {
        if let Err(error) = write_atomic(&app.dir.join("settings.json"), text.as_bytes()) {
            app.log(format!("ПРЕДУПРЕЖДЕНИЕ: не удалось сохранить settings.json: {error}"));
        }
    }
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    match settings.mode.as_str() {
        "variety" => {
            if !(1..=MAX_VARIETY_PARALLEL).contains(&settings.variety_parallel) {
                return Err(format!("Одновременно процессов: от 1 до {MAX_VARIETY_PARALLEL}"));
            }
            if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&settings.variety_interval_ms) {
                return Err(format!("Пауза: от {MIN_INTERVAL_MS} до {MAX_INTERVAL_MS} мс"));
            }
            if !(5..=3600).contains(&settings.hold_seconds) {
                return Err("Удержание: от 5 до 3600 секунд".into());
            }
        }
        "hours" => {
            if !(MIN_HOURS_PARALLEL..=MAX_HOURS_PARALLEL).contains(&settings.hours_parallel) {
                return Err(format!(
                    "Параллельных процессов: от {MIN_HOURS_PARALLEL} до {MAX_HOURS_PARALLEL}"
                ));
            }
            if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&settings.hours_interval_ms) {
                return Err(format!("Пауза: от {MIN_INTERVAL_MS} до {MAX_INTERVAL_MS} мс"));
            }
            if settings.duration_minutes > 1440 {
                return Err("Длительность: от 0 до 1440 минут".into());
            }
        }
        _ => return Err("Неизвестный режим".into()),
    }

    Ok(())
}

pub fn start_engine(app: &Arc<App>, settings: Settings) -> Result<(), String> {
    validate(&settings)?;

    if app.quit.load(Ordering::SeqCst) {
        return Err("Программа завершается".into());
    }

    let mut engine = lock(&app.engine);

    if engine.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return Err("Уже запущено".into());
    }

    if let Some(handle) = engine.take() {
        let _ = handle.join();
    }

    app.stop.store(false, Ordering::SeqCst);

    {
        let mut status = lock(&app.status);
        *status = Status {
            running: true,
            mode: settings.mode.clone(),
            phase: "Подготовка".into(),
            started: Some(Instant::now()),
            planned_seconds: if settings.mode == "hours" {
                settings.duration_minutes * 60
            } else {
                0
            },
            ..Status::default()
        };
    }

    *lock(&app.settings) = settings.clone();
    save_settings(app);

    let app_for_thread = Arc::clone(app);
    *engine = Some(thread::spawn(move || engine_main(app_for_thread, settings)));

    Ok(())
}

pub fn engine_main(app: Arc<App>, settings: Settings) {
    if let Err(error) = engine_run(&app, &settings) {
        app.log(format!("ОШИБКА: {error}"));
    }

    {
        let mut status = lock(&app.status);
        status.running = false;
        status.active = 0;
        status.phase = if app.stopped() {
            "Остановлено".into()
        } else {
            "Готово".into()
        };
    }

    app.log("Запуск завершён.");
}
pub fn engine_run(app: &App, settings: &Settings) -> Result<(), String> {
    let games = load_games(&app.dir)?;
    *lock(&app.games_count) = games.len();

    let done: HashSet<String> = lock(&app.history)
        .games
        .iter()
        .filter(|(_, entry)| entry.completed)
        .map(|(key, _)| key.clone())
        .collect();

    let (mut selected, played): (Vec<Game>, Vec<Game>) =
        games.into_iter().partition(|game| !done.contains(&game.key));

    if settings.skip_played {
        app.log(format!(
            "Пропущено игр из истории: {}. Новых доступно: {}.",
            played.len(),
            selected.len()
        ));
    } else {
        selected.extend(played);
    }

    let hours = settings.mode == "hours";

    if hours {
        selected.truncate(settings.hours_parallel);
    } else if settings.limit > 0 {
        selected.truncate(settings.limit);
    }

    if selected.is_empty() {
        return Err(
            "нет игр для запуска: все уже в истории. Снимите галочку или очистите историю.".into(),
        );
    }

    lock(&app.status).planned_games = selected.len() as u64;

    let temp = TempDirectory::create(&app.dir)
        .map_err(|error| format!("не удалось создать временный каталог: {error}"))?;

    if hours {
        if selected.len() < settings.hours_parallel {
            app.log(format!(
                "ПРЕДУПРЕЖДЕНИЕ: доступно только {} игр из {} запрошенных.",
                selected.len(),
                settings.hours_parallel
            ));
        }

        let hold = (settings.duration_minutes > 0)
            .then(|| Duration::from_secs(settings.duration_minutes * 60));

        app.log(format!(
            "Накрутка часов: {} процессов, длительность {}.",
            selected.len(),
            hold.map_or("до «Стоп»".to_string(), |d| fmt_duration(d.as_secs()))
        ));

        run_batch(
            app,
            &temp.path,
            &selected,
            Duration::from_millis(settings.hours_interval_ms),
            hold,
            HOURS_COMPLETE_AFTER_SECONDS,
            "Часы",
        );
    } else {
        let total_batches = selected.len().div_ceil(settings.variety_parallel);

        app.log(format!(
            "Разные игры: {} игр, {} пачек по {}.",
            selected.len(),
            total_batches,
            settings.variety_parallel
        ));

        for (index, batch) in selected.chunks(settings.variety_parallel).enumerate() {
            if app.stopped() {
                break;
            }

            let label = format!("Пачка {}/{total_batches}", index + 1);
            app.log(format!("{label}: запуск {} процессов.", batch.len()));

            run_batch(
                app,
                &temp.path,
                batch,
                Duration::from_millis(settings.variety_interval_ms),
                Some(Duration::from_secs(settings.hold_seconds)),
                settings.hold_seconds,
                &label,
            );
        }
    }

    Ok(())
}

pub fn run_batch(
    app: &App,
    temp_dir: &Path,
    games: &[Game],
    interval: Duration,
    hold: Option<Duration>,
    complete_after: u64,
    label: &str,
) {
    let mut workers: Vec<Worker> = Vec::new();
    let mut last_tick = Instant::now();

    for (index, game) in games.iter().enumerate() {
        if app.stopped() {
            break;
        }

        app.set_phase(format!("{label}: запуск {}/{}", index + 1, games.len()));

        match start_worker(&app.exe, temp_dir, &game.exe) {
            Ok(child) => {
                app.log(format!("Запущен {} ({}), PID {}.", game.name, game.exe, child.id()));
                register_launch(app, game);

                workers.push(Worker {
                    game: game.clone(),
                    child,
                    started: Instant::now(),
                    ended: None,
                    credited: 0,
                    counted: false,
                });

                let mut status = lock(&app.status);
                status.launched += 1;
                status.active = workers.iter().filter(|w| w.ended.is_none()).count();
            }
            Err(error) => {
                app.log(format!("ПРЕДУПРЕЖДЕНИЕ: не удалось запустить {}: {error}", game.exe));
                lock(&app.status).failed += 1;
            }
        }

        if index + 1 < games.len() {
            wait_and_poll(app, &mut workers, interval, &mut last_tick);
        }
    }

    if !workers.is_empty() {
        let hold_started = Instant::now();
        let mut last_checkpoint = Instant::now();

        loop {
            if app.stopped() {
                break;
            }

            let elapsed = hold_started.elapsed();

            if let Some(hold) = hold {
                if elapsed >= hold {
                    break;
                }
                app.set_phase(format!(
                    "{label}: работает, осталось {}",
                    fmt_duration(hold.saturating_sub(elapsed).as_secs())
                ));
            } else {
                app.set_phase(format!(
                    "{label}: работает {} (до «Стоп»)",
                    fmt_duration(elapsed.as_secs())
                ));
            }

            if workers.iter().all(|w| w.ended.is_some()) {
                app.log(format!("{label}: все процессы завершились сами."));
                break;
            }

            wait_and_poll(app, &mut workers, Duration::from_secs(1), &mut last_tick);

            if last_checkpoint.elapsed() >= Duration::from_secs(60) {
                checkpoint(app, &mut workers, complete_after);
                save_history(app);
                last_checkpoint = Instant::now();
            }
        }
    } else if !app.stopped() {
        app.log(format!("ПРЕДУПРЕЖДЕНИЕ: {label}: нет работающих процессов."));
    }

    poll_workers(app, &mut workers, &mut last_tick);
    let now = Instant::now();
    for worker in &mut workers {
        if worker.ended.is_none() {
            worker.ended = Some(now);
        }
    }

    checkpoint(app, &mut workers, complete_after);

    app.set_phase(format!("{label}: остановка процессов"));
    stop_workers(app, &mut workers);
    lock(&app.status).active = 0;

    save_history(app);

    // Удаляем только свои ссылки в своём временном каталоге
    for game in games {
        let path = temp_dir.join(&game.exe);
        if path.exists() {
            if let Err(error) = fs::remove_file(&path) {
                app.log(format!(
                    "ПРЕДУПРЕЖДЕНИЕ: не удалось удалить {}: {error}",
                    path.display()
                ));
            }
        }
    }

    app.log(format!("{label}: завершено."));
}


pub fn register_launch(app: &App, game: &Game) {
    let now = unix_now();
    let mut history = lock(&app.history);

    let entry = history
        .games
        .entry(game.key.clone())
        .or_insert_with(|| HistoryEntry {
            name: game.name.clone(),
            exe: game.exe.clone(),
            first_unix: now,
            ..HistoryEntry::default()
        });

    entry.runs += 1;
    entry.last_unix = now;
}

pub fn checkpoint(app: &App, workers: &mut [Worker], complete_after: u64) {
    let now = Instant::now();
    let timestamp = unix_now();
    let mut newly_completed = 0u64;

    {
        let mut history = lock(&app.history);

        for worker in workers.iter_mut() {
            let ran = worker
                .ended
                .unwrap_or(now)
                .saturating_duration_since(worker.started)
                .as_secs();

            let delta = ran.saturating_sub(worker.credited);
            worker.credited = ran;

            let entry = history
                .games
                .entry(worker.game.key.clone())
                .or_insert_with(|| HistoryEntry {
                    name: worker.game.name.clone(),
                    exe: worker.game.exe.clone(),
                    first_unix: timestamp,
                    ..HistoryEntry::default()
                });

            entry.seconds += delta;
            entry.last_unix = timestamp;

            if !worker.counted && ran >= complete_after {
                worker.counted = true;
                entry.completed = true;
                newly_completed += 1;
            }
        }
    }

    lock(&app.status).completed += newly_completed;
}

pub fn wait_and_poll(app: &App, workers: &mut [Worker], duration: Duration, last_tick: &mut Instant) {
    let started = Instant::now();

    while !app.stopped() && started.elapsed() < duration {
        let step = duration
            .saturating_sub(started.elapsed())
            .min(Duration::from_millis(250));

        thread::sleep(step);
        poll_workers(app, workers, last_tick);
    }
}

pub fn poll_workers(app: &App, workers: &mut [Worker], last_tick: &mut Instant) {
    let now = Instant::now();
    let delta = now.saturating_duration_since(*last_tick).as_secs_f64();
    *last_tick = now;

    let alive_before = workers.iter().filter(|w| w.ended.is_none()).count();
    let mut exited = 0u64;

    for worker in workers.iter_mut() {
        if worker.ended.is_some() {
            continue;
        }

        match worker.child.try_wait() {
            Ok(Some(status)) => {
                worker.ended = Some(now);
                exited += 1;
                app.log(format!(
                    "ПРЕДУПРЕЖДЕНИЕ: {} завершился раньше: {status}",
                    worker.game.exe
                ));
            }
            Ok(None) => {}
            Err(error) => {
                worker.ended = Some(now);
                exited += 1;
                app.log(format!(
                    "ПРЕДУПРЕЖДЕНИЕ: ошибка проверки {}: {error}",
                    worker.game.exe
                ));
            }
        }
    }

    let mut status = lock(&app.status);
    status.process_seconds += alive_before as f64 * delta;
    status.failed += exited;
    status.active = workers.iter().filter(|w| w.ended.is_none()).count();
}

pub fn start_worker(current_exe: &Path, temp_dir: &Path, name: &str) -> io::Result<Child> {
    let stub_path = temp_dir.join(name);

    if !stub_path.exists() && fs::hard_link(current_exe, &stub_path).is_err() {
        fs::copy(current_exe, &stub_path)?;
    }

    let mut command = Command::new(&stub_path);

    command
        .arg(WORKER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command.spawn()
}

pub fn stop_workers(app: &App, workers: &mut [Worker]) {
    if workers.is_empty() {
        return;
    }

    // закрытие stdin сообщает всем заглушкам, что пора выйти.
    for worker in workers.iter_mut() {
        drop(worker.child.stdin.take());
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut finished = vec![false; workers.len()];

    while Instant::now() < deadline && finished.iter().any(|done| !done) {
        for (index, worker) in workers.iter_mut().enumerate() {
            if !finished[index] && !matches!(worker.child.try_wait(), Ok(None)) {
                finished[index] = true;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    let mut forced = 0usize;

    for (index, worker) in workers.iter_mut().enumerate() {
        if finished[index] {
            continue;
        }

        forced += 1;
        if let Err(error) = worker.child.kill() {
            app.log(format!("Не удалось остановить {}: {error}", worker.game.exe));
        }
        let _ = worker.child.wait();
    }

    app.log(format!(
        "Остановлено процессов: {}{}.",
        workers.len(),
        if forced > 0 {
            format!(", из них принудительно: {forced}")
        } else {
            String::new()
        }
    ));
}

pub fn worker_main() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    set_linux_process_name()?;

    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use std::ptr;

        type Handle = *mut c_void;

        #[repr(C)]
        struct WindowClass {
            style: u32,
            window_proc: unsafe extern "system" fn(
                Handle,
                u32,
                usize,
                isize,
            ) -> isize,
            class_extra: i32,
            window_extra: i32,
            instance: Handle,
            icon: Handle,
            cursor: Handle,
            background: Handle,
            menu_name: *const u16,
            class_name: *const u16,
        }

        #[repr(C)]
        struct Message {
            window: Handle,
            message: u32,
            wparam: usize,
            lparam: isize,
            time: u32,
            point_x: i32,
            point_y: i32,
            private: u32,
        }

        #[link(name = "user32")]
        extern "system" {
            fn RegisterClassW(class: *const WindowClass) -> u16;

            fn CreateWindowExW(
                extended_style: u32,
                class_name: *const u16,
                window_name: *const u16,
                style: u32,
                x: i32,
                y: i32,
                width: i32,
                height: i32,
                parent: Handle,
                menu: Handle,
                instance: Handle,
                parameter: Handle,
            ) -> Handle;

            fn ShowWindow(window: Handle, command: i32) -> i32;

            fn DefWindowProcW(
                window: Handle,
                message: u32,
                wparam: usize,
                lparam: isize,
            ) -> isize;

            fn GetMessageW(
                message: *mut Message,
                window: Handle,
                min_filter: u32,
                max_filter: u32,
            ) -> i32;

            fn TranslateMessage(message: *const Message) -> i32;
            fn DispatchMessageW(message: *const Message) -> isize;

            fn PostMessageW(
                window: Handle,
                message: u32,
                wparam: usize,
                lparam: isize,
            ) -> i32;

            fn PostQuitMessage(exit_code: i32);
        }

        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(module_name: *const u16) -> Handle;
        }

        const WM_CLOSE: u32 = 0x0010;
        const WM_DESTROY: u32 = 0x0002;

        const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
        const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
        const WS_POPUP: u32 = 0x8000_0000;
        const SW_SHOWNA: i32 = 8;

        unsafe extern "system" fn window_proc(
            window: Handle,
            message: u32,
            wparam: usize,
            lparam: isize,
        ) -> isize {
            if message == WM_DESTROY {
                unsafe {
                    PostQuitMessage(0);
                }
                return 0;
            }

            unsafe {
                DefWindowProcW(window, message, wparam, lparam)
            }
        }

        let title = env::current_exe()?
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Game")
            .to_owned();

        let class_name: Vec<u16> =
            "QDCordStub\0".encode_utf16().collect();

        let window_title: Vec<u16> =
            format!("{title}\0").encode_utf16().collect();

        let instance = unsafe {
            GetModuleHandleW(ptr::null())
        };

        if instance.is_null() {
            return Err(io::Error::last_os_error());
        }

        let class = WindowClass {
            style: 0,
            window_proc,
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: ptr::null_mut(),
            cursor: ptr::null_mut(),
            background: ptr::null_mut(),
            menu_name: ptr::null(),
            class_name: class_name.as_ptr(),
        };

        let registered = unsafe {
            RegisterClassW(&class)
        };

        if registered == 0 {
            return Err(io::Error::last_os_error());
        }

        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_POPUP,
                -10000,
                -10000,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            )
        };

        if window.is_null() {
            return Err(io::Error::last_os_error());
        }

        unsafe {
            ShowWindow(window, SW_SHOWNA);
        }

        let window_id = window as usize;

        thread::spawn(move || {
            let mut stdin = io::stdin();
            let mut buffer = [0u8; 64];

            loop {
                match stdin.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error)
                        if error.kind() == io::ErrorKind::Interrupted =>
                    {
                        continue;
                    }
                    Err(_) => break,
                }
            }

            unsafe {
                PostMessageW(window_id as Handle, WM_CLOSE, 0, 0);
            }
        });

        let mut message: Message = unsafe {
            std::mem::zeroed()
        };

        loop {
            let result = unsafe {
                GetMessageW(&mut message, ptr::null_mut(), 0, 0)
            };

            if result == -1 {
                return Err(io::Error::last_os_error());
            }

            if result == 0 {
                break;
            }

            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let mut stdin = io::stdin();
        let mut buffer = [0u8; 64];

        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if error.kind() == io::ErrorKind::Interrupted =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub fn set_linux_process_name() -> io::Result<()> {
    use std::ffi::CString;

    let exe = env::current_exe()?;

    let name = exe
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::other("Не удалось определить имя процесса"))?;

    let mut end = name.len().min(15);

    while !name.is_char_boundary(end) {
        end -= 1;
    }

    let name = CString::new(&name[..end])
        .map_err(|_| io::Error::other("Имя содержит нулевой байт"))?;

    let result = unsafe {
        libc::prctl(libc::PR_SET_NAME, name.as_ptr(), 0, 0, 0)
    };

    if result == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}
