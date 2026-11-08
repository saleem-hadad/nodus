//! The Nodus binary: the app, and the session host it runs as `Nodus host …`,
//! as the Electron app runs its own binary as Node for the host. Only this and
//! `nodus-ui` know the UI exists.
//!
//! Ports `src/main/index.ts` and `src/main/menu.ts`.

#[expect(clippy::print_stdout, reason = "the binary only says which version it is so far")]
fn main() {
    println!("Nodus {}", env!("CARGO_PKG_VERSION"));
}
