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
    app();
}

#[expect(clippy::print_stdout, reason = "the binary only says which version it is so far")]
fn app() {
    println!("Nodus {}", env!("CARGO_PKG_VERSION"));
}
