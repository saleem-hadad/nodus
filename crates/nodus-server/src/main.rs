//! `nodus-server`: Nodus on a server, and its session host. Installed on a
//! server as `/opt/nodus/bin/nodus` (SRV-001); what each command does is in
//! the library.

fn main() {
    let mut args = std::env::args_os().map(|arg| arg.to_string_lossy().into_owned());
    let program = args.next().unwrap_or_default();
    let name = std::path::Path::new(&program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let args: Vec<String> = args.collect();
    std::process::exit(nodus_server::run(&name, &args));
}
