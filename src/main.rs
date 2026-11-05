// В release-сборке консольное окно не открывается
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod engine;
#[cfg(windows)]
mod gui;

use engine::{create_app, shutdown_app, worker_main, WORKER_ARG};
use std::env;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(not(windows))]
use engine::{lock, start_engine};
#[cfg(not(windows))]
use std::sync::atomic::Ordering;
#[cfg(not(windows))]
use std::{thread, time::Duration};

fn main() {
    if let Err(error) = run() {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}


fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os();
    let _program_name = args.next();

    if args.next().as_deref() == Some(std::ffi::OsStr::new(WORKER_ARG)) {
        worker_main()?;
        return Ok(());
    }

    #[cfg(any(windows, target_os = "linux"))]
    {
        ui_main()
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("Поддерживаются только Windows и Linux".into())
    }
}

#[cfg(windows)]
fn ui_main() -> Result<(), Box<dyn std::error::Error>> {
    let app = create_app()?;

    if let Err(error) = gui::run_gui(Arc::clone(&app)) {
        app.log(format!("ОШИБКА интерфейса: {error}"));
        gui::error_box(&error);
    }

    shutdown_app(&app);
    Ok(())
}

#[cfg(not(windows))]
fn ui_main() -> Result<(), Box<dyn std::error::Error>> {
    let app = create_app()?;
    let settings = lock(&app.settings).clone();

    start_engine(&app, settings)?;

    while lock(&app.status).running && !app.quit.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));
    }

    shutdown_app(&app);
    Ok(())
}
