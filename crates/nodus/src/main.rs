//! The Nodus binary: the app, and the session host it runs as `Nodus host …`,
//! as the Electron app runs its own binary as Node for the host. Only this and
//! `nodus-ui` know the UI exists.
//!
//! Ports `src/main/index.ts` and `src/main/menu.ts`.

fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    // `Nodus host <socket> <key>`: the session host, before anything of the
    // app's starts, so it has no window, menu or Dock icon (spec 02, HOST-035).
    if args.first().map(String::as_str) == Some("host") {
        std::process::exit(nodus_host::main(&args[1..]));
    }
    std::process::exit(app());
}

/// The app: its data folder (`NODUS_USER_DATA`, else the OS's app data
/// folder, PLAT-003/PLAT-004), then the window. Anything else on the command
/// line is ignored, as Electron ignored what macOS passes it.
fn app() -> i32 {
    let Some(user_data) = nodus_store::user_data_dir() else {
        return 1;
    };
    match nodus_ui::run(user_data) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
