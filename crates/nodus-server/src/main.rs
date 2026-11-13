//! `nodus-server`: Nodus on a server, and its session host. Installed on a
//! server as `/opt/nodus/bin/nodus` (SRV-001); what each command does is in
//! the library.

fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    std::process::exit(nodus_server::main(&args));
}
