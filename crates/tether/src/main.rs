//! tether CLI: the single entry point that the Neovim plugin, the diagram viewer and Claude call.

mod browse;
mod cli;
mod view;

fn main() {
    std::process::exit(cli::run());
}
