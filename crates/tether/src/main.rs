//! tether CLI: the single entry point that the Neovim plugin, the diagram viewer and Claude call.

mod cli;

fn main() {
    std::process::exit(cli::run());
}
