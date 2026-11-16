use clap::{Args, Subcommand};
use tether_core::config::{self, Config};
use tether_core::error::{Result, TetherError};

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Print the effective configuration (defaults + file)
    Show(ShowArgs),
    /// Print the config file path (whether or not it exists)
    Path,
    /// Write a commented default config file (refuses to overwrite)
    Init,
    /// Print one value by dotted key, e.g. `nvim.command` or `keys.comment`
    Get { key: String },
}

#[derive(Args)]
pub struct ShowArgs {
    #[arg(long)]
    pub json: bool,
}

impl ConfigCmd {
    pub fn wants_json(&self) -> bool {
        matches!(self, ConfigCmd::Show(a) if a.json)
    }
}

pub fn run(c: ConfigCmd) -> Result<()> {
    match c {
        ConfigCmd::Path => {
            println!("{}", config::config_path().display());
            Ok(())
        }
        ConfigCmd::Show(a) => {
            let cfg = Config::load()?;
            if a.json {
                println!("{}", serde_json::to_string(&cfg).map_err(|e| TetherError::Other(e.to_string()))?);
            } else {
                println!("# {}", config::config_path().display());
                print!("{}", toml::to_string_pretty(&cfg).map_err(|e| TetherError::Other(e.to_string()))?);
            }
            Ok(())
        }
        ConfigCmd::Init => {
            let path = config::config_path();
            config::init_file(&path)?;
            println!("wrote {}", path.display());
            Ok(())
        }
        ConfigCmd::Get { key } => {
            let cfg = Config::load()?;
            match cfg.get(&key) {
                Some(serde_json::Value::String(s)) => println!("{s}"),
                Some(v) => println!("{v}"),
                None => return Err(TetherError::Input(format!("unknown config key `{key}`"))),
            }
            Ok(())
        }
    }
}
